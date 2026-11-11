//! Last saved prompt audit.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use serde_json::Value;

use crate::app::App;
use crate::theme;
use crate::ui::short;

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(theme::heading(if app.prompt.loading {
            " Prompt audit · refreshing "
        } else {
            " Prompt audit "
        }));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.prompt.loading && app.prompt.document.is_none() {
        frame.render_widget(
            Paragraph::new(Line::styled("Loading the last prompt...", theme::warn())),
            inner,
        );
        return;
    }
    let Some(document) = &app.prompt.document else {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled("No prompt has been saved yet.", theme::muted()),
                Line::from(""),
                Line::styled(
                    "Send Hearth a Telegram message, then press o to refresh.",
                    theme::faint(),
                ),
            ])
            .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    };
    let lines = if app.prompt.raw {
        raw_lines(document)
    } else {
        system_lines(document)
    };
    let mut paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let max_scroll = paragraph
        .line_count(inner.width)
        .saturating_sub(inner.height as usize);
    app.prompt.max_scroll.set(max_scroll);
    let scroll = app.prompt.scroll.min(max_scroll).min(u16::MAX as usize) as u16;
    paragraph = paragraph.scroll((scroll, 0));
    frame.render_widget(paragraph, inner);
}

fn system_lines(document: &Value) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let model = document["model"].as_str().unwrap_or("unknown model");
    let total = token(document, "total");
    let fixed = token(document, "fixed");
    let prefix = document["prefix_hash"].as_str().unwrap_or_default();
    lines.push(Line::from(vec![
        Span::styled(" SYSTEM PROMPT", theme::accent_style()),
        Span::styled(format!("  {model}"), theme::text()),
    ]));
    lines.push(Line::from(vec![
        Span::styled(" tokens  ", theme::muted()),
        Span::styled(
            total
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            theme::text(),
        ),
        Span::styled(" total · ", theme::faint()),
        Span::styled(
            fixed
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            theme::text(),
        ),
        Span::styled(" fixed · prefix ", theme::faint()),
        Span::styled(short(prefix, 12), theme::muted()),
    ]));
    lines.push(Line::from(""));

    push_section(
        &mut lines,
        "identity",
        "Identity · SOUL.md",
        document["identity"].as_str(),
        token(document, "identity"),
    );
    push_section(
        &mut lines,
        "project_instructions",
        "Project instructions · AGENTS.md / HEARTH.md",
        document["guidance"].as_str(),
        token(document, "guidance"),
    );
    push_section(
        &mut lines,
        "standing_memory",
        "Standing memory",
        document["standing_memory"].as_str(),
        token(document, "standing"),
    );
    push_section(
        &mut lines,
        "skills",
        "Skills index",
        document["skills_index"].as_str(),
        token(document, "skills"),
    );
    push_tools(&mut lines, &document["tools"], token(document, "tools"));
    push_section(
        &mut lines,
        "relevant_memory",
        "Relevant memory for this turn",
        document["recall"].as_str(),
        token(document, "recall"),
    );
    push_transcript(
        &mut lines,
        &document["transcript"],
        token(document, "transcript"),
    );
    push_section(
        &mut lines,
        "runtime",
        "Runtime note",
        document["runtime"].as_str(),
        token(document, "runtime"),
    );
    lines
}

fn push_section(
    lines: &mut Vec<Line<'static>>,
    tag: &str,
    title: &str,
    value: Option<&str>,
    tokens: Option<u64>,
) {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return;
    };
    push_heading(lines, tag, title, tokens);
    for line in value.lines() {
        lines.push(Line::from(vec![
            Span::styled("  │ ", theme::faint()),
            Span::styled(line.to_owned(), theme::text()),
        ]));
    }
    push_close(lines, tag);
}

