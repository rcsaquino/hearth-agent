//! Overview cards and recent activity.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{App, DaemonState};
use crate::theme;
use crate::ui::{activity, format_uptime, short};

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let banner_height = if app.banner.is_some() { 1 } else { 0 };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(banner_height),
            Constraint::Length(10),
            Constraint::Min(5),
        ])
        .split(area);
    if let Some(banner) = &app.banner {
        frame.render_widget(
            Paragraph::new(Line::styled(format!(" {banner}"), theme::bad())),
            rows[0],
        );
    }
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(36),
            Constraint::Percentage(34),
            Constraint::Percentage(30),
        ])
        .split(rows[1]);
    runtime_card(app, frame, columns[0]);
    usage_card(app, frame, columns[1]);
    work_card(app, frame, columns[2]);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(rows[2]);
    connections_card(app, frame, bottom[0]);
    recent_card(app, frame, bottom[1]);
}

fn runtime_card(app: &App, frame: &mut Frame, area: Rect) {
    let (label, style) = match &app.daemon {
        DaemonState::Running => ("Running", theme::good()),
        DaemonState::Starting => ("Starting", theme::warn()),
        DaemonState::Stopping => ("Stopping", theme::warn()),
        DaemonState::Stopped => ("Stopped", theme::muted()),
        DaemonState::Error(_) => ("Error", theme::bad()),
    };
    let uptime = app
        .status
        .as_ref()
        .and_then(|status| status["uptime_seconds"].as_u64())
        .map(format_uptime)
        .unwrap_or_else(|| "-".to_owned());
    let models = app
        .settings_data
        .as_ref()
        .and_then(|settings| settings.models.as_ref());
    let provider = models
        .map(|models| short(&models.base_url, 28))
        .unwrap_or_else(|| "not configured".to_owned());
    let context = models
        .map(|models| {
            format!(
                "{} · {} reserve",
                compact_count(models.context_window),
                compact_count(models.max_output_tokens)
            )
        })
        .unwrap_or_else(|| "not configured".to_owned());
    let workspace = app
        .settings_data
        .as_ref()
        .and_then(|settings| settings.agent.workspace.as_ref())
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| app.home.join("workspace").display().to_string());
    card(
        frame,
        area,
        "Runtime",
        vec![
            Line::from(vec![
                Span::styled("● ", style),
                Span::styled(label, style),
                Span::styled(format!("  up {uptime}"), theme::muted()),
            ]),
            metric("model", short(&model(app), 30), theme::text()),
            metric("thinking", thinking(app), theme::think()),
            metric("provider", provider, theme::muted()),
            metric("context", context, theme::text()),
            metric("workspace", short(&workspace, 30), theme::faint()),
        ],
    );
}

fn usage_card(app: &App, frame: &mut Frame, area: Rect) {
    let status = app.status.as_ref();
    let usage = status.and_then(|status| status.get("usage"));
    let read = |key: &str| {
        usage
            .and_then(|usage| usage.get(key))
            .map(|value| {
                (
                    value["input_tokens"].as_u64().unwrap_or(0),
                    value["output_tokens"].as_u64().unwrap_or(0),
                    value["cache_read_tokens"].as_u64().unwrap_or(0),
                )
            })
            .unwrap_or_else(|| {
                if key == "today" {
                    (
                        status
                            .and_then(|status| status["input_tokens"].as_u64())
                            .unwrap_or(0),
                        status
                            .and_then(|status| status["output_tokens"].as_u64())
                            .unwrap_or(0),
                        status
                            .and_then(|status| status["cache_read_tokens"].as_u64())
                            .unwrap_or(0),
                    )
                } else {
                    (0, 0, 0)
                }
            })
    };
    let rows = [
        ("today", read("today")),
        ("1 week", read("week")),
        ("1 month", read("month")),
        ("total", read("total")),
    ];
    let mut lines = vec![usage_header()];
    for (label, (input, output, _cache)) in rows {
        let label_style = if label == "total" {
            theme::accent_style()
        } else {
            theme::muted()
        };
        lines.push(usage_row(label, label_style, input, output));
    }
    let today = read("today");
    let hit_rate = hearth_runtime::cache_hit_rate(today.0 as i64, today.2 as i64) * 100.0;
    lines.push(metric(
        "cache hit",
        format!("{hit_rate:.1}% · {} read", compact_count(today.2)),
        theme::text(),
    ));
    card(frame, area, "Usage", lines);
}

