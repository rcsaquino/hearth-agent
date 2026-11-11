//! Friendly all-settings form.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::settings_form::{RowKind, SettingsForm, SettingsPicker};
use crate::theme;
use crate::ui::short;

pub fn render(form: &SettingsForm, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_focus())
        .title(theme::heading(" All settings "));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let footer_height = if form.edit.is_some() { 6 } else { 4 };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(footer_height)])
        .split(inner);
    let cursor = list(form, frame, rows[0]);
    if let Some(cursor) = cursor {
        frame.set_cursor_position(cursor);
    }
    footer(form, frame, rows[1]);
    if let Some(picker) = &form.picker {
        picker_popup(picker, frame, inner);
    }
}

fn list(form: &SettingsForm, frame: &mut Frame, area: Rect) -> Option<(u16, u16)> {
    let height = area.height.max(1) as usize;
    let label_width = ((area.width as usize * 40) / 100).clamp(20, 38);
    let prefix_width = label_width + 4;
    let value_width = (area.width as usize)
        .saturating_sub(prefix_width + 2)
        .max(4);
    let start = page_start(form.selected, form.rows.len(), height);
    let mut lines = Vec::new();
    let mut cursor = None;
    for index in start..(start + height).min(form.rows.len()) {
        let row = &form.rows[index];
        let selected = index == form.selected;
        if row.kind == RowKind::Header {
            let rule = "─".repeat((area.width as usize).saturating_sub(row.label.len() + 4));
            lines.push(Line::from(vec![
                Span::styled(format!(" {} ", row.label), theme::accent_style()),
                Span::styled(rule, theme::faint()),
            ]));
            continue;
        }
        let marker = if selected { "▸" } else { " " };
        let changed = if row.modified { "*" } else { " " };
        let marker_text = format!("{marker}{changed}");
        let label_text = format!(" {:<label_width$} ", short(&row.label, label_width));
        let label = format!("{marker_text}{label_text}");
        let label_style = if selected {
            Style::default().fg(theme::SELECT_FG).bg(theme::SELECT_BG)
        } else {
            theme::muted()
        };
        let marker_style = if selected {
            label_style
        } else if row.modified {
            theme::warn()
        } else {
            theme::muted()
        };
        let editing = form.edit.as_ref().is_some_and(|edit| edit.row == index);
        let mut spans = vec![
            Span::styled(marker_text, marker_style),
            Span::styled(label_text, label_style),
        ];
        if editing {
            let edit = form.edit.as_ref().expect("edit");
            let (shown, cursor_col) =
                line_window(&edit.input.masked(), edit.input.cursor, value_width);
            spans.push(Span::styled(
                format!(" {shown:<value_width$} "),
                theme::field_focus(),
            ));
            cursor = Some((
                area.x + UnicodeWidthStr::width(label.as_str()) as u16 + 1 + cursor_col,
                area.y + (index - start) as u16,
            ));
        } else {
            let style = value_style(row.kind, &row.value, selected);
            spans.push(Span::styled(
                format!(" {:<value_width$} ", short(&row.value, value_width)),
                style,
            ));
        }
        let line = Line::from(spans);
        lines.push(if selected && !editing {
            line.style(Style::default().bg(theme::SELECT_BG).fg(theme::SELECT_FG))
        } else {
            line
        });
    }
    frame.render_widget(Paragraph::new(lines), area);
    cursor
}

