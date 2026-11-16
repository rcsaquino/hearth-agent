//! Skills browser.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

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
            "{} skills · active {} · learned {} · showing {}",
            stats.total,
            stats.active,
            stats.learned,
            app.skills.filter.label()
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
            Paragraph::new(app.skills.filter.empty()).wrap(Wrap { trim: false }),
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
                "{marker}{} [{}] {:>4}",
                skill.name,
                skill.scope.as_str(),
                rate(skill)
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
}

/// The rate column. A skill with no counted use has no measured rate, so it
/// shows a dash instead of a zero that reads as a failure.
fn rate(skill: &hearth_runtime::skills::Skill) -> String {
    if skill.uses == 0 {
        "—".to_owned()
    } else {
        format!("{:.0}%", skill.success_rate() * 100.0)
    }
}
