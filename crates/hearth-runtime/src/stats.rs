//! Usage and cost summaries over the read-only database.

use std::collections::BTreeSet;
use std::path::Path;

use hearth_config::ConfigError;
use hearth_storage::StorageError;
use serde::Serialize;
use thiserror::Error;

/// One aggregate usage row.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StatsRow {
    pub day: String,
    pub model: String,
    pub kind: String,
    pub calls: i64,
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cost_micros: i64,
    pub unpriced: bool,
}

/// The usage summary.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StatsReport {
    pub rows: Vec<StatsRow>,
    pub total_cost_micros: i64,
    pub unpriced_models: Vec<String>,
}

/// A stats failure.
#[derive(Debug, Error)]
pub enum StatsError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Read the usage summary without writing.
pub fn stats_report(home: &Path) -> Result<StatsReport, StatsError> {
    let settings = hearth_config::load(home)?;
    hearth_config::Settings::validate(&settings)?;
    let connection = hearth_storage::open_read_only(&home.join("hearth.db"))?;
    let mut statement = connection
        .prepare(
            "SELECT date(ts / 1000000, 'unixepoch') AS day, model, kind, COUNT(*), \
             COALESCE(SUM(input_tokens), 0), COALESCE(SUM(cache_read_tokens), 0), \
             COALESCE(SUM(cache_write_tokens), 0), COALESCE(SUM(output_tokens), 0), \
             COALESCE(SUM(reasoning_tokens), 0), COALESCE(SUM(cost_micros), 0) \
             FROM usage_events GROUP BY day, model, kind ORDER BY day, model, kind",
        )
        .map_err(StorageError::from)?;
    let priced: BTreeSet<String> = settings
        .models
        .as_ref()
        .map(|models| models.pricing.keys().cloned().collect())
        .unwrap_or_default();
    let rows_iter = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, i64>(9)?,
            ))
        })
        .map_err(StorageError::from)?;
    let mut rows = Vec::new();
    for row in rows_iter {
        let (day, model, kind, calls, input, cache_read, cache_write, output, reasoning, cost) =
            row.map_err(StorageError::from)?;
        rows.push(StatsRow {
            day,
            model: model.clone(),
            kind,
            calls,
            input_tokens: input,
            cache_read_tokens: cache_read,
            cache_write_tokens: cache_write,
            output_tokens: output,
            reasoning_tokens: reasoning,
            cost_micros: cost,
            unpriced: !priced.contains(&model),
        });
    }
    let total_cost_micros = rows.iter().map(|row| row.cost_micros).sum();
    let unpriced_models = rows
        .iter()
        .filter(|row| row.unpriced)
        .map(|row| row.model.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(StatsReport {
        rows,
        total_cost_micros,
        unpriced_models,
    })
}

/// Print the usage summary.
pub fn stats(home: &Path, json: bool) -> Result<(), StatsError> {
    let report = stats_report(home)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_owned())
        );
        return Ok(());
    }
    for row in &report.rows {
        let unpriced = if row.unpriced { " (unpriced)" } else { "" };
        println!(
            "{} {} {}: {} calls, input {}, cache {}, output {}, cost ${:.6}{}",
            row.day,
            row.model,
            row.kind,
            row.calls,
            row.input_tokens,
            row.cache_read_tokens,
            row.output_tokens,
            row.cost_micros as f64 / 1_000_000.0,
            unpriced
        );
    }
    println!(
        "total cost: ${:.6}",
        report.total_cost_micros as f64 / 1_000_000.0
    );
    if !report.unpriced_models.is_empty() {
        println!("unpriced models: {}", report.unpriced_models.join(", "));
    }
    Ok(())
}
