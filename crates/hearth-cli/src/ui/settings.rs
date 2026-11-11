//! Settings and daemon lifecycle controls.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{App, start_label};
use crate::theme;

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Min(4)])
        .split(area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(theme::heading(" Daemon settings "));
    let inner = block.inner(rows[0]);
    frame.render_widget(block, rows[0]);
    let options = [
        format!("Start: {}", start_label(&app.settings.daemon.start)),
        format!(
            "Keep running after this window closes: {}",
            if app.settings.daemon.keep_running {
                "Yes"
            } else {
                "No"
            }
        ),
        "All settings…".to_owned(),
        "Start daemon".to_owned(),
        "Stop daemon".to_owned(),
        "Restart daemon".to_owned(),
    ];
    let mut lines = Vec::new();
    for (index, option) in options.iter().enumerate() {
        let selected = index == app.settings.focus;
        let marker = if selected { "▸" } else { " " };
        let text = format!(" {marker} {option}");
        lines.push(if selected {
            Line::styled(text, theme::selected())
        } else {
            Line::styled(text, theme::text())
        });
    }
    lines.push(Line::from(""));
    let (model, thinking) = match app
        .settings_data
        .as_ref()
        .and_then(|settings| settings.models.as_ref())
    {
        Some(models) => (
            models.model.clone(),
            models
                .thinking
                .clone()
                .unwrap_or_else(|| "provider default".to_owned()),
        ),
        None => ("not configured".to_owned(), "not configured".to_owned()),
    };
    lines.push(Line::from(vec![
        Span::styled("  model     ", theme::muted()),
        Span::styled(model, theme::text()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  thinking  ", theme::muted()),
        Span::styled(thinking, theme::think()),
    ]));
    lines.push(Line::styled(
        "  Press e to retry loading all settings.",
        theme::faint(),
    ));
    frame.render_widget(Paragraph::new(lines), inner);

    let detail = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(theme::heading(" Configuration "));
    let inner = detail.inner(rows[1]);
    frame.render_widget(detail, rows[1]);
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("home      ", theme::muted()),
        Span::styled(app.home.display().to_string(), theme::text()),
    ]));
    if let Some(settings) = &app.settings_data {
        if let Some(models) = &settings.models {
            lines.push(Line::from(vec![
                Span::styled("provider  ", theme::muted()),
                Span::styled(models.base_url.clone(), theme::text()),
            ]));
        }
        if let Some(telegram) = &settings.telegram {
            lines.push(Line::from(vec![
                Span::styled("telegram  ", theme::muted()),
                Span::styled(
                    format!("{} allowed user(s)", telegram.allowed_user_ids.len()),
                    theme::text(),
                ),
            ]));
        }
        let media = settings.media.as_ref();
        let services = [
            media.and_then(|media| media.image.as_ref()).is_some(),
            media.and_then(|media| media.stt.as_ref()).is_some(),
            media.and_then(|media| media.tts.as_ref()).is_some(),
        ];
        let names = ["images", "transcription", "speech"]
            .into_iter()
            .zip(services)
            .filter(|(_, on)| *on)
            .map(|(name, _)| name)
            .collect::<Vec<_>>();
        lines.push(Line::from(vec![
            Span::styled("optional  ", theme::muted()),
            Span::styled(
                if names.is_empty() {
                    "none".to_owned()
                } else {
                    names.join(", ")
                },
                theme::text(),
            ),
        ]));
    } else {
        lines.push(Line::styled(
            "Settings could not be loaded. Run the doctor checks.",
            theme::bad(),
        ));
    }
    if let Some(message) = &app.settings.message {
        lines.push(Line::from(""));
        lines.push(Line::styled(message.clone(), theme::warn()));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
