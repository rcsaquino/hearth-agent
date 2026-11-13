//! Prompt and response audit: everything sent to the model and everything back.
//!
//! One card, one column. Every request segment and every response block is a
//! stacked section. Sections collapse to a couple of lines with a trailing
//! ellipsis; `enter` expands the selected one and `a` toggles all of them.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use serde_json::Value;
use time::{OffsetDateTime, UtcOffset};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, PromptView};
use crate::theme;
use crate::ui::{count, short};

/// Longest tool result shown in audit mode; raw mode shows everything.
const RESULT_CLIP: usize = 600;
/// Body lines shown for a collapsed section before the ellipsis.
const COLLAPSED_LINES: usize = 2;

pub(crate) fn select_previous(prompt: &mut PromptView) {
    prompt.selected = prompt.selected.saturating_sub(1);
    prompt.scroll = 0;
}

pub(crate) fn select_first(prompt: &mut PromptView) {
    prompt.selected = 0;
    prompt.scroll = 0;
}

pub(crate) fn select_last(prompt: &mut PromptView) {
    let rows = section_count(prompt);
    prompt.selected = rows.saturating_sub(1);
    prompt.scroll = 0;
}

pub(crate) fn select_next(prompt: &mut PromptView) {
    let rows = section_count(prompt);
    prompt.selected = (prompt.selected + 1).min(rows.saturating_sub(1));
    prompt.scroll = 0;
}

fn section_count(prompt: &PromptView) -> usize {
    prompt
        .cache
        .borrow()
        .as_ref()
        .map(|cache| cache.sections.len())
        .unwrap_or(1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheKey {
    prompt_hash: String,
    requested_at: i64,
    response_id: String,
    selected: usize,
    expanded: Vec<usize>,
    raw: bool,
    width: u16,
}

impl CacheKey {
    fn new(app: &App, exchange: &Value, width: u16) -> Self {
        Self {
            prompt_hash: exchange["prompt_hash"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            requested_at: exchange["requested_at"].as_i64().unwrap_or(0),
            response_id: exchange["response"]["entry_id"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            selected: app.prompt.selected,
            expanded: app.prompt.expanded.iter().copied().collect(),
            raw: app.prompt.raw,
            width,
        }
    }
}

/// One stacked section inside the prompt card.
pub(crate) struct Section {
    /// First rendered line of this section.
    pub(crate) start: usize,
}

pub(crate) struct PromptCache {
    key: CacheKey,
    pub(crate) sections: Vec<Section>,
    lines: Vec<Line<'static>>,
}

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let card = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(theme::heading(if app.prompt.loading {
            " Prompt · refreshing "
        } else if app.prompt.raw {
            " Prompt · raw "
        } else {
            " Prompt "
        }));
    let inner = card.inner(area);
    frame.render_widget(card, area);

    if app.prompt.loading && app.prompt.exchanges.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled("Loading saved prompts...", theme::warn())),
            inner,
        );
        return;
    }
    let Some(exchange) = app.prompt.current() else {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled("No prompts yet.", theme::muted()),
                Line::from(""),
                Line::styled(
                    "Send Hearth a Telegram message, then press o to refresh.",
                    theme::faint(),
                ),
            ]),
            inner,
        );
        return;
    };

    let key = CacheKey::new(app, exchange, inner.width);
    let mut slot = app.prompt.cache.borrow_mut();
    if slot.as_ref().is_none_or(|cache| cache.key != key) {
        *slot = Some(build_cache(app, exchange, &key, usize::from(inner.width)));
    }
    let cache = slot.as_mut().expect("prompt cache");

    let count = cache.sections.len();
    let selected = app.prompt.selected.min(count.saturating_sub(1));
    let height = inner.height.max(1) as usize;
    let total = cache.lines.len();
    let max_scroll = total.saturating_sub(height);
    app.prompt.max_scroll.set(max_scroll);
    let anchor = cache
        .sections
        .get(selected)
        .map(|section| section.start)
        .unwrap_or(0);
    let scroll = (anchor + app.prompt.scroll).min(max_scroll);
    let end = (scroll + height).min(total);
    frame.render_widget(Paragraph::new(cache.lines[scroll..end].to_vec()), inner);
}

