//! Full-screen text editor for files and settings.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::editor::OpenEditor;
use crate::theme;

pub fn render(editor: &OpenEditor, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_focus())
        .title(theme::heading(format!(" Edit · {} ", editor.text.title)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(inner);
    let cursor = text(editor, frame, rows[0]);
    if let Some(cursor) = cursor {
        frame.set_cursor_position(cursor);
    }
    status(editor, frame, rows[1]);
}

fn text(editor: &OpenEditor, frame: &mut Frame, area: Rect) -> Option<(u16, u16)> {
    let height = area.height.max(1) as usize;
    let digits = editor.text.lines.len().to_string().len().max(4);
    let gutter = digits + 2;
    let text_width = area.width.saturating_sub(gutter as u16) as usize;
    let start = page_start(editor.text.row, editor.text.lines.len(), height);
    let mut lines = Vec::new();
    let mut cursor = None;
    for row in start..(start + height).min(editor.text.lines.len()) {
        let current = row == editor.text.row;
        let text = editor.text.lines.get(row).map(String::as_str).unwrap_or("");
        let (shown, cursor_col) = if current {
            line_window(text, editor.text.col, text_width)
        } else {
            line_window(text, 0, text_width)
        };
        let number_style = if current {
            theme::accent_style()
        } else {
            theme::faint()
        };
        let text_style = if current {
            Style::default().fg(theme::TEXT).bg(theme::FIELD_BG)
        } else {
            theme::text()
        };
        let mut spans = vec![
            Span::styled(format!("{:>digits$} ", row + 1), number_style),
            Span::styled(format!("{shown:<text_width$}"), text_style),
        ];
        if current {
            cursor = Some((
                area.x + gutter as u16 + cursor_col,
                area.y + (row - start) as u16,
            ));
        }
        lines.push(Line::from(std::mem::take(&mut spans)));
    }
    frame.render_widget(Paragraph::new(lines), area);
    cursor
}

fn status(editor: &OpenEditor, frame: &mut Frame, area: Rect) {
    let state = if editor.text.dirty {
        Span::styled("modified", theme::warn())
    } else {
        Span::styled("saved", theme::good())
    };
    let first = Line::from(vec![
        Span::styled(" ", theme::faint()),
        Span::styled(editor.text.path.display().to_string(), theme::muted()),
        Span::styled("  ·  ", theme::faint()),
        Span::styled(
            format!("line {}:{}", editor.text.row + 1, editor.text.col + 1),
            theme::text(),
        ),
        Span::styled("  ·  ", theme::faint()),
        state,
    ]);
    let second = if let Some(error) = &editor.text.error {
        Line::styled(format!(" {error}"), theme::bad())
    } else if let Some(notice) = &editor.text.notice {
        Line::styled(format!(" {notice}"), theme::good())
    } else {
        theme::hints(&[
            ("ctrl+s", "save"),
            ("esc", "close"),
            ("tab", "indent"),
            ("↑↓←→", "move"),
        ])
    };
    frame.render_widget(Paragraph::new(vec![first, second]), area);
}

fn page_start(row: usize, lines: usize, height: usize) -> usize {
    if lines <= height {
        return 0;
    }
    row.saturating_sub(height / 3)
        .min(lines.saturating_sub(height))
}

fn line_window(line: &str, cursor: usize, max_width: usize) -> (String, u16) {
    let chars: Vec<char> = line.chars().collect();
    let cursor = cursor.min(chars.len());
    let total = width_of(&chars);
    if total <= max_width {
        return (line.to_owned(), width_of(&chars[..cursor]) as u16);
    }
    let mut start = 0;
    let mut cursor_width = width_of(&chars[..cursor]);
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

fn width_of(chars: &[char]) -> usize {
    chars.iter().map(|character| char_width(*character)).sum()
}

fn char_width(character: char) -> usize {
    UnicodeWidthChar::width(character).unwrap_or(0)
}

#[allow(dead_code)]
fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}
