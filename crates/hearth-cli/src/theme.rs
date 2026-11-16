//! Shared terminal theme and small widgets.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub const ACCENT: Color = Color::Rgb(103, 232, 249);
pub const TEXT: Color = Color::Rgb(226, 232, 240);
pub const MUTED: Color = Color::Rgb(148, 163, 184);
pub const FAINT: Color = Color::Rgb(124, 138, 158);
pub const GOOD: Color = Color::Rgb(134, 239, 172);
pub const WARN: Color = Color::Rgb(253, 224, 71);
pub const BAD: Color = Color::Rgb(252, 165, 165);
pub const THINK: Color = Color::Rgb(196, 181, 253);
pub const KEY_BG: Color = Color::Rgb(51, 65, 85);
pub const SELECT_BG: Color = Color::Rgb(14, 116, 144);
pub const SELECT_FG: Color = Color::Rgb(236, 254, 255);
pub const FIELD_BG: Color = Color::Rgb(30, 41, 59);
pub const FIELD_FOCUS_BG: Color = Color::Rgb(21, 94, 117);
/// The band behind a message the person wrote.
pub const USER_BG: Color = Color::Rgb(23, 52, 64);
/// The band behind a tool call.
pub const TOOL_BG: Color = Color::Rgb(40, 40, 30);
/// The band behind a tool result that came back.
pub const TOOL_OK_BG: Color = Color::Rgb(22, 46, 36);
/// The band behind a tool result the harness could not finish.
pub const TOOL_BAD_BG: Color = Color::Rgb(54, 30, 34);
/// The band behind a context layer or a memory receipt.
pub const CTX_BG: Color = Color::Rgb(26, 31, 40);

pub fn accent_style() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn text() -> Style {
    Style::default().fg(TEXT)
}

pub fn heading(text: impl Into<String>) -> Line<'static> {
    Line::styled(text.into(), accent_style())
}

pub fn muted() -> Style {
    Style::default().fg(MUTED)
}

pub fn faint() -> Style {
    Style::default().fg(FAINT)
}

pub fn border() -> Style {
    Style::default().fg(FAINT)
}

pub fn border_focus() -> Style {
    Style::default().fg(ACCENT)
}

pub fn good() -> Style {
    Style::default().fg(GOOD)
}

pub fn warn() -> Style {
    Style::default().fg(WARN)
}

pub fn bad() -> Style {
    Style::default().fg(BAD)
}

pub fn think() -> Style {
    Style::default().fg(THINK)
}

pub fn selected() -> Style {
    Style::default()
        .bg(SELECT_BG)
        .fg(SELECT_FG)
        .add_modifier(Modifier::BOLD)
}

pub fn field() -> Style {
    Style::default().fg(TEXT).bg(FIELD_BG)
}

pub fn field_focus() -> Style {
    Style::default()
        .fg(Color::White)
        .bg(FIELD_FOCUS_BG)
        .add_modifier(Modifier::BOLD)
}

/// Text on a panel band.
pub fn panel_text(bg: Color) -> Style {
    Style::default().fg(TEXT).bg(bg)
}

/// The right-aligned meta of a panel header, and the fold stub under it.
pub fn panel_stub(bg: Color) -> Style {
    Style::default().fg(FAINT).bg(bg)
}

/// A blank fill for a panel row, so its band reaches both edges.
pub fn panel_fill(bg: Color) -> Style {
    Style::default().bg(bg)
}

pub fn key_hint(key: &str) -> Span<'static> {
    Span::styled(format!(" {key} "), Style::default().fg(TEXT).bg(KEY_BG))
}

pub fn hints(pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (key, label)) in pairs.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(key_hint(key));
        spans.push(Span::styled(format!(" {label}"), muted()));
    }
    Line::from(spans)
}

pub fn label(text: &str) -> Span<'static> {
    Span::styled(text.to_owned(), muted())
}

pub fn value(text_value: impl Into<String>) -> Span<'static> {
    Span::styled(text_value.into(), text())
}
