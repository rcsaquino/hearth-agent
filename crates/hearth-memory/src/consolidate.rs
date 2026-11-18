//! The deterministic audit and repair pass.
//!
//! One transaction does three things: closes canonical duplicates, measures the
//! groups that need a decision, and writes the report beside its timestamp. A
//! crash rolls the whole pass back, so a row is never half-closed.
//!
//! Canonical means the same fingerprint under the same subject. Those rows make
//! the same claim with the same words, so keeping one and closing the rest loses
//! nothing. A near duplicate, a statement repeated under two subjects, and a row
//! that carries more than one statement all need a decision, so they are only
//! reported.

use std::collections::{BTreeMap, BTreeSet};

use hearth_core::Session;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::errors::{MemoryError, Result};
use crate::model::Memory;
use crate::store::{self, COLUMNS, map_memory};

/// The most current rows the near and compound reports read in one pass.
const SCAN: i64 = 10_000;
/// The most examples one report keeps for a group.
const SAMPLE: usize = 20;
/// The most rows one subject contributes to the near-pair comparison.
const NEAR_GROUP: usize = 128;

/// One canonical group that was repaired.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalGroup {
    /// The row that stayed current.
    pub kept: String,
    /// The rows whose window was closed.
    pub closed: Vec<String>,
    pub content: String,
}

/// Two current rows that say the same thing without being canonical duplicates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NearPair {
    pub left: String,
    pub right: String,
    pub overlap: f32,
}

/// One statement repeated under more than one non-empty subject.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubjectConflict {
    pub ids: Vec<String>,
    pub subjects: Vec<String>,
    pub content: String,
}

/// A current row that carries more than one statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompoundRow {
    pub id: String,
    pub statements: usize,
    pub content: String,
}

/// What one consolidation pass found and repaired.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConsolidationReport {
    /// The instant the pass ran, in UTC microseconds.
    pub at: i64,
    /// How many current rows the near and compound reports read.
    pub scanned: usize,
    /// True when the store held more current rows than `SCAN`.
    pub truncated: bool,
    /// Current rows whose window this pass closed.
    pub canonical_closed: usize,
    /// Near pairs this pass closed.
    pub near_closed: usize,
    /// Near pairs left current, because closing one would cost a statement.
    pub near_duplicates: usize,
    /// Statements repeated under different non-empty subjects.
    pub cross_subject: usize,
    /// Current rows that carry more than one statement.
    pub compound: usize,
    /// The canonical groups this pass repaired, up to the sample bound.
    pub canonical: Vec<CanonicalGroup>,
    /// Near pairs, up to the sample bound.
    pub near: Vec<NearPair>,
    /// Cross-subject conflicts, up to the sample bound.
    pub subject_conflicts: Vec<SubjectConflict>,
    /// Compound rows, up to the sample bound.
    pub compound_rows: Vec<CompoundRow>,
}

impl ConsolidationReport {
    /// The counts as an activity or status payload.
    pub fn summary_json(&self) -> Value {
        json!({
            "at": self.at,
            "scanned": self.scanned,
            "truncated": self.truncated,
            "canonical_closed": self.canonical_closed,
            "near_closed": self.near_closed,
            "near_duplicates": self.near_duplicates,
            "cross_subject": self.cross_subject,
            "compound": self.compound,
        })
    }

    /// Whether the pass found nothing to report.
    pub fn is_empty(&self) -> bool {
        self.canonical_closed == 0
            && self.near_closed == 0
            && self.near_duplicates == 0
            && self.cross_subject == 0
            && self.compound == 0
    }
}

/// Run the pass against one store.
pub fn run(session: &Session) -> Result<ConsolidationReport> {
    let now = session.now_micros();
    session
        .storage()
        .write(move |writer| consolidate_in(writer, now))
}

fn consolidate_in(writer: &hearth_storage::Writer<'_>, now: i64) -> Result<ConsolidationReport> {
    let mut report = ConsolidationReport {
        at: now,
        ..ConsolidationReport::default()
    };
    close_canonical(writer, now, &mut report)?;
    report.cross_subject = report_subject_conflicts(writer, now, &mut report.subject_conflicts)?;
    read_current(writer, now, &mut report)?;
    write_stamp(writer, now, &report)?;
    Ok(report)
}