fn compact_count(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}m", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn work_card(app: &App, frame: &mut Frame, area: Rect) {
    let status = app.status.as_ref();
    let running = status
        .and_then(|status| status["tasks_running"].as_u64())
        .unwrap_or(0);
    let asleep = status
        .and_then(|status| status["tasks_asleep"].as_u64())
        .unwrap_or(0);
    let memory = status
        .and_then(|status| status["memory_items"].as_u64())
        .unwrap_or(0);
    let standing = status
        .and_then(|status| status["standing_memory_version"].as_u64())
        .unwrap_or(0);
    let consolidation = status
        .and_then(|status| status["last_consolidation"].as_str())
        .unwrap_or("not yet");
    let review = status
        .and_then(|status| status["last_skill_review"].as_str())
        .unwrap_or("not yet");
    let changes = status
        .and_then(|status| status["last_skill_review_changes"].as_u64())
        .unwrap_or(0);
    let settings = app.settings_data.as_ref();
    let helpers = settings
        .map(|settings| {
            format!(
                "{} concurrent · depth {}",
                settings.tasks.max_concurrent, settings.tasks.max_depth
            )
        })
        .unwrap_or_else(|| "not configured".to_owned());
    let budgets = settings
        .map(|settings| {
            format!(
                "{} standing · {} recall",
                compact_count(settings.memory.standing_max_tokens),
                compact_count(settings.memory.recall_max_tokens)
            )
        })
        .unwrap_or_else(|| "not configured".to_owned());
    card(
        frame,
        area,
        "Work & memory",
        vec![
            metric(
                "tasks",
                format!("{running} running · {asleep} asleep"),
                theme::text(),
            ),
            metric("helpers", helpers, theme::text()),
            metric(
                "memory",
                format!("{memory} items · standing v{standing}"),
                theme::text(),
            ),
            metric("budgets", budgets, theme::muted()),
            metric("maintain", short(consolidation, 24), theme::muted()),
            metric(
                "skills",
                format!("{} · {changes} changes", short(review, 18)),
                theme::muted(),
            ),
        ],
    );
}

fn connections_card(app: &App, frame: &mut Frame, area: Rect) {
    let settings = app.settings_data.as_ref();
    let telegram = settings.and_then(|settings| settings.telegram.as_ref());
    let configured = telegram
        .and_then(|telegram| telegram.bot_token.as_ref())
        .is_some();
    let users = telegram
        .map(|telegram| telegram.allowed_user_ids.len())
        .unwrap_or(0);
    let endpoint = telegram
        .and_then(|telegram| telegram.base_url.as_deref())
        .unwrap_or("api.telegram.org");
    let telegram_line = if configured {
        format!("configured · {users} user(s) · {}", short(endpoint, 24))
    } else {
        "not configured".to_owned()
    };
    let web = settings.and_then(|settings| settings.web.as_ref());
    let search = web
        .map(|web| web.search.order.join(" → "))
        .unwrap_or_else(|| "not configured".to_owned());
    let fetch = web
        .map(|web| web.fetch.order.join(" → "))
        .unwrap_or_else(|| "not configured".to_owned());
    let media = settings.and_then(|settings| settings.media.as_ref());
    let services = [
        (
            "images",
            media.and_then(|media| media.image.as_ref()).is_some(),
        ),
        (
            "transcription",
            media.and_then(|media| media.stt.as_ref()).is_some(),
        ),
        (
            "speech",
            media.and_then(|media| media.tts.as_ref()).is_some(),
        ),
    ];
    let names = services
        .into_iter()
        .filter(|(_, on)| *on)
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    let media_line = if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    };
    card(
        frame,
        area,
        "Connections",
        vec![
            metric("Telegram", telegram_line, theme::text()),
            metric("search", short(&search, 30), theme::muted()),
            metric("fetch", short(&fetch, 30), theme::muted()),
            metric("media", media_line, theme::muted()),
        ],
    );
}

fn recent_card(app: &App, frame: &mut Frame, area: Rect) {
    let rows = activity::rows(&app.events);
    let limit = recent_limit(area);
    let lines = recent_lines(&rows, limit, app.local_offset);
    card(frame, area, "Recent activity", lines);
}

/// Visible rows inside a bordered card. `area` includes both border lines, so
/// the two-line allowance is exact and the newest row is never clipped.
fn recent_limit(area: Rect) -> usize {
    area.height.saturating_sub(2).max(1) as usize
}

