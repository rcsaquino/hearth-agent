//! Live activity feed and event detail.

use std::collections::VecDeque;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use serde_json::Value;
use time::{OffsetDateTime, UtcOffset};

use crate::app::App;
use crate::theme;
use crate::ui::{event_summary, short};

/// One visible activity row. Streaming deltas are coalesced so the feed stays readable.
pub struct ActivityRow {
    pub first: usize,
    pub last: usize,
    pub kind: String,
    pub timestamp: i64,
    pub summary: String,
    pub chunks: usize,
}

/// One row per event, oldest first. Streaming deltas are no longer emitted, so
/// every event is a single immutable row: the feed only ever grows at the end
/// and rows already shown are never rewritten.
pub fn rows(events: &VecDeque<Value>) -> Vec<ActivityRow> {
    events
        .iter()
        .enumerate()
        .map(|(index, event)| ActivityRow {
            first: index,
            last: index,
            kind: event["kind"].as_str().unwrap_or("event").to_owned(),
            timestamp: event["ts"].as_i64().unwrap_or(0),
            summary: event_summary(event),
            chunks: 1,
        })
        .collect()
}

pub fn row_count(events: &VecDeque<Value>) -> usize {
    rows(events).len()
}

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(area);
    let rows = rows(&app.events);
    let selected = app.activity_selected.min(rows.len().saturating_sub(1));
    list(app, frame, columns[0], &rows, selected);
    detail(app, frame, columns[1], &rows, selected);
}

fn list(app: &App, frame: &mut Frame, area: Rect, rows: &[ActivityRow], selected: usize) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(theme::heading(if app.activity_paused {
            " Activity · paused "
        } else {
            " Activity · live "
        }));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.events.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled("No live events yet.", theme::muted()),
                Line::from(""),
                Line::styled(
                    "Send Hearth a Telegram message; runs appear here as they happen.",
                    theme::faint(),
                ),
            ])
            .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    let height = inner.height.max(1) as usize;
    let start = list_start(selected, rows.len(), height);
    let mut lines = Vec::new();
    for (index, row) in rows.iter().enumerate().skip(start).take(height) {
        let is_selected = index == selected;
        let marker = if is_selected { "▶" } else { " " };
        let (tag, style) = kind_badge(&row.kind, row.first, app);
        let time = event_time(row.timestamp, app.local_offset);
        let summary_width = inner.width.saturating_sub(24) as usize;
        let summary = short(&row.summary, summary_width.max(8));
        let spans = vec![
            Span::styled(
                format!(" {marker} "),
                if is_selected {
                    theme::selected()
                } else {
                    theme::faint()
                },
            ),
            Span::styled(format!("{time} "), theme::muted()),
            Span::styled(format!("{tag:<7}"), style),
            Span::styled(summary, theme::text()),
        ];
        let line = Line::from(spans);
        lines.push(if is_selected {
            line.style(Style::default().bg(theme::SELECT_BG).fg(theme::SELECT_FG))
        } else {
            line
        });
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn detail(app: &App, frame: &mut Frame, area: Rect, rows: &[ActivityRow], selected: usize) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(theme::heading(" Event detail "));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(row) = rows.get(selected) else {
        frame.render_widget(
            Paragraph::new("Select an event with the arrow keys.")
                .style(theme::faint())
                .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    };
    let Some(first) = app.events.get(row.first) else {
        return;
    };
    let (tag, style) = kind_badge(&row.kind, row.first, app);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!(" {tag} "),
                style.add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Span::styled(
                format!("  {}", event_time_full(row.timestamp, app.local_offset)),
                theme::muted(),
            ),
        ]),
        Line::from(""),
    ];
    push_meta(&mut lines, "task", first["task_id"].as_str());
    push_meta(
        &mut lines,
        "conversation",
        first["conversation_id"].as_str(),
    );
    if row.chunks > 1 {
        push_meta(
            &mut lines,
            "stream",
            Some(&format!("{} chunks", row.chunks)),
        );
    }
    lines.push(Line::from(""));
    render_event(first, &mut lines);
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let rendered = paragraph.line_count(inner.width);
    let scroll = detail_scroll(
        selected,
        rows.len(),
        app.activity_paused,
        rendered,
        inner.height,
    );
    frame.render_widget(paragraph.scroll((scroll, 0)), inner);
}

/// First visible row. The live (last) row is pinned to the bottom edge so the
/// newest text never drifts out of view; any other selection stays centered.
pub(crate) fn list_start(selected: usize, total: usize, height: usize) -> usize {
    if total > 0 && selected + 1 == total {
        (selected + 1).saturating_sub(height.max(1))
    } else {
        selected.saturating_sub(height.saturating_sub(1) / 2)
    }
}