fn build_cache(app: &App, exchange: &Value, key: &CacheKey, width: usize) -> PromptCache {
    let own = exchange["request"]["prefix_hash"].as_str();
    let peers = app
        .prompt
        .exchanges
        .iter()
        .filter(|other| other["request"]["prefix_hash"].as_str() == own)
        .count();
    let view = View {
        exchange,
        older: app.prompt.older(),
        total: app.prompt.exchanges.len(),
        peers: if own.is_some() { peers } else { 0 },
        offset: app.local_offset,
        width,
    };

    let mut lines = Vec::new();
    push_header(&mut lines, &view);

    let rows = if app.prompt.raw {
        raw_rows(exchange)
    } else {
        build_rows(&view)
    };

    let body_width = width.saturating_sub(3).max(8);
    let mut sections = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            lines.push(Line::from(""));
        }
        let start = lines.len();
        let expanded = app.prompt.expanded.contains(&index);
        let body = if row.content.is_empty() {
            vec![("(empty)".to_owned(), theme::faint())]
        } else {
            row.content.clone()
        };
        let mut wrapped = Vec::new();
        for (text, style) in &body {
            for line in wrap_text(text, body_width) {
                wrapped.push((line, *style));
            }
        }
        let limit = if expanded {
            wrapped.len()
        } else {
            COLLAPSED_LINES.min(wrapped.len())
        };
        let hidden = wrapped.len().saturating_sub(limit);

        let mut heading = Row::new();
        heading.push(" ▌ ", row.label_style);
        heading.push(
            row.label.clone(),
            row.label_style.add_modifier(Modifier::BOLD),
        );
        if hidden > 0 {
            heading.push(format!("  {hidden} more · enter"), theme::faint());
        }
        lines.push(heading.line());

        for (line_index, (text, style)) in wrapped.iter().take(limit).enumerate() {
            let mut text = text.clone();
            if hidden > 0 && line_index + 1 == limit {
                text.push('…');
            }
            lines.push(Line::styled(format!("   {text}"), *style));
        }
        sections.push(Section { start });
    }

    PromptCache {
        key: key.clone(),
        sections,
        lines,
    }
}

fn raw_rows(exchange: &Value) -> Vec<PromptRow> {
    let text = serde_json::to_string_pretty(exchange).unwrap_or_else(|_| exchange.to_string());
    vec![PromptRow {
        label: "raw request".to_owned(),
        label_style: theme::text(),
        content: text
            .lines()
            .map(|line| (line.to_owned(), theme::text()))
            .collect(),
    }]
}

/// Everything the renderer needs about the exchange on screen.
struct View<'a> {
    exchange: &'a Value,
    /// The next-older exchange, used for change markers.
    older: Option<&'a Value>,
    total: usize,
    /// Loaded exchanges sharing this request's prefix hash.
    peers: usize,
    offset: UtcOffset,
    width: usize,
}

impl View<'_> {
    fn request(&self) -> &Value {
        &self.exchange["request"]
    }

    /// True when this prefix part differs from the older exchange.
    fn changed(&self, part: Option<&str>) -> bool {
        let Some(part) = part else {
            return false;
        };
        let Some(older) = self.older else {
            return false;
        };
        let current = self.request()["prefix_parts"]
            .get(part)
            .and_then(Value::as_str);
        let previous = older["request"]["prefix_parts"]
            .get(part)
            .and_then(Value::as_str);
        current != previous
    }
}

