//! Saved memory browser.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::App;
use crate::theme;
use crate::ui::short;

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    list(app, frame, columns[0]);
    detail(app, frame, columns[1]);
}

fn list(app: &App, frame: &mut Frame, area: Rect) {
    let records = app.memory.filtered();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(theme::heading(format!(
            " Memory · {} saved ",
            records.len()
        )));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);
    filter(app, frame, rows[0]);
    if let Some(error) = &app.memory.error {
        frame.render_widget(
            Paragraph::new(error.clone())
                .style(theme::bad())
                .wrap(Wrap { trim: false }),
            rows[1],
        );
        return;
    }
    if records.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled("No memories match.", theme::muted()),
                Line::from(""),
                Line::styled(
                    "Facts the agent chose to remember, plus the standing bullets injected every turn.",
                    theme::faint(),
                ),
            ])
            .wrap(Wrap { trim: false }),
            rows[1],
        );
        return;
    }
    let height = rows[1].height.max(1) as usize;
    let start = app
        .memory
        .selected
        .saturating_sub(height.saturating_sub(1) / 2);
    let mut lines = Vec::new();
    for (index, record) in records.iter().enumerate().skip(start).take(height) {
        let selected = index == app.memory.selected;
        let marker = if selected { "▸" } else { " " };
        let (status, status_style) = status(record);
        let kind = short(&crate::app::store_label(record).to_uppercase(), 10);
        let content_width = rows[1].width.saturating_sub(20) as usize;
        let spans = vec![
            Span::styled(
                format!(" {marker} "),
                if selected {
                    Style::default().fg(theme::SELECT_FG).bg(theme::SELECT_BG)
                } else {
                    theme::faint()
                },
            ),
            Span::styled(format!("{status} "), status_style),
            Span::styled(format!("{kind:<10} "), theme::muted()),
            Span::styled(short(&record.content, content_width), theme::text()),
        ];
        let line = Line::from(spans);
        lines.push(if selected {
            line.style(Style::default().bg(theme::SELECT_BG).fg(theme::SELECT_FG))
        } else {
            line
        });
    }
    frame.render_widget(Paragraph::new(lines), rows[1]);
}

fn filter(app: &App, frame: &mut Frame, area: Rect) {
    let line = if app.memory.filter_mode || !app.memory.filter.is_empty() {
        Line::from(vec![
            Span::styled(" Search  ", theme::accent_style()),
            Span::styled(app.memory.filter.clone(), theme::text()),
            Span::styled("▏", theme::accent_style()),
        ])
    } else {
        Line::from(vec![
            Span::styled(" ", theme::muted()),
            theme::key_hint("search"),
            Span::styled(" filter by content, kind, tag, or status", theme::faint()),
        ])
    };
    frame.render_widget(Paragraph::new(line.clone()), area);
    if app.memory.filter_mode {
        let x = area.x + 9 + app.memory.filter.chars().count() as u16;
        frame.set_cursor_position((x, area.y));
    }
}

fn detail(app: &App, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(theme::heading(" Memory detail "));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(record) = app.memory.selected_record() else {
        frame.render_widget(
            Paragraph::new("Select a memory with the arrow keys.")
                .style(theme::faint())
                .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    };
    let mut lines = vec![Line::from("")];
    for line in record.content.lines() {
        lines.push(Line::styled(format!("  {line}"), theme::text()));
    }
    lines.push(Line::from(""));
    meta(&mut lines, "store", crate::app::store_label(record));
    if !record.source.is_empty() {
        meta(&mut lines, "source", &record.source);
    }
    if record.revision > 0 {
        meta(&mut lines, "revision", &record.revision.to_string());
    }
    if !record.tags.is_empty() {
        meta(&mut lines, "tags", &record.tags.join(", "));
    }
    if !record.created_at.is_empty() {
        meta(&mut lines, "created", &record.created_at);
    }
    if !record.updated_at.is_empty() {
        meta(&mut lines, "updated", &record.updated_at);
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        format!("  id {}", short(&record.id, 28)),
        theme::faint(),
    ));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn status(record: &hearth_runtime::inspect::MemoryRecord) -> (&'static str, Style) {
    if record.standing {
        ("★", theme::warn())
    } else {
        ("●", theme::good())
    }
}

fn meta(lines: &mut Vec<Line<'static>>, label: &str, value: &str) {
    lines.push(Line::from(vec![
        Span::styled(format!("  {label:<11}"), theme::muted()),
        Span::styled(value.to_owned(), theme::text()),
    ]));
}