fn close_canonical(
    writer: &hearth_storage::Writer<'_>,
    now: i64,
    report: &mut ConsolidationReport,
) -> Result<()> {
    let groups: Vec<(String, String)> = writer.query(
        "SELECT fingerprint, subject FROM memories \
         WHERE valid_from <= ? AND valid_to IS NULL \
         GROUP BY fingerprint, subject HAVING COUNT(*) > 1 \
         ORDER BY fingerprint, subject",
        [now],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    for (fingerprint, subject) in groups {
        let rows: Vec<Memory> = writer.query(
            &format!(
                "SELECT {COLUMNS} FROM memories WHERE fingerprint = ? AND subject = ? \
                 AND valid_from <= ? AND valid_to IS NULL"
            ),
            rusqlite::params![fingerprint, subject, now],
            map_memory,
        )?;
        let Some(survivor) = survivor(&rows) else {
            continue;
        };
        let mut closed = Vec::new();
        for row in &rows {
            if row.id == survivor.id {
                continue;
            }
            writer.execute(
                "UPDATE memories SET valid_to = ?, superseded_by = ?, pinned = 0 \
                 WHERE id = ? AND valid_to IS NULL",
                rusqlite::params![now, survivor.id, row.id],
            )?;
            report.canonical_closed += 1;
            closed.push(row.id.clone());
        }
        if !closed.is_empty() && report.canonical.len() < SAMPLE {
            report.canonical.push(CanonicalGroup {
                kept: survivor.id.clone(),
                closed,
                content: survivor.content.clone(),
            });
        }
        // Abort here in a crash test: the transaction rolls back as one unit, so
        // the seeded duplicate stays current and open.
        hearth_storage::crash_if("memory.consolidate");
    }
    Ok(())
}

/// The row a canonical group keeps: the strongest current statement.
fn survivor(rows: &[Memory]) -> Option<&Memory> {
    rows.iter().max_by(|left, right| {
        left.pinned
            .cmp(&right.pinned)
            .then(left.kind.prior().total_cmp(&right.kind.prior()))
            .then(left.updated_at.cmp(&right.updated_at))
            .then(right.id.cmp(&left.id))
    })
}

fn report_subject_conflicts(
    writer: &hearth_storage::Writer<'_>,
    now: i64,
    samples: &mut Vec<SubjectConflict>,
) -> Result<usize> {
    let fingerprints: Vec<String> = writer.query(
        "SELECT fingerprint FROM memories \
         WHERE valid_from <= ? AND valid_to IS NULL AND subject <> '' \
         GROUP BY fingerprint HAVING COUNT(DISTINCT subject) > 1 \
         ORDER BY fingerprint",
        [now],
        |row| row.get(0),
    )?;
    let total = fingerprints.len();
    for fingerprint in fingerprints.iter().take(SAMPLE) {
        let rows: Vec<(String, String, String)> = writer.query(
            "SELECT id, subject, content FROM memories \
             WHERE fingerprint = ? AND valid_from <= ? AND valid_to IS NULL AND subject <> '' \
             ORDER BY subject, id",
            rusqlite::params![fingerprint, now],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let subjects: Vec<String> = rows
            .iter()
            .map(|(_, subject, _)| subject.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        samples.push(SubjectConflict {
            ids: rows.iter().map(|(id, _, _)| id.clone()).collect(),
            subjects,
            content: rows
                .first()
                .map(|(_, _, content)| content.clone())
                .unwrap_or_default(),
        });
    }
    Ok(total)
}

fn read_current(
    writer: &hearth_storage::Writer<'_>,
    now: i64,
    report: &mut ConsolidationReport,
) -> Result<()> {
    let total: i64 = writer.query_row(
        "SELECT COUNT(*) FROM memories WHERE valid_from <= ? AND valid_to IS NULL",
        [now],
        |row| row.get(0),
    )?;
    let rows: Vec<Memory> = writer.query(
        &format!(
            "SELECT {COLUMNS} FROM memories WHERE valid_from <= ? AND valid_to IS NULL \
             ORDER BY updated_at DESC, id LIMIT {SCAN}"
        ),
        [now],
        map_memory,
    )?;
    report.scanned = rows.len();
    report.truncated = total > rows.len() as i64;

    for row in &rows {
        let statements = crate::text::statement_count(&row.content);
        if statements > 1 {
            report.compound += 1;
            if report.compound_rows.len() < SAMPLE {
                report.compound_rows.push(CompoundRow {
                    id: row.id.clone(),
                    statements,
                    content: row.content.clone(),
                });
            }
        }
    }

    repair_near(writer, now, &rows, report)?;
    Ok(())
}

/// Close the pairs that are one statement, and record the pairs that are not.
///
/// A pair is provable when the write gate would have refused the second write:
/// the same statement under the same subject, or one statement that contains the
/// other. Only those are closed. A pair that merely overlaps could be a changed
/// value, and closing the wrong one would retire a fact, so it is reported.
fn repair_near(
    writer: &hearth_storage::Writer<'_>,
    now: i64,
    rows: &[Memory],
    report: &mut ConsolidationReport,
) -> Result<()> {
    let mut groups: BTreeMap<String, Vec<&Memory>> = BTreeMap::new();
    for row in rows {
        groups.entry(row.subject.clone()).or_default().push(row);
    }
    let mut closed_here: BTreeSet<String> = BTreeSet::new();
    for group in groups.values() {
        for index in 0..group.len().min(NEAR_GROUP) {
            for other in group.iter().take(NEAR_GROUP).skip(index + 1) {
                let left = group[index];
                if left.id == other.id {
                    continue;
                }
                if closed_here.contains(&left.id) || closed_here.contains(&other.id) {
                    continue;
                }
                if !store::one_statement(&left.content, &other.content) {
                    continue;
                }
                let (kept, redundant) = redundant_side(left, other);
                if redundant.pinned {
                    report.near_duplicates += 1;
                    if report.near.len() < SAMPLE {
                        report.near.push(NearPair {
                            left: left.id.clone(),
                            right: other.id.clone(),
                            overlap: store::similarity(&left.content, &other.content),
                        });
                    }
                    continue;
                }
                writer.execute(
                    "UPDATE memories SET valid_to = ?, superseded_by = ?, pinned = 0 \
                     WHERE id = ? AND valid_to IS NULL",
                    rusqlite::params![now, kept.id, redundant.id],
                )?;
                closed_here.insert(redundant.id.clone());
                report.near_closed += 1;
            }
        }
    }
    Ok(())
}

/// The pair's two sides as the row that stays and the row that stops.
///
/// The fuller wording wins, because it says everything the other said. Between
/// two equally full statements the survivor rule decides, the same rule the
/// canonical repair uses.
fn redundant_side<'a>(left: &'a Memory, right: &'a Memory) -> (&'a Memory, &'a Memory) {
    if store::contains_statement(&left.content, &right.content) {
        return (right, left);
    }
    if store::contains_statement(&right.content, &left.content) {
        return (left, right);
    }
    match survivor(&[left.clone(), right.clone()]) {
        Some(kept) if kept.id == left.id => (left, right),
        _ => (right, left),
    }
}

fn write_stamp(
    writer: &hearth_storage::Writer<'_>,
    now: i64,
    report: &ConsolidationReport,
) -> Result<()> {
    let encoded = serde_json::to_string(report).map_err(|error| {
        MemoryError::unavailable(format!("report could not be encoded: {error}"))
    })?;
    let stamp = OffsetDateTime::from_unix_timestamp_nanos(now as i128 * 1_000)
        .ok()
        .and_then(|instant| instant.format(&Rfc3339).ok())
        .unwrap_or_else(|| now.to_string());
    for (key, value) in [
        ("last_consolidation", stamp),
        ("last_consolidation_report", encoded),
    ] {
        writer.execute(
            "INSERT INTO meta (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
    }
    Ok(())
}
