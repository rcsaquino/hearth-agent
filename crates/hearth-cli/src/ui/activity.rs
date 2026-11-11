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

pub fn rows(events: &VecDeque<Value>) -> Vec<ActivityRow> {
    let mut rows = Vec::new();
    let mut index = 0;
    while index < events.len() {
        let event = &events[index];
        let kind = event["kind"].as_str().unwrap_or("event");
        let timestamp = event["ts"].as_i64().unwrap_or(0);
        if matches!(kind, "text_delta" | "thinking_delta") {
            let task = event["task_id"].as_str().unwrap_or_default();
            let mut text = event["data"]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let mut last = index;
            while last + 1 < events.len() {
                let next = &events[last + 1];
                if next["kind"].as_str() != Some(kind)
                    || next["task_id"].as_str().unwrap_or_default() != task
                {
                    break;
                }
                text.push_str(next["data"]["text"].as_str().unwrap_or_default());
                last += 1;
            }
            let label = if kind == "thinking_delta" {
                "Thinking"
            } else {
                "Reply"
            };
            rows.push(ActivityRow {
                first: index,
                last,
                kind: kind.to_owned(),
                timestamp,
                summary: format!("{label} · {}", one_line(&text, 120)),
                chunks: last - index + 1,
            });
            index = last + 1;
        } else {
            rows.push(ActivityRow {
                first: index,
                last: index,
                kind: kind.to_owned(),
                timestamp,
                summary: event_summary(event),
                chunks: 1,
            });
            index += 1;
        }
    }
    rows
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
    let start = selected.saturating_sub(height.saturating_sub(1) / 2);
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
    if matches!(row.kind.as_str(), "text_delta" | "thinking_delta") {
        let text = app
            .events
            .iter()
            .skip(row.first)
            .take(row.last - row.first + 1)
            .filter_map(|event| event["data"]["text"].as_str())
            .collect::<String>();
        let style = if row.kind == "thinking_delta" {
            theme::think()
        } else {
            theme::text()
        };
        for line in text.lines() {
            lines.push(Line::styled(format!("  {line}"), style));
        }
    } else {
        render_event(first, &mut lines);
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn render_event(event: &Value, lines: &mut Vec<Line<'static>>) {
    let data = &event["data"];
    let kind = event["kind"].as_str().unwrap_or("event");
    match kind {
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
        "thinking_delta" => ("THINK", theme::think()),
        "text_delta" => ("REPLY", theme::text()),
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

fn one_line(text: &str, width: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    short(&collapsed, width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn events() -> VecDeque<Value> {
        [
            json!({"ts": 1, "kind": "run_started", "task_id": "t", "data": {}}),
            json!({"ts": 2, "kind": "thinking_delta", "task_id": "t", "data": {"text": "first "}}),
            json!({"ts": 3, "kind": "thinking_delta", "task_id": "t", "data": {"text": "second"}}),
            json!({"ts": 4, "kind": "text_delta", "task_id": "t", "data": {"text": "answer"}}),
            json!({"ts": 5, "kind": "run_settled", "task_id": "t", "data": {"status": "done"}}),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn adjacent_deltas_are_one_readable_row() {
        let rows = rows(&events());
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1].kind, "thinking_delta");
        assert_eq!(rows[1].chunks, 2);
        assert!(rows[1].summary.contains("first second"));
        assert_eq!(rows[2].kind, "text_delta");
        assert_eq!(rows[2].chunks, 1);
    }
}