/// Newest-last list of activity rows, trimmed to the newest `limit` entries.
fn recent_lines(
    rows: &[activity::ActivityRow],
    limit: usize,
    offset: time::UtcOffset,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return vec![Line::styled(
            "No live events yet. Send Hearth a Telegram message.",
            theme::muted(),
        )];
    }
    rows.iter()
        .rev()
        .take(limit.max(1))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|row| {
            let time = activity::event_time(row.timestamp, offset);
            Line::from(vec![
                Span::styled(format!("  {time} "), theme::muted()),
                Span::styled(
                    format!("{:<7}", kind_label(&row.kind)),
                    kind_style(&row.kind),
                ),
                Span::styled(short(&row.summary, 58), theme::text()),
            ])
        })
        .collect()
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "run_started" => "RUN",
        "turn_started" => "TURN",
        "task_step" => "STEP",
        "user_message" => "YOU",
        "assistant_reply" => "REPLY",
        "tool_call_started" => "TOOL",
        "tool_result" => "RESULT",
        "usage" => "USAGE",
        "run_settled" => "DONE",
        _ => "EVENT",
    }
}

fn kind_style(kind: &str) -> Style {
    match kind {
        "tool_call_started" => theme::warn(),
        "tool_result" | "usage" | "run_settled" => theme::good(),
        "user_message" | "run_started" | "turn_started" => theme::accent_style(),
        _ => theme::muted(),
    }
}

fn usage_header() -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<8}", "period"), theme::faint()),
        Span::styled(format!("{:>7}", "in"), theme::faint()),
        Span::styled(format!("{:>7}", "out"), theme::faint()),
    ])
}

fn usage_row(label: &str, label_style: Style, input: u64, output: u64) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<8}"), label_style),
        Span::styled(format!("{:>7}", compact_count(input)), theme::text()),
        Span::styled(format!("{:>7}", compact_count(output)), theme::text()),
    ])
}

fn metric(label: &str, value: impl Into<String>, style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<10} "), theme::muted()),
        Span::styled(value.into(), style),
    ])
}

fn model(app: &App) -> String {
    app.status
        .as_ref()
        .and_then(|status| status["model"].as_str())
        .or_else(|| {
            app.settings_data
                .as_ref()
                .and_then(|settings| settings.models.as_ref())
                .map(|models| models.model.as_str())
        })
        .unwrap_or("not configured")
        .to_owned()
}

fn thinking(app: &App) -> String {
    app.settings_data
        .as_ref()
        .and_then(|settings| settings.models.as_ref())
        .and_then(|models| models.thinking.clone())
        .unwrap_or_else(|| "provider default".to_owned())
}

fn card(frame: &mut Frame, area: Rect, title: &str, lines: Vec<Line<'_>>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(theme::heading(format!(" {title} ")));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn usage_rows_keep_their_columns_aligned() {
        let header = text(&usage_header());
        let month = text(&usage_row("1 month", theme::muted(), 2_000_000, 500_000));
        assert_eq!(
            header.find("in").unwrap() + 2,
            month.find("2.0m").unwrap() + 4
        );
        assert_eq!(
            header.find("out").unwrap() + 3,
            month.find("500.0k").unwrap() + 6
        );
    }

    fn activity_rows(count: usize) -> Vec<activity::ActivityRow> {
        (0..count)
            .map(|index| activity::ActivityRow {
                first: index,
                last: index,
                kind: "text_delta".to_owned(),
                timestamp: index as i64 * 1_000_000,
                summary: format!("reply {index}"),
                chunks: 1,
            })
            .collect()
    }

    #[test]
    fn recent_card_shows_the_newest_row_last() {
        let rows = activity_rows(40);
        let limit = recent_limit(Rect::new(0, 0, 40, 6));
        assert_eq!(limit, 4);
        let lines = recent_lines(&rows, limit, time::UtcOffset::UTC);
        assert_eq!(lines.len(), limit, "the newest rows must fill the card");
        let rendered = lines.iter().map(text).collect::<Vec<_>>();
        assert!(
            rendered.last().unwrap().contains("reply 39"),
            "newest row must be the final line: {rendered:?}"
        );
        assert!(
            !rendered
                .iter()
                .any(|line| line.contains("reply 39")
                    && !std::ptr::eq(line, rendered.last().unwrap())),
            "the newest row must appear once"
        );
        assert!(rendered[0].contains("reply 36"), "{rendered:?}");
    }

    #[test]
    fn recent_card_survives_a_tiny_pane_and_no_events() {
        let rows = activity_rows(3);
        assert_eq!(recent_limit(Rect::new(0, 0, 20, 1)), 1);
        assert_eq!(recent_limit(Rect::new(0, 0, 20, 0)), 1);
        let lines = recent_lines(
            &rows,
            recent_limit(Rect::new(0, 0, 40, 2)),
            time::UtcOffset::UTC,
        );
        assert_eq!(lines.len(), 1);
        assert!(text(&lines[0]).contains("reply 2"));
        let empty = recent_lines(&[], 4, time::UtcOffset::UTC);
        assert_eq!(empty.len(), 1);
        assert!(text(&empty[0]).contains("No live events yet"));
    }
}