/// Vertical offset for the detail pane. Only the live streaming row is
/// bottom-pinned; a paused feed or an older selection stays at the top so the
/// reader keeps their place.
fn detail_scroll(
    selected: usize,
    total: usize,
    paused: bool,
    rendered_lines: usize,
    height: u16,
) -> u16 {
    let pinned = !paused && total > 0 && selected + 1 == total;
    if !pinned {
        return 0;
    }
    let visible = usize::from(height).max(1);
    rendered_lines
        .saturating_sub(visible)
        .min(u16::MAX as usize) as u16
}

fn render_event(event: &Value, lines: &mut Vec<Line<'static>>) {
    let data = &event["data"];
    let kind = event["kind"].as_str().unwrap_or("event");
    match kind {
        "user_message" => push_block(
            lines,
            "message",
            data["text"].as_str(),
            theme::accent_style(),
        ),
        "assistant_reply" => {
            let calls = data["tool_calls"].as_u64().unwrap_or(0);
            push_meta(lines, "tool calls", Some(&calls.to_string()));
            push_block(lines, "thinking", data["thinking"].as_str(), theme::think());
            push_block(lines, "reply", data["text"].as_str(), theme::text());
        }
        "run_started" => push_meta(lines, "submission", data["submission_id"].as_str()),
        "turn_started" => push_meta(lines, "turn", Some(&data["turn"].to_string())),
        "task_step" => {
            push_meta(lines, "phase", data["phase"].as_str());
            push_meta(lines, "turn", Some(&data["turn"].to_string()));
        }
        "tool_call_started" => {
            push_meta(lines, "tool", data["name"].as_str());
            push_meta(lines, "call", data["call_id"].as_str());
            push_block(
                lines,
                "arguments",
                data["arguments"].as_str(),
                theme::muted(),
            );
        }
        "tool_result" => {
            let failed = data["is_error"].as_bool().unwrap_or(false);
            push_meta(lines, "result", Some(if failed { "error" } else { "ok" }));
            push_meta(lines, "call", data["call_id"].as_str());
            push_block(
                lines,
                "content",
                data["content"].as_str(),
                if failed { theme::bad() } else { theme::text() },
            );
        }
        "usage" => {
            push_meta(
                lines,
                "input",
                data["input_tokens"]
                    .as_u64()
                    .map(crate::ui::count)
                    .as_deref(),
            );
            push_meta(
                lines,
                "output",
                data["output_tokens"]
                    .as_u64()
                    .map(crate::ui::count)
                    .as_deref(),
            );
            push_meta(
                lines,
                "cache read",
                data["cache_read_tokens"]
                    .as_u64()
                    .map(crate::ui::count)
                    .as_deref(),
            );
        }
        "run_settled" => {
            let status = data["status"].as_str().unwrap_or("settled");
            let style = if status == "done" {
                theme::good()
            } else {
                theme::warn()
            };
            lines.push(Line::styled(format!("  status  {status}"), style));
        }
        _ => {
            if let Some(object) = data.as_object() {
                if object.is_empty() {
                    lines.push(Line::styled("  (no extra detail)", theme::faint()));
                }
                for (key, value) in object {
                    let text = value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string());
                    push_block(lines, key, Some(&text), theme::text());
                }
            }
        }
    }
}

fn push_meta(lines: &mut Vec<Line<'static>>, label: &str, value: Option<&str>) {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return;
    };
    lines.push(Line::from(vec![
        Span::styled(format!("  {label:<13}"), theme::muted()),
        Span::styled(short(value, 64), theme::text()),
    ]));
}

fn push_block(lines: &mut Vec<Line<'static>>, label: &str, value: Option<&str>, style: Style) {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return;
    };
    lines.push(Line::styled(format!("  {label}"), theme::muted()));
    for line in value.lines() {
        lines.push(Line::styled(format!("  │ {line}"), style));
    }
}

fn kind_badge(kind: &str, first: usize, app: &App) -> (&'static str, Style) {
    match kind {
        "run_started" => ("RUN", theme::accent_style()),
        "turn_started" => ("TURN", theme::accent_style()),
        "task_step" => ("STEP", theme::muted()),
        "user_message" => ("YOU", theme::accent_style()),
        "assistant_reply" => ("REPLY", theme::text()),
        "tool_call_started" => ("TOOL", theme::warn()),
        "tool_result" => {
            let failed = app
                .events
                .get(first)
                .and_then(|event| event["data"]["is_error"].as_bool())
                .unwrap_or(false);
            if failed {
                ("RESULT", theme::bad())
            } else {
                ("RESULT", theme::good())
            }
        }
        "usage" => ("USAGE", theme::good()),
        "run_settled" => {
            let status = app
                .events
                .get(first)
                .and_then(|event| event["data"]["status"].as_str())
                .unwrap_or_default();
            if status == "done" {
                ("DONE", theme::good())
            } else {
                ("STOP", theme::warn())
            }
        }
        _ => ("EVENT", theme::muted()),
    }
}