fn footer(form: &SettingsForm, frame: &mut Frame, area: Rect) {
    if let Some(edit) = &form.edit {
        let label = form
            .rows
            .get(edit.row)
            .map(|row| row.label.as_str())
            .unwrap_or("Setting");
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::border_focus())
            .title(theme::heading(format!(" Tip · {label} ")));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let mut lines = vec![
            Line::from(""),
            Line::styled(format!(" {}", edit.hint), theme::text()),
        ];
        if let Some(error) = &form.error {
            lines.push(Line::styled(format!(" {error}"), theme::bad()));
        } else {
            lines.push(theme::hints(&[("enter", "save"), ("esc", "cancel")]));
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }

    let mut lines = vec![Line::from("")];
    let Some(row) = form.selected_row() else {
        lines.push(Line::styled(
            " Changes save as you go; only changed values are stored.",
            theme::faint(),
        ));
        frame.render_widget(Paragraph::new(lines), area);
        return;
    };
    let tip = if row.tooltip.is_empty() {
        row.help
    } else {
        row.tooltip
    };
    lines.push(Line::from(vec![
        Span::styled(format!(" Tip · {}  ", row.label), theme::accent_style()),
        Span::styled(tip, theme::faint()),
    ]));
    if let Some(error) = &form.error {
        lines.push(Line::styled(format!(" {error}"), theme::bad()));
    } else if let Some(notice) = &form.notice {
        lines.push(Line::styled(format!(" {notice}"), theme::good()));
    } else {
        lines.push(Line::from(""));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn picker_popup(picker: &SettingsPicker, frame: &mut Frame, area: Rect) {
    let width = area.width.saturating_sub(8).min(44);
    let height = (picker.options.len() as u16 + 2)
        .min(area.height.saturating_sub(2))
        .max(3);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    frame.render_widget(ratatui::widgets::Clear, popup);
    let visible = height.saturating_sub(2) as usize;
    let start = picker.selected.saturating_sub(visible / 2);
    let mut lines = Vec::new();
    for (index, option) in picker.options.iter().enumerate().skip(start).take(visible) {
        let marker = if index == picker.selected { "▸" } else { " " };
        let style = if index == picker.selected {
            theme::selected()
        } else {
            theme::text()
        };
        lines.push(Line::styled(format!(" {marker} {option}"), style));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_focus())
        .title(theme::heading(format!(" {} ", picker.title)));
    frame.render_widget(Paragraph::new(lines).block(block), popup);
    let hints = Rect {
        x: popup.x,
        y: popup.y + popup.height,
        width: popup.width,
        height: 1,
    };
    if hints.y < area.y + area.height {
        frame.render_widget(
            Paragraph::new(theme::hints(&[
                ("↑↓", "move"),
                ("enter", "choose"),
                ("esc", "cancel"),
            ])),
            hints,
        );
    }
}

fn value_style(kind: RowKind, value: &str, selected: bool) -> Style {
    if selected {
        return Style::default().fg(theme::SELECT_FG).bg(theme::SELECT_BG);
    }
    match kind {
        RowKind::Bool if value == "Yes" => theme::good(),
        RowKind::Bool => theme::muted(),
        RowKind::SearchOrder | RowKind::FetchOrder if value != "Off" => theme::good(),
        RowKind::SearchOrder | RowKind::FetchOrder => theme::muted(),
        RowKind::Secret => theme::faint(),
        RowKind::RawJson
        | RowKind::AddImage
        | RowKind::AddStt
        | RowKind::AddTts
        | RowKind::EditSoul
        | RowKind::EditGuidance
        | RowKind::StartDaemon
        | RowKind::StopDaemon
        | RowKind::RestartDaemon => theme::accent_style(),
        _ => theme::text(),
    }
}

fn page_start(row: usize, rows: usize, height: usize) -> usize {
    if rows <= height {
        return 0;
    }
    row.saturating_sub(height / 3)
        .min(rows.saturating_sub(height))
}

fn line_window(line: &str, cursor: usize, max_width: usize) -> (String, u16) {
    let chars: Vec<char> = line.chars().collect();
    let cursor = cursor.min(chars.len());
    let total: usize = chars.iter().map(|character| char_width(*character)).sum();
    if total <= max_width {
        return (
            line.to_owned(),
            chars[..cursor]
                .iter()
                .map(|character| char_width(*character))
                .sum::<usize>() as u16,
        );
    }
    let mut start = 0;
    let mut cursor_width = chars[..cursor]
        .iter()
        .map(|character| char_width(*character))
        .sum::<usize>();
    while start < cursor {
        let leading = usize::from(start > 0);
        if leading + cursor_width < max_width {
            break;
        }
        cursor_width = cursor_width.saturating_sub(char_width(chars[start]));
        start += 1;
    }
    let leading = usize::from(start > 0);
    let mut shown_width = leading + cursor_width;
    let mut end = cursor;
    while end < chars.len() {
        let width = char_width(chars[end]);
        if shown_width + width + 1 > max_width {
            break;
        }
        shown_width += width;
        end += 1;
    }
    let mut text = String::new();
    if leading > 0 {
        text.push('…');
    }
    text.extend(&chars[start..end]);
    if end < chars.len() {
        text.push('…');
    }
    (text, (leading + cursor_width) as u16)
}

fn char_width(character: char) -> usize {
    UnicodeWidthChar::width(character).unwrap_or(0)
}