fn push_tools(lines: &mut Vec<Line<'static>>, tools: &Value, tokens: Option<u64>) {
    let Some(tools) = tools.as_array().filter(|tools| !tools.is_empty()) else {
        return;
    };
    push_heading(lines, "tools", "Tool definitions", tokens);
    for tool in tools {
        let name = tool["name"].as_str().unwrap_or("tool");
        let description = tool["description"].as_str().unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled("  │ ", theme::faint()),
            Span::styled("◆ ", theme::accent_style()),
            Span::styled(name.to_owned(), theme::accent_style()),
        ]));
        for line in description.lines() {
            lines.push(Line::from(vec![
                Span::styled("  │   ", theme::faint()),
                Span::styled(line.to_owned(), theme::muted()),
            ]));
        }
    }
    push_close(lines, "tools");
}

fn push_transcript(lines: &mut Vec<Line<'static>>, transcript: &Value, tokens: Option<u64>) {
    let Some(messages) = transcript
        .as_array()
        .filter(|messages| !messages.is_empty())
    else {
        return;
    };
    push_heading(lines, "transcript", "Conversation transcript", tokens);
    for (index, message) in messages.iter().enumerate() {
        if index > 0 {
            lines.push(Line::from(""));
        }
        let role = message["role"].as_str().unwrap_or("message");
        lines.push(Line::from(vec![
            Span::styled("  │ ", theme::faint()),
            Span::styled(format!("{role:<9}"), theme::accent_style()),
        ]));
        let Some(blocks) = message["blocks"].as_array() else {
            continue;
        };
        for block in blocks {
            let kind = block["type"].as_str().unwrap_or_default();
            match kind {
                "text" => push_text(lines, block["text"].as_str(), theme::text()),
                "thinking" => push_text(lines, block["text"].as_str(), theme::think()),
                "tool_call" => {
                    let name = block["name"].as_str().unwrap_or("tool");
                    lines.push(Line::from(vec![
                        Span::styled("  │   ", theme::faint()),
                        Span::styled("tool call ", theme::warn()),
                        Span::styled(name.to_owned(), theme::text()),
                    ]));
                    if let Some(arguments) = block.get("arguments") {
                        let pretty = serde_json::to_string_pretty(arguments)
                            .unwrap_or_else(|_| arguments.to_string());
                        push_text(lines, Some(&pretty), theme::muted());
                    }
                }
                "tool_result" => {
                    let failed = block["is_error"].as_bool().unwrap_or(false);
                    lines.push(Line::from(vec![
                        Span::styled("  │   ", theme::faint()),
                        Span::styled(
                            if failed { "tool error" } else { "tool result" },
                            if failed { theme::bad() } else { theme::good() },
                        ),
                    ]));
                    push_text(lines, block["content"].as_str(), theme::muted());
                }
                other => {
                    lines.push(Line::from(vec![
                        Span::styled("  │   ", theme::faint()),
                        Span::styled(format!("[{other}]"), theme::faint()),
                    ]));
                }
            }
        }
    }
    push_close(lines, "transcript");
}

fn push_text(lines: &mut Vec<Line<'static>>, value: Option<&str>, style: Style) {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return;
    };
    for line in value.lines() {
        lines.push(Line::from(vec![
            Span::styled("  │     ", theme::faint()),
            Span::styled(line.to_owned(), style),
        ]));
    }
}

fn push_heading(lines: &mut Vec<Line<'static>>, tag: &str, title: &str, tokens: Option<u64>) {
    lines.push(Line::from(vec![
        Span::styled(format!(" <{tag}>"), theme::accent_style()),
        Span::styled(format!("  {title}"), theme::muted()),
        Span::styled(
            tokens
                .map(|value| format!("  ~{value} tok"))
                .unwrap_or_default(),
            theme::faint(),
        ),
    ]));
}

fn push_close(lines: &mut Vec<Line<'static>>, tag: &str) {
    lines.push(Line::styled(format!(" </{tag}>"), theme::faint()));
    lines.push(Line::from(""));
}

fn token(document: &Value, name: &str) -> Option<u64> {
    document["tokens"][name].as_u64()
}

fn raw_lines(document: &Value) -> Vec<Line<'static>> {
    let text = serde_json::to_string_pretty(document).unwrap_or_else(|_| document.to_string());
    text.lines()
        .map(|line| Line::styled(line.to_owned(), theme::text()))
        .collect()
}