pub(crate) fn event_time(micros: i64, offset: UtcOffset) -> String {
    let time = OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH);
    let local = time.to_offset(offset);
    format!(
        "{:02}:{:02}:{:02}",
        local.hour(),
        local.minute(),
        local.second()
    )
}

fn event_time_full(micros: i64, offset: UtcOffset) -> String {
    let time = OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH);
    let local = time.to_offset(offset);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        local.year(),
        u8::from(local.month()),
        local.day(),
        local.hour(),
        local.minute(),
        local.second()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn events() -> VecDeque<Value> {
        [
            json!({"ts": 1, "kind": "run_started", "task_id": "t", "data": {}}),
            json!({"ts": 2, "kind": "user_message", "task_id": "t", "data": {"text": "please inspect"}}),
            json!({"ts": 3, "kind": "tool_call_started", "task_id": "t", "data": {"name": "read_file"}}),
            json!({"ts": 4, "kind": "assistant_reply", "task_id": "t", "data": {"text": "answer", "thinking": "", "tool_calls": 0}}),
            json!({"ts": 5, "kind": "run_settled", "task_id": "t", "data": {"status": "done"}}),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn every_event_is_its_own_row_and_rows_never_merge() {
        let rows = rows(&events());
        // One row per event: nothing is coalesced, so the feed only grows.
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].kind, "run_started");
        assert_eq!(rows[1].kind, "user_message");
        assert!(
            rows[1].summary.contains("please inspect"),
            "{:?}",
            rows[1].summary
        );
        assert_eq!(rows[2].kind, "tool_call_started");
        assert_eq!(rows[3].kind, "assistant_reply");
        assert_eq!(rows[3].summary, "Reply · answer");
        assert_eq!(rows[4].kind, "run_settled");
        for row in &rows {
            assert_eq!(row.first, row.last, "a row must own exactly one event");
        }
    }

    #[test]
    fn appending_an_event_never_rewrites_earlier_rows() {
        let mut events = events();
        let before = rows(&events);
        events.push_back(
            json!({"ts": 6, "kind": "user_message", "task_id": "t", "data": {"text": "next"}}),
        );
        let after = rows(&events);
        assert_eq!(after.len(), before.len() + 1);
        for (old, new) in before.iter().zip(after.iter()) {
            assert_eq!(old.kind, new.kind);
            assert_eq!(old.summary, new.summary, "an existing row was rewritten");
        }
    }

    #[test]
    fn a_user_message_and_a_reply_both_appear() {
        let rows = rows(&events());
        let labels: Vec<&str> = rows.iter().map(|row| row.kind.as_str()).collect();
        assert!(labels.contains(&"user_message"));
        assert!(labels.contains(&"assistant_reply"));
        assert!(
            !labels.iter().any(|kind| kind.ends_with("_delta")),
            "deltas must not stream into the feed: {labels:?}"
        );
    }

    #[test]
    fn the_live_row_sits_on_the_last_visible_line() {
        // 20 rows in a 6-line pane: the newest row must land on the bottom.
        let start = list_start(19, 20, 6);
        assert_eq!(start, 14);
        assert_eq!(start + 6, 20);
        // Older selections keep the centered behaviour and never underflow.
        assert_eq!(list_start(0, 20, 6), 0);
        assert_eq!(list_start(10, 20, 6), 8);
        assert_eq!(list_start(0, 1, 6), 0);
        assert_eq!(list_start(0, 0, 0), 0);
    }

    #[test]
    fn streaming_detail_scrolls_to_the_newest_line() {
        let text = (0..40)
            .map(|index| format!("streamed line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let lines = text
            .lines()
            .map(|line| Line::styled(line.to_owned(), theme::text()))
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        let rendered = paragraph.line_count(40);
        assert!(rendered > 10, "expected wrapped lines, got {rendered}");
        let scroll = detail_scroll(2, 3, false, rendered, 10);
        assert!(scroll > 0, "the live row must scroll into view");
        assert_eq!(
            usize::from(scroll) + 10,
            rendered.min(usize::from(scroll) + 10),
            "scroll must keep the last line inside the pane"
        );
        // Paused or older selections never force a scroll.
        assert_eq!(detail_scroll(2, 3, true, rendered, 10), 0);
        assert_eq!(detail_scroll(1, 3, false, rendered, 10), 0);
        // Content shorter than the pane cannot scroll.
        assert_eq!(detail_scroll(0, 1, false, 3, 10), 0);
    }
}