fn push_header(lines: &mut Vec<Line<'static>>, view: &View<'_>) {
    let request = view.request();
    let model = request["model"].as_str().unwrap_or("unknown model");
    let mut row = Row::new();
    row.push(" PROMPT", theme::accent_style());
    row.push(format!("  {model}"), theme::text());
    let position = format!(" {} loaded ", view.total);
    row.pad_to(view.width.saturating_sub(position.width()));
    row.push(position, theme::muted());
    lines.push(row.line());

    let requested = view.exchange["requested_at"].as_i64().unwrap_or(0);
    let hash = view.exchange["prompt_hash"].as_str().unwrap_or("-");
    let prefix = request["prefix_hash"].as_str().unwrap_or("-");
    let mut row = Row::new();
    row.push(" ", theme::faint());
    row.push(stamp(requested, view.offset), theme::muted());
    row.push("  ·  prompt ", theme::faint());
    row.push(short(hash, 12), theme::text());
    row.push("  ·  prefix ", theme::faint());
    row.push(short(prefix, 12), theme::text());
    lines.push(row.line());

    let total = token(request, "total");
    let fixed = token(request, "fixed");
    let mut row = Row::new();
    row.push(" tokens  ", theme::muted());
    row.push(
        total.map(count).unwrap_or_else(|| "-".to_owned()),
        theme::text(),
    );
    row.push(" total · ", theme::faint());
    row.push(
        fixed.map(count).unwrap_or_else(|| "-".to_owned()),
        theme::text(),
    );
    row.push(" fixed · ", theme::faint());
    let fixed_share = if let (Some(fixed), Some(total)) = (fixed, total)
        && total > 0
    {
        format!("{:.1}% of the prompt", fixed as f64 / total as f64 * 100.0)
    } else {
        "-".to_owned()
    };
    row.push(fixed_share, theme::faint());
    lines.push(row.line());

    let mut row = Row::new();
    row.push(" cache   ", theme::muted());
    if view.peers > 1 {
        row.push(
            format!(
                "{} of {} loaded exchanges share this prefix",
                view.peers, view.total
            ),
            theme::good(),
        );
    } else {
        row.push(
            "no other loaded exchange shares this prefix".to_owned(),
            theme::faint(),
        );
    }
    lines.push(row.line());

    if has_markers(view) {
        let mut row = Row::new();
        row.push(" legend  ", theme::muted());
        row.push("▲ ", theme::warn());
        row.push(
            "prefix part changed versus the previous loaded prompt".to_owned(),
            theme::faint(),
        );
        lines.push(row.line());
    }
    lines.push(Line::from(""));
}

fn has_markers(view: &View<'_>) -> bool {
    ["identity", "guidance", "standing", "skills", "tools"]
        .iter()
        .any(|part| view.changed(Some(part)))
}

struct PromptRow {
    label: String,
    label_style: Style,
    content: Vec<(String, Style)>,
}

impl PromptRow {
    fn text(label: &str, style: Style, text: &str) -> Self {
        let empty = text.trim().is_empty();
        let content = if empty {
            vec![("(empty)".to_owned(), theme::faint())]
        } else {
            text.lines()
                .map(|line| (line.to_owned(), style))
                .collect::<Vec<_>>()
        };
        Self {
            label: label.to_owned(),
            label_style: style,
            content,
        }
    }

    fn tool_call(prefix: &str, call: &Value) -> Self {
        let name = call["name"].as_str().unwrap_or("tool");
        let content = call
            .get("arguments")
            .map(arguments_text)
            .unwrap_or_default();
        let label = if prefix == "assistant" {
            format!("tool call · {name}")
        } else {
            format!("{prefix} · tool · {name}")
        };
        Self {
            label,
            label_style: theme::warn(),
            content: vec![(content, theme::muted())],
        }
    }

    fn tool_result(prefix: &str, result: &Value) -> Self {
        let failed = result["is_error"].as_bool().unwrap_or(false);
        let content = clipped(result["content"].as_str().unwrap_or_default(), RESULT_CLIP);
        let style = if failed { theme::bad() } else { theme::good() };
        let label = if prefix == "tool" {
            "tool · result".to_owned()
        } else {
            format!("{prefix} · result")
        };
        Self {
            label,
            label_style: style,
            content: content
                .lines()
                .map(|line| (line.to_owned(), style))
                .collect(),
        }
    }
}

