//! Skills browser.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::App;
use crate::theme;
use crate::ui::activity;

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(4)])
        .split(area);
    let summary = match &app.skills.stats {
        Some(stats) => format!(
            "{} skills · active {} · learned {}",
            stats.total, stats.active, stats.learned
        ),
        None => "Skills".to_owned(),
    };
    frame.render_widget(
        Paragraph::new(Line::styled(summary, theme::muted())),
        rows[0],
    );
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(rows[1]);
    let list = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(theme::heading(" Library "));
    let inner = list.inner(columns[0]);
    frame.render_widget(list, columns[0]);
    if let Some(error) = &app.skills.error {
        frame.render_widget(
            Paragraph::new(error.clone()).wrap(Wrap { trim: false }),
            inner,
        );
    } else if app.skills.skills.is_empty() {
        frame.render_widget(
            Paragraph::new("No skills yet. Press n to write one, or let Hearth learn one.")
                .wrap(Wrap { trim: false }),
            inner,
        );
    } else {
        let height = inner.height.max(1) as usize;
        let start = activity::list_start(app.skills.selected, app.skills.skills.len(), height);
        let mut lines = Vec::new();
        for (index, skill) in app
            .skills
            .skills
            .iter()
            .enumerate()
            .skip(start)
            .take(height)
        {
            let selected = index == app.skills.selected;
            let marker = if selected { "▶ " } else { "  " };
            let label = format!(
                "{marker}{} [{}] {:.0}%",
                skill.name,
                skill.scope.as_str(),
                skill.success_rate() * 100.0
            );
            lines.push(if selected {
                Line::raw(label).style(theme::selected())
            } else {
                Line::raw(label)
            });
        }
        frame.render_widget(Paragraph::new(lines), inner);
    }
    let body = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(theme::heading(" Skill "));
    let inner = body.inner(columns[1]);
    frame.render_widget(body, columns[1]);
    match &app.skills.body {
        Some(text) => {
            let paragraph = Paragraph::new(text.as_str()).wrap(Wrap { trim: false });
            let max_scroll = paragraph
                .line_count(inner.width)
                .saturating_sub(inner.height as usize);
            app.skills.max_scroll.set(max_scroll);
            let scroll = app.skills.scroll.min(max_scroll).min(u16::MAX as usize) as u16;
            frame.render_widget(paragraph.scroll((scroll, 0)), inner);
        }
        None => {
            app.skills.max_scroll.set(0);
            frame.render_widget(
                Paragraph::new("Select a skill to read it.").wrap(Wrap { trim: false }),
                inner,
            );
        }
    }
    new_skill_prompt(app, frame, area);
}

fn new_skill_prompt(app: &App, frame: &mut Frame, area: Rect) {
    let Some(input) = &app.skills.new_name else {
        return;
    };
    let width = area.width.saturating_sub(10).min(60);
    let height = 6;
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_focus())
        .title(theme::heading(" New user skill "));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let lines = vec![
        Line::styled(" Name", theme::muted()),
        Line::from(vec![
            Span::styled(" › ", theme::accent_style()),
            Span::styled(format!(" {} ", input.value), theme::field_focus()),
        ]),
        Line::from(""),
        Line::styled(
            " Letters, numbers, spaces, - or _. Enter creates, Esc cancels.",
            theme::faint(),
        ),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
    frame.set_cursor_position((inner.x + 4 + input.cursor as u16, inner.y + 1));
}