/// Every request segment and response block, in wire order.
fn build_rows(view: &View<'_>) -> Vec<PromptRow> {
    let mut rows = Vec::new();
    for (label, part, key, style) in [
        (
            "system · identity",
            "identity",
            "identity",
            theme::accent_style(),
        ),
        (
            "system · guidance",
            "guidance",
            "guidance",
            theme::accent_style(),
        ),
        (
            "system · standing memory",
            "standing",
            "standing_memory",
            theme::accent_style(),
        ),
        (
            "system · skills index",
            "skills",
            "skills_index",
            theme::accent_style(),
        ),
    ] {
        let text = view.request()[key].as_str().unwrap_or_default();
        rows.push(PromptRow::text(
            &row_label(view, label, Some(part)),
            style,
            text,
        ));
    }

    let messages = view.request()["transcript"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let newest_user = messages
        .iter()
        .rposition(|message| message["role"].as_str() == Some("user"));
    for (index, message) in messages.iter().enumerate() {
        if Some(index) == newest_user {
            let recall = view.request()["recall"].as_str().unwrap_or_default();
            if !recall.trim().is_empty() {
                rows.push(PromptRow::text(
                    "system · relevant memory",
                    theme::good(),
                    recall,
                ));
            }
        }
        match message["role"].as_str().unwrap_or_default() {
            "user" => {
                let text = message_text(message);
                rows.push(PromptRow::text("user", theme::accent_style(), &text));
            }
            "assistant" => {
                if let Some(blocks) = message["blocks"].as_array() {
                    for block in blocks {
                        match block["type"].as_str().unwrap_or_default() {
                            "thinking" => rows.push(PromptRow::text(
                                "assistant · thinking",
                                theme::think(),
                                block["text"].as_str().unwrap_or_default(),
                            )),
                            "text" => rows.push(PromptRow::text(
                                "assistant",
                                theme::text(),
                                block["text"].as_str().unwrap_or_default(),
                            )),
                            "tool_call" => rows.push(PromptRow::tool_call("assistant", block)),
                            "file" => rows.push(PromptRow::text(
                                "assistant · file",
                                theme::faint(),
                                &format!(
                                    "{} · {}",
                                    block["name"].as_str().unwrap_or("file"),
                                    block["path"].as_str().unwrap_or("-")
                                ),
                            )),
                            "image" => rows.push(PromptRow::text(
                                "assistant · image",
                                theme::faint(),
                                block["path"].as_str().unwrap_or("-"),
                            )),
                            _ => {}
                        }
                    }
                }
            }
            "tool" => {
                if let Some(blocks) = message["blocks"].as_array() {
                    for block in blocks {
                        if block["type"].as_str() == Some("tool_result") {
                            rows.push(PromptRow::tool_result("tool", block));
                        }
                    }
                }
            }
            "system" => rows.push(PromptRow::text(
                "system",
                theme::faint(),
                &message_text(message),
            )),
            _ => {}
        }
    }

    rows.push(PromptRow::text(
        "system · runtime note",
        theme::muted(),
        view.request()["runtime"].as_str().unwrap_or_default(),
    ));

    let tools = view.request()["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut tool_content = Vec::new();
    for (index, tool) in tools.iter().enumerate() {
        let name = tool["name"].as_str().unwrap_or("tool");
        tool_content.push((name.to_owned(), theme::accent_style()));
        let description = tool["description"].as_str().unwrap_or_default();
        for line in description.lines() {
            tool_content.push((line.to_owned(), theme::muted()));
        }
        if let Some(parameters) = tool.get("parameters").filter(|value| !value.is_null()) {
            let fields = schema_fields(parameters);
            if !fields.is_empty() {
                tool_content.push(("args".to_owned(), theme::muted()));
                for field in fields {
                    tool_content.push((format!("  {field}"), theme::faint()));
                }
            }
        }
        if index + 1 < tools.len() {
            tool_content.push((String::new(), theme::faint()));
        }
    }
    rows.push(PromptRow {
        label: row_label(view, "request · tools", Some("tools")),
        label_style: theme::accent_style(),
        content: tool_content,
    });

    let response = &view.exchange["response"];
    if response.is_null() {
        rows.push(PromptRow::text(
            "response",
            theme::faint(),
            "no saved assistant entry for this prompt",
        ));
    } else if let Some(blocks) = response["blocks"].as_array() {
        for block in blocks {
            match block["type"].as_str().unwrap_or_default() {
                "thinking" => rows.push(PromptRow::text(
                    "response · thinking",
                    theme::think(),
                    block["text"].as_str().unwrap_or_default(),
                )),
                "text" => rows.push(PromptRow::text(
                    "response",
                    theme::text(),
                    block["text"].as_str().unwrap_or_default(),
                )),
                "tool_call" => rows.push(PromptRow::tool_call("response", block)),
                "tool_result" => rows.push(PromptRow::tool_result("response", block)),
                "file" => rows.push(PromptRow::text(
                    "response · file",
                    theme::faint(),
                    &format!(
                        "{} · {}",
                        block["name"].as_str().unwrap_or("file"),
                        block["path"].as_str().unwrap_or("-")
                    ),
                )),
                "image" => rows.push(PromptRow::text(
                    "response · image",
                    theme::faint(),
                    block["path"].as_str().unwrap_or("-"),
                )),
                _ => {}
            }
        }
    }
    rows.push(PromptRow {
        label: "response · usage".to_owned(),
        label_style: theme::faint(),
        content: vec![(usage_line(&view.exchange["usage"]), theme::faint())],
    });

    rows
}

fn row_label(view: &View<'_>, label: &str, part: Option<&str>) -> String {
    if view.changed(part) {
        format!("▲ {label}")
    } else {
        label.to_owned()
    }
}

fn schema_fields(parameters: &Value) -> Vec<String> {
    let Some(properties) = parameters.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let required: Vec<&str> = parameters
        .get("required")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut fields = Vec::new();
    for (name, schema) in properties {
        let mut field = format!("{name}: {}", schema_type(schema));
        if let Some(values) = schema.get("enum").and_then(Value::as_array) {
            let values: Vec<&str> = values.iter().filter_map(Value::as_str).collect();
            if !values.is_empty() {
                field.push_str(&format!(" ({})", values.join(" | ")));
            }
        }
        if required.contains(&name.as_str()) {
            field.push_str(" (required)");
        }
        fields.push(field);
    }
    fields
}

fn schema_type(schema: &Value) -> String {
    schema["type"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            schema["type"].as_array().map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
        })
        .unwrap_or_else(|| "any".to_owned())
}

fn usage_line(usage: &Value) -> String {
    if usage.is_null() {
        return "usage · not linked to this prompt".to_owned();
    }
    let input = usage["input_tokens"].as_u64().unwrap_or(0);
    let output = usage["output_tokens"].as_u64().unwrap_or(0);
    let reasoning = usage["reasoning_tokens"].as_u64().unwrap_or(0);
    let cache = usage["cache_read_tokens"].as_u64().unwrap_or(0);
    let latency = usage["latency_ms"].as_u64().unwrap_or(0);
    format!(
        "usage · in {} · out {} · cache read {} · thinking {} · {}ms",
        count(input),
        count(output),
        count(cache),
        count(reasoning),
        count(latency)
    )
}

fn arguments_text(arguments: &Value) -> String {
    serde_json::to_string_pretty(arguments).unwrap_or_else(|_| arguments.to_string())
}

fn message_text(message: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(text) = message["text"].as_str()
        && !text.is_empty()
    {
        parts.push(text.to_owned());
    }
    if let Some(blocks) = message["blocks"].as_array() {
        for block in blocks {
            if block["type"].as_str() == Some("text")
                && let Some(text) = block["text"].as_str()
                && !text.is_empty()
            {
                parts.push(text.to_owned());
            }
        }
    }
    parts.join("\n")
}

struct Row {
    spans: Vec<Span<'static>>,
    width: usize,
}

impl Row {
    fn new() -> Self {
        Self {
            spans: Vec::new(),
            width: 0,
        }
    }

    fn push(&mut self, text: impl Into<String>, style: Style) {
        let span = Span::styled(text.into(), style);
        self.width += span.content.width();
        self.spans.push(span);
    }

    fn pad_to(&mut self, column: usize) {
        let pad = column.saturating_sub(self.width);
        if pad > 0 {
            self.spans.push(Span::raw(" ".repeat(pad)));
            self.width += pad;
        }
    }

    fn line(self) -> Line<'static> {
        Line::from(self.spans)
    }
}

fn token(document: &Value, name: &str) -> Option<u64> {
    document["tokens"][name].as_u64()
}

fn stamp(micros: i64, offset: UtcOffset) -> String {
    let time = OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .to_offset(offset);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        time.year(),
        u8::from(time.month()),
        time.day(),
        time.hour(),
        time.minute(),
        time.second()
    )
}

/// Shorten display-only text. Raw mode never goes through here.
fn clipped(text: &str, max: usize) -> String {
    let total = text.chars().count();
    if total <= max {
        return text.to_owned();
    }
    let head = text.chars().take(max).collect::<String>();
    format!("{head}… ({} more chars, r for raw)", total - max)
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines = Vec::new();
    for raw in text.lines() {
        if raw.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in raw.split_whitespace() {
            if line.is_empty() {
                line.push_str(word);
            } else if line.width() + 1 + word.width() <= width {
                line.push(' ');
                line.push_str(word);
            } else {
                lines.push(std::mem::take(&mut line));
                line.push_str(word);
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample(identity: &str) -> Value {
        json!({
            "request": {
                "model": "test-model",
                "identity": identity,
                "guidance": "PROJECT RULES",
                "standing_memory": "STANDING LINES",
                "skills_index": "skill-one: does things",
                "tools": [
                    {
                        "name": "read_file",
                        "description": "Read a file from the workspace.",
                        "parameters": {
                            "type": "object",
                            "properties": {"path": {"type": "string"}},
                            "required": ["path"]
                        }
                    },
                    {
                        "name": "write_file",
                        "description": "Write a file.",
                        "parameters": {
                            "type": "object",
                            "properties": {"content": {"type": "string"}}
                        }
                    }
                ],
                "recall": "MEMORY HIT",
                "transcript": [
                    {"role": "user", "blocks": [{"type": "text", "text": "please inspect"}]},
                    {
                        "role": "assistant",
                        "blocks": [
                            {"type": "thinking", "text": "weighing"},
                            {"type": "tool_call", "name": "read_file", "arguments": {"path": "Cargo.toml"}}
                        ]
                    }
                ],
                "runtime": "RUNTIME NOTE",
                "prefix_hash": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                "prefix_parts": {
                    "identity": format!("hash({identity})"),
                    "guidance": "g1",
                    "standing": "s1",
                    "skills": "k1",
                    "tools": "t1"
                },
                "tokens": {
                    "identity": 10,
                    "guidance": 20,
                    "standing": 30,
                    "skills": 40,
                    "tools": 50,
                    "transcript": 60,
                    "recall": 70,
                    "runtime": 80,
                    "total": 360,
                    "fixed": 150
                }
            },
            "prompt_hash": "abc123def4567890",
            "requested_at": 1_700_000_000_000_000_i64,
            "response": {
                "entry_id": "entry-1",
                "created_at": 1_700_000_005_000_000_i64,
                "blocks": [
                    {"type": "thinking", "text": "weighing options"},
                    {"type": "text", "text": "here is the answer"},
                    {"type": "tool_call", "id": "c1", "name": "lookup", "arguments": {"model": "x"}},
                    {"type": "tool_result", "is_error": false, "content": "ok"}
                ],
                "tokens_est": 42
            },
            "usage": {
                "input_tokens": 1000,
                "output_tokens": 200,
                "reasoning_tokens": 50,
                "cache_read_tokens": 900,
                "latency_ms": 1200
            }
        })
    }

    fn render_audit(exchange: &Value, older: Option<&Value>) -> String {
        let view = View {
            exchange,
            older,
            total: 2,
            peers: 2,
            offset: UtcOffset::UTC,
            width: 110,
        };
        let mut text = String::new();
        let mut header = Vec::new();
        push_header(&mut header, &view);
        for line in header {
            text.push_str(
                &line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>(),
            );
            text.push('\n');
        }
        for row in build_rows(&view).iter() {
            text.push_str(&row.label);
            text.push('\n');
            for (line, _) in row.content.clone() {
                text.push_str(&line);
                text.push('\n');
            }
        }
        text
    }

    #[test]
    fn the_audit_shows_every_request_segment_and_the_response() {
        let exchange = sample("SOUL TEXT");
        let text = render_audit(&exchange, None);
        let mut last = 0;
        for label in [
            "system · identity",
            "system · guidance",
            "system · standing memory",
            "system · skills index",
            "system · relevant memory",
            "user",
            "assistant · thinking",
            "tool call · read_file",
            "system · runtime note",
            "request · tools",
            "response · thinking",
            "response",
            "response · tool · lookup",
            "response · usage",
        ] {
            let position = text
                .find(label)
                .unwrap_or_else(|| panic!("missing {label} in {text}"));
            assert!(position >= last, "{label} is out of wire order in {text}");
            last = position;
        }
        assert!(
            !text.contains("<identity>") && !text.contains("</identity>"),
            "the audit should not show raw XML tags: {text}"
        );
        assert!(
            !text.contains("</standing_memory>"),
            "the audit should not show closing tags: {text}"
        );
        assert!(text.contains("SOUL TEXT"), "{text}");
        assert!(text.contains("PROJECT RULES"), "{text}");
        assert!(text.contains("RUNTIME NOTE"), "{text}");
        assert!(
            text.contains("please inspect"),
            "newest user message: {text}"
        );
    }

    #[test]
    fn tool_parameters_and_the_response_blocks_are_built_out() {
        let exchange = sample("SOUL TEXT");
        let text = render_audit(&exchange, None);
        assert!(text.contains("read_file"), "{text}");
        assert!(text.contains("args"), "{text}");
        assert!(text.contains("path: string (required)"), "{text}");
        assert!(
            !text.contains("\"properties\""),
            "raw schema JSON should stay in raw mode: {text}"
        );
        assert!(text.contains("thinking"), "{text}");
        assert!(text.contains("here is the answer"), "{text}");
        assert!(text.contains("response · tool · lookup"), "{text}");
        assert!(text.contains("usage · in 1,000"), "{text}");
        assert!(text.contains("cache read 900"), "{text}");
    }

    #[test]
    fn relevant_memory_sits_just_before_the_newest_user_message() {
        let exchange = sample("SOUL TEXT");
        let text = render_audit(&exchange, None);
        let memory = text
            .find("system · relevant memory")
            .expect("relevant memory");
        let newest_user = text.find("please inspect").expect("newest user");
        assert!(
            memory < newest_user,
            "relevant memory must sit before the newest user: {text}"
        );
    }

    #[test]
    fn the_runtime_note_follows_the_conversation() {
        let exchange = sample("SOUL TEXT");
        let text = render_audit(&exchange, None);
        let conversation = text.find("please inspect").expect("conversation");
        let runtime = text.find("RUNTIME NOTE").expect("runtime note");
        let response = text.find("request · tools").expect("tools");
        assert!(
            conversation < runtime && runtime < response,
            "runtime note must sit after the transcript and before request tools: {text}"
        );
    }

    #[test]
    fn a_changed_prefix_part_is_marked_against_the_older_exchange() {
        let newest = sample("NEW SOUL");
        let older = sample("OLD SOUL");
        let marked = render_audit(&newest, Some(&older));
        assert!(marked.contains('▲'), "expected a change marker: {marked}");
        assert!(marked.contains("prefix part changed"), "{marked}");

        let first = render_audit(&newest, None);
        assert!(!first.contains('▲'), "no older exchange means no markers");
        assert!(!first.contains("prefix part changed"), "{first}");
    }

    #[test]
    fn a_missing_usage_link_says_so_instead_of_zeroes() {
        let mut exchange = sample("SOUL TEXT");
        exchange["usage"] = json!(null);
        let text = render_audit(&exchange, None);
        assert!(text.contains("usage · not linked"), "{text}");
        assert!(!text.contains("cache read 0"), "{text}");
    }

    #[test]
    fn a_missing_response_is_reported_without_panicking() {
        let mut exchange = sample("SOUL TEXT");
        exchange["response"] = json!(null);
        let text = render_audit(&exchange, None);
        assert!(text.contains("response"), "{text}");
        assert!(text.contains("no saved assistant entry"), "{text}");
    }

    #[test]
    fn a_collapsed_section_ends_with_an_ellipsis_and_expands() {
        let body = (0..20)
            .map(|index| format!("body line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let row = PromptRow::text("demo", theme::text(), &body);
        let wrapped = wrap_text(&body, 40);
        assert!(wrapped.len() > COLLAPSED_LINES);
        let collapsed = wrapped.len().saturating_sub(COLLAPSED_LINES);
        assert_eq!(
            collapsed,
            row.content.len().checked_sub(COLLAPSED_LINES).unwrap()
        );
        let limit = COLLAPSED_LINES.min(row.content.len());
        assert_eq!(limit, 2);
    }

    #[test]
    fn narrow_widths_clamp_instead_of_underflowing() {
        for width in [0usize, 1, 12, 40, 75, 76, 200] {
            let exchange = sample("SOUL TEXT");
            let view = View {
                exchange: &exchange,
                older: None,
                total: 9,
                peers: 1,
                offset: UtcOffset::UTC,
                width,
            };
            let rows = build_rows(&view);
            assert!(!rows.is_empty(), "width {width} produced no rows");
            for row in &rows {
                let wrapped = wrap_text(
                    &row.content
                        .first()
                        .map(|(text, _)| text.clone())
                        .unwrap_or_default(),
                    width.saturating_sub(3),
                );
                assert!(!wrapped.is_empty(), "width {width} produced no lines");
            }
        }
    }
}
