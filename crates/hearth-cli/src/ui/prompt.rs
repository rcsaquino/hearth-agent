//! Live session export: everything sent to the model and everything back.
//!
//! One scrollable document. Every request segment, tool declaration, transcript
//! item, and response block is shown in full. The newest saved exchange is shown
//! by default; `[` and `]` walk older snapshots and `f` resumes the live tail.

use hearth_runtime::Zone;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, PromptView};
use crate::theme;
use crate::ui::{count, short};

/// Body lines move by this many rows for PageUp and PageDown.
const PAGE_LINES: usize = 15;
/// Body lines shown under a section heading.
const BODY_INDENT: &str = "   ";

/// Scroll up by `lines`, pausing the live tail.
pub(crate) fn scroll_up(prompt: &mut PromptView, lines: usize) {
    let current = if prompt.follow {
        prompt.max_scroll.get()
    } else {
        prompt.scroll
    };
    prompt.follow = false;
    prompt.scroll = current.saturating_sub(lines);
    sync_selected(prompt);
}

/// Scroll down by `lines`, resuming the live tail at the bottom.
pub(crate) fn scroll_down(prompt: &mut PromptView, lines: usize) {
    let max = prompt.max_scroll.get();
    let current = if prompt.follow { max } else { prompt.scroll };
    prompt.scroll = (current + lines).min(max);
    prompt.follow = prompt.scroll >= max;
    sync_selected(prompt);
}

pub(crate) fn page_up(prompt: &mut PromptView) {
    scroll_up(prompt, PAGE_LINES);
}

pub(crate) fn page_down(prompt: &mut PromptView) {
    scroll_down(prompt, PAGE_LINES);
}

pub(crate) fn scroll_top(prompt: &mut PromptView) {
    prompt.follow = false;
    prompt.scroll = 0;
    prompt.selected = 0;
}

pub(crate) fn scroll_bottom(prompt: &mut PromptView) {
    prompt.follow = true;
    prompt.scroll = prompt.max_scroll.get();
    prompt.selected = section_count(prompt).saturating_sub(1);
}

pub(crate) fn toggle_follow(prompt: &mut PromptView) {
    prompt.follow = !prompt.follow;
    if prompt.follow {
        prompt.scroll = prompt.max_scroll.get();
    }
}

/// Show the next-older saved snapshot.
pub(crate) fn older(prompt: &mut PromptView) -> bool {
    if prompt.cursor + 1 >= prompt.exchanges.len() {
        return false;
    }
    prompt.cursor += 1;
    prompt.follow = false;
    prompt.scroll = 0;
    prompt.selected = 0;
    true
}

/// Show the next-newer saved snapshot. The newest snapshot resumes live tail.
pub(crate) fn newer(prompt: &mut PromptView) -> bool {
    if prompt.cursor == 0 {
        return false;
    }
    prompt.cursor -= 1;
    prompt.follow = prompt.cursor == 0;
    prompt.scroll = if prompt.follow {
        prompt.max_scroll.get()
    } else {
        0
    };
    prompt.selected = 0;
    true
}

fn sync_selected(prompt: &mut PromptView) {
    let index = prompt
        .cache
        .borrow()
        .as_ref()
        .map(|cache| {
            cache
                .sections
                .iter()
                .rposition(|section| section.start <= prompt.scroll)
                .unwrap_or(0)
        })
        .unwrap_or(0);
    prompt.selected = index;
}

fn section_count(prompt: &PromptView) -> usize {
    prompt
        .cache
        .borrow()
        .as_ref()
        .map(|cache| cache.sections.len())
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheKey {
    prompt_hash: String,
    requested_at: i64,
    response_id: String,
    raw: bool,
    width: u16,
    zone: String,
}

impl CacheKey {
    fn new(exchange: &Value, width: u16, raw: bool, zone: &Zone) -> Self {
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
            raw,
            width,
            zone: zone.name().to_owned(),
        }
    }
}

/// One section inside the session document.
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
    let title = if app.prompt.loading {
        " Prompt · refreshing "
    } else if app.prompt.raw {
        " Prompt · raw "
    } else if app.prompt.follow {
        " Prompt · live "
    } else {
        " Prompt · paused "
    };
    let card = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(theme::heading(title));
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
                Line::styled("No session yet.", theme::muted()),
                Line::from(""),
                Line::styled(
                    "Send Hearth a Telegram message. This view refreshes after every reply.",
                    theme::faint(),
                ),
            ]),
            inner,
        );
        return;
    };

    let key = CacheKey::new(exchange, inner.width, app.prompt.raw, &app.zone);
    let mut slot = app.prompt.cache.borrow_mut();
    if slot.as_ref().is_none_or(|cache| cache.key != key) {
        *slot = Some(build_cache(app, exchange, &key, usize::from(inner.width)));
    }
    let cache = slot.as_mut().expect("prompt cache");

    let height = inner.height.max(1) as usize;
    let total = cache.lines.len();
    let max_scroll = total.saturating_sub(height);
    app.prompt.max_scroll.set(max_scroll);
    let scroll = if app.prompt.follow {
        max_scroll
    } else {
        app.prompt.scroll.min(max_scroll)
    };
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
        snapshot: app.prompt.cursor,
        peers: if own.is_some() { peers } else { 0 },
        follow: app.prompt.follow,
        zone: &app.zone,
        width,
    };

    let mut lines = Vec::new();
    push_header(&mut lines, &view);

    let rows = if app.prompt.raw {
        raw_rows(exchange)
    } else {
        build_rows(&view)
    };

    let body_width = width.saturating_sub(BODY_INDENT.width()).max(8);
    let mut sections = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            lines.push(Line::from(""));
        }
        let start = lines.len();
        lines.push(heading_line(row, width));
        let body = if row.content.is_empty() {
            vec![("(empty)".to_owned(), theme::faint())]
        } else {
            row.content.clone()
        };
        for (text, style) in &body {
            for line in wrap_text(text, body_width) {
                if line.is_empty() {
                    lines.push(Line::from(""));
                } else {
                    lines.push(Line::styled(format!("{BODY_INDENT}{line}"), *style));
                }
            }
        }
        sections.push(Section { start });
    }

    PromptCache {
        key: key.clone(),
        sections,
        lines,
    }
}

fn heading_line(row: &PromptRow, width: usize) -> Line<'static> {
    let mut heading = Row::new();
    heading.push(" ▌ ", row.label_style);
    heading.push(
        row.label.clone(),
        row.label_style.add_modifier(Modifier::BOLD),
    );
    if let Some(meta) = &row.meta {
        let column = width.saturating_sub(meta.width() + 1);
        heading.pad_to(column);
        heading.push(format!("{meta} "), theme::faint());
    }
    heading.line()
}

fn raw_rows(exchange: &Value) -> Vec<PromptRow> {
    let text = serde_json::to_string_pretty(exchange).unwrap_or_else(|_| exchange.to_string());
    vec![PromptRow {
        label: "exchange · raw JSON".to_owned(),
        label_style: theme::text(),
        content: text
            .lines()
            .map(|line| (line.to_owned(), theme::text()))
            .collect(),
        meta: None,
    }]
}

/// Everything the renderer needs about the exchange on screen.
struct View<'a> {
    exchange: &'a Value,
    /// The next-older exchange, used for change markers.
    older: Option<&'a Value>,
    total: usize,
    /// Zero-based snapshot position; zero is the newest exchange.
    snapshot: usize,
    /// Loaded exchanges sharing this request's prefix hash.
    peers: usize,
    follow: bool,
    zone: &'a Zone,
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
    row.push(" SESSION", theme::accent_style());
    row.push(format!("  {model}"), theme::text());
    let position = format!(" snapshot {} of {} ", view.snapshot + 1, view.total);
    row.pad_to(view.width.saturating_sub(position.width()));
    row.push(position, theme::muted());
    lines.push(row.line());

    let requested = view.exchange["requested_at"].as_i64().unwrap_or(0);
    let session = request["session_id"].as_str().unwrap_or("-");
    let mut row = Row::new();
    row.push(" ", theme::faint());
    row.push(
        if view.follow {
            "● live"
        } else {
            "● paused"
        },
        if view.follow {
            theme::good()
        } else {
            theme::warn()
        },
    );
    row.push("  ·  asked ", theme::faint());
    row.push(view.zone.iso(requested), theme::muted());
    row.push("  ·  session ", theme::faint());
    row.push(short(session, 8), theme::text());
    let previous_count = view.total.saturating_sub(view.snapshot + 1);
    if previous_count > 0 {
        row.push(
            format!("  ·  {previous_count} previous with ["),
            theme::faint(),
        );
    }
    if view.snapshot > 0 {
        row.push(
            format!("  ·  {} newer with ]", view.snapshot),
            theme::faint(),
        );
    }
    lines.push(row.line());

    let hash = view.exchange["prompt_hash"].as_str().unwrap_or("-");
    let prefix = request["prefix_hash"].as_str().unwrap_or("-");
    let total = token(request, "total");
    let fixed = token(request, "fixed");
    let transcript = token(request, "transcript");
    let mut row = Row::new();
    row.push(" prompt   ", theme::muted());
    row.push(short(hash, 12), theme::text());
    row.push("  ·  prefix ", theme::faint());
    row.push(short(prefix, 12), theme::text());
    row.push("  ·  tokens ", theme::faint());
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
    row.push(
        transcript.map(count).unwrap_or_else(|| "-".to_owned()),
        theme::text(),
    );
    row.push(" transcript", theme::faint());
    lines.push(row.line());

    let mut row = Row::new();
    row.push(" cache    ", theme::muted());
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
    if let (Some(fixed), Some(total)) = (fixed, total)
        && total > 0
    {
        row.push("  ·  ", theme::faint());
        row.push(
            format!("{:.1}% fixed", fixed as f64 / total as f64 * 100.0),
            theme::faint(),
        );
    }
    lines.push(row.line());

    if has_markers(view) {
        let mut row = Row::new();
        row.push(" legend   ", theme::muted());
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
    meta: Option<String>,
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
            meta: None,
        }
    }

    fn with_meta(mut self, meta: impl Into<String>) -> Self {
        let meta = meta.into();
        self.meta = (!meta.is_empty()).then_some(meta);
        self
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
            meta: None,
        }
    }

    fn tool_result(prefix: &str, result: &Value) -> Self {
        let failed = result["is_error"].as_bool().unwrap_or(false);
        let content = result["content"].as_str().unwrap_or_default().to_owned();
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
            meta: None,
        }
    }
}

/// Every request segment and response block, with the stable layers first.
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
        ("system · rules", "rules", "rules", theme::accent_style()),
        (
            "system · skills index",
            "skills",
            "skills_index",
            theme::accent_style(),
        ),
    ] {
        let text = view.request()[key].as_str().unwrap_or_default();
        rows.push(
            PromptRow::text(&row_label(view, label, Some(part)), style, text)
                .with_meta(token_meta(view.request(), key)),
        );
    }

    rows.push(tool_row(view));

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
                rows.push(
                    PromptRow::text("system · relevant memory", theme::good(), recall)
                        .with_meta(token_meta(view.request(), "recall")),
                );
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
        if let Some(note) = message["runtime_note"].as_str() {
            rows.push(
                PromptRow::text("runtime note", theme::muted(), note)
                    .with_meta(token_meta(view.request(), "runtime")),
            );
        }
    }

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
    let usage = &view.exchange["usage"];
    if !usage.is_null() {
        rows.push(PromptRow {
            label: "response · usage".to_owned(),
            label_style: theme::faint(),
            content: vec![(usage_line(usage), theme::faint())],
            meta: view.exchange["response"]["tokens_est"]
                .as_u64()
                .filter(|tokens| *tokens > 0)
                .map(|tokens| format!("{} tok", count(tokens))),
        });
    }

    rows
}

fn tool_row(view: &View<'_>) -> PromptRow {
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
    if tool_content.is_empty() {
        tool_content.push(("(no tools)".to_owned(), theme::faint()));
    }
    PromptRow {
        label: row_label(view, "request · tools", Some("tools")),
        label_style: theme::accent_style(),
        content: tool_content,
        meta: Some(token_meta(view.request(), "tools")),
    }
}

fn row_label(view: &View<'_>, label: &str, part: Option<&str>) -> String {
    if view.changed(part) {
        format!("▲ {label}")
    } else {
        label.to_owned()
    }
}

fn token_meta(request: &Value, key: &str) -> String {
    token(request, key)
        .map(|tokens| format!("{} tok", count(tokens)))
        .unwrap_or_default()
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
        if required.contains(&name.as_str()) {
            field.push_str(" (required)");
        } else {
            field.push_str(" (optional)");
        }
        if let Some(values) = schema.get("enum").and_then(Value::as_array) {
            let values: Vec<&str> = values.iter().filter_map(Value::as_str).collect();
            if !values.is_empty() {
                field.push_str(&format!(" [{}]", values.join(" | ")));
            }
        }
        if let Some(description) = schema["description"].as_str() {
            let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
            if !description.is_empty() {
                field.push_str(&format!(" — {description}"));
            }
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

/// Wrap text without losing leading indentation. Long words are hard-split.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines = Vec::new();
    for raw in text.lines() {
        let expanded = raw.replace('\t', "    ");
        let trimmed = expanded.trim_end();
        if trimmed.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        let indent_len = trimmed.len() - trimmed.trim_start_matches(' ').len();
        let indent = " ".repeat(indent_len.min(width.saturating_sub(1)));
        let body = trimmed.trim_start_matches(' ');
        let mut line = indent.clone();
        let mut has_word = false;
        for word in body.split_whitespace() {
            let word_width = word.width();
            let needed = if has_word { 1 + word_width } else { word_width };
            if line.width() + needed <= width {
                if has_word {
                    line.push(' ');
                }
                line.push_str(word);
                has_word = true;
                continue;
            }
            if has_word {
                lines.push(std::mem::take(&mut line));
                line = indent.clone();
            }
            if line.width() + word_width <= width {
                line.push_str(word);
                has_word = true;
                continue;
            }
            let available = width.saturating_sub(line.width()).max(1);
            let mut chunk = String::new();
            let mut chunk_width = 0;
            let mut chunks = Vec::new();
            for character in word.chars() {
                let character_width = character.width().unwrap_or(0);
                if chunk_width > 0 && chunk_width + character_width > available {
                    chunks.push(std::mem::take(&mut chunk));
                    chunk_width = 0;
                }
                chunk.push(character);
                chunk_width += character_width;
            }
            if !chunk.is_empty() {
                chunks.push(chunk);
            }
            let last = chunks.pop().unwrap_or_default();
            for chunk in chunks {
                line.push_str(&chunk);
                lines.push(std::mem::take(&mut line));
                line = indent.clone();
            }
            line.push_str(&last);
            has_word = true;
        }
        if has_word || indent.is_empty() {
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
    use crate::app::{App, Screen, Tab};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use serde_json::{Value, json};
    use tempfile::TempDir;

    const QUESTION: &str = "please inspect the workspace";
    const ANSWER: &str = "the workspace holds the binary crate";
    const SOUL: &str = "SOUL TEXT the identity layer";

    fn prompt_app(exchanges: Vec<Value>) -> (TempDir, App) {
        let home = TempDir::new().expect("temp home");
        let mut app = App::new(home.path().to_path_buf());
        app.screen = Screen::Dashboard;
        app.tab = Tab::Prompt;
        app.zone = Zone::utc();
        app.prompt.exchanges = exchanges;
        (home, app)
    }

    /// Render the Prompt tab alone at a fixed width, one string per screen row.
    fn rendered(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                let area = frame.area();
                render(app, frame, area);
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for row in 0..height {
            for column in 0..width {
                text.push_str(buffer[(column, row)].symbol());
            }
            text.push('\n');
        }
        text
    }

    /// Rows whose visible text starts with `label`.
    fn starting(text: &str, label: &str) -> Vec<usize> {
        text.lines()
            .enumerate()
            .filter(|(_, line)| line.trim_start().starts_with(label))
            .map(|(index, _)| index)
            .collect()
    }

    /// Rows whose visible text is exactly `label`.
    fn exact(text: &str, label: &str) -> Vec<usize> {
        text.lines()
            .enumerate()
            .filter(|(_, line)| line.trim_end() == label)
            .map(|(index, _)| index)
            .collect()
    }

    fn line_of(text: &str, needle: &str) -> usize {
        text.lines()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("missing {needle} in\n{text}"))
    }

    fn request(transcript: Value, total: u64) -> Value {
        json!({
            "identity": SOUL,
            "guidance": "PROJECT RULES",
            "rules": "STANDING LINES",
            "skills_index": "skill-one: does things",
            "tools": [{
                "name": "read_file",
                "description": "Read a file from the workspace.",
                "parameters": {
                    "type": "object",
                    "properties": {"path": {"type": "string", "description": "The path to read."}},
                    "required": ["path"]
                }
            }],
            "recall": "MEMORY HIT",
            "transcript": transcript,
            "model": "test-model",
            "session_id": "session-one",
            "prefix_hash": "prefix-one",
            "prefix_parts": {
                "identity": "i1",
                "guidance": "g1",
                "rules": "r1",
                "skills": "k1",
                "tools": "t1"
            },
            "tokens": {
                "identity": 10,
                "guidance": 20,
                "rules": 30,
                "skills": 40,
                "tools": 50,
                "transcript": 60,
                "recall": 70,
                "runtime": 80,
                "total": total,
                "fixed": 150
            }
        })
    }

    /// Two saved requests of one conversation, newest first.
    fn conversation() -> Vec<Value> {
        let user = json!({
            "role": "user",
            "runtime_note": "CLOCK NOTE seven seconds past",
            "blocks": [{"type": "text", "text": QUESTION}]
        });
        let call = json!({
            "type": "tool_call",
            "id": "c1",
            "name": "read_file",
            "arguments": {"path": "Cargo.toml"}
        });
        let answered = json!({"role": "assistant", "blocks": [call.clone()]});
        let result = json!({
            "role": "tool",
            "runtime_note": "CLOCK NOTE eight seconds past",
            "blocks": [{"type": "tool_result", "call_id": "c1", "content": "package hearth-cli"}]
        });
        let newest = json!({
            "request": request(json!([user.clone(), answered, result]), 620),
            "prompt_hash": "bbbbbbbb-1111-2222-3333-444444444444",
            "requested_at": 1_700_000_010_000_000_i64,
            "response": {
                "entry_id": "entry-2",
                "created_at": 1_700_000_015_000_000_i64,
                "blocks": [{"type": "text", "text": ANSWER}],
                "tokens_est": 12
            },
            "usage": Value::Null
        });
        let oldest = json!({
            "request": request(json!([user]), 480),
            "prompt_hash": "aaaaaaaa-1111-2222-3333-444444444444",
            "requested_at": 1_700_000_000_000_000_i64,
            "response": {
                "entry_id": "entry-1",
                "created_at": 1_700_000_005_000_000_i64,
                "blocks": [call],
                "tokens_est": 20
            },
            "usage": Value::Null
        });
        vec![newest, oldest]
    }

    /// Three saved requests of one long conversation, newest first.
    fn long_conversation() -> Vec<Value> {
        let identity = (0..10)
            .map(|index| format!("identity line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut saved = Vec::new();
        let mut messages: Vec<Value> = Vec::new();
        for index in 0..3 {
            let body = (0..18)
                .map(|line| format!("question {index} line {line}"))
                .collect::<Vec<_>>()
                .join("\n");
            messages.push(json!({"role": "user", "blocks": [{"type": "text", "text": body}]}));
            let mut document = request(Value::Array(messages.clone()), 500 + index);
            document["identity"] = json!(identity);
            let answer = format!("answer {index}");
            messages
                .push(json!({"role": "assistant", "blocks": [{"type": "text", "text": answer}]}));
            saved.push(json!({
                "request": document,
                "prompt_hash": format!("hash-{index}-0123456789"),
                "requested_at": 1_700_000_000_000_000_i64 + index as i64 * 10_000_000,
                "response": {
                    "entry_id": format!("entry-{index}"),
                    "created_at": 1_700_000_005_000_000_i64,
                    "blocks": [{"type": "text", "text": answer}],
                    "tokens_est": 10
                },
                "usage": Value::Null
            }));
        }
        saved.reverse();
        saved
    }

    fn cache_lines(app: &App) -> Vec<String> {
        let cache = app.prompt.cache.borrow();
        let cache = cache.as_ref().expect("the render must build a cache");
        cache
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn a_user_message_reads_differently_from_a_system_layer() {
        let (_home, app) = prompt_app(conversation());
        let text = rendered(&app, 100, 60);
        let system = starting(&text, "system · identity");
        assert_eq!(
            system.len(),
            1,
            "the identity layer belongs to the conversation once: {text}"
        );
        let user = exact(&text, "you");
        assert_eq!(
            user.len(),
            1,
            "the merged transcript shows the question once: {text}"
        );
        assert!(system[0] < user[0], "the system layers come first: {text}");
        assert!(text.contains(SOUL), "the identity body is visible: {text}");
        assert!(text.contains("PROJECT RULES"), "{text}");
        assert!(text.contains("MEMORY HIT"), "{text}");
        assert!(
            starting(&text, "hearth").len() == 1,
            "the answer is labelled as Hearth: {text}"
        );
    }

    #[test]
    fn tool_calls_and_results_sit_between_the_question_and_the_answer() {
        let (_home, app) = prompt_app(conversation());
        let text = rendered(&app, 100, 60);
        let question = line_of(&text, QUESTION);
        let call = line_of(&text, "tool call · read_file");
        let result = line_of(&text, "tool result");
        let answer = line_of(&text, ANSWER);
        assert!(
            question < call && call < result && result < answer,
            "the turn must read question, call, result, answer: {text}"
        );
        assert!(
            text.contains("Cargo.toml"),
            "the arguments are shown: {text}"
        );
        assert!(
            text.contains("package hearth-cli"),
            "the result body is shown: {text}"
        );
    }

    #[test]
    fn two_snapshots_of_one_conversation_do_not_repeat_the_history() {
        let (_home, app) = prompt_app(conversation());
        let text = rendered(&app, 100, 60);
        assert_eq!(
            text.matches(QUESTION).count(),
            1,
            "the earlier question is not repeated: {text}"
        );
        assert_eq!(
            text.matches("package hearth-cli").count(),
            1,
            "the earlier tool result is not repeated: {text}"
        );
        assert!(
            text.contains("1 · 2023-11-14T22:13:20+00:00"),
            "the oldest snapshot marker is present: {text}"
        );
        assert!(
            text.contains("2 · 2023-11-14T22:13:30+00:00"),
            "the newest snapshot marker is present: {text}"
        );
        assert!(
            text.contains("aaaaaaaa"),
            "the marker carries the short request hash: {text}"
        );
        assert!(
            text.contains("▸── 2 · "),
            "the newest snapshot is the selected one: {text}"
        );
    }

    #[test]
    fn the_runtime_note_renders_under_the_message_that_carried_it() {
        let (_home, app) = prompt_app(conversation());
        let text = rendered(&app, 100, 60);
        assert!(
            !text.contains("system · CLOCK") && !text.contains("runtime note"),
            "the note is not a system message of its own: {text}"
        );
        let first = exact(&text, "you")[0];
        let note = line_of(&text, "CLOCK NOTE seven seconds past");
        let call = line_of(&text, "tool call · read_file");
        assert!(
            first < note && note < call,
            "the note sits under the question it rode with: {text}"
        );
        let result = exact(&text, "tool result")[0];
        let second = line_of(&text, "CLOCK NOTE eight seconds past");
        let answer = line_of(&text, ANSWER);
        assert!(
            result < second && second < answer,
            "the note sits under the tool result it rode with: {text}"
        );
    }

    #[test]
    fn the_working_line_appears_only_while_a_turn_is_in_flight() {
        let (_home, mut app) = prompt_app(conversation());
        assert!(
            !rendered(&app, 100, 24).contains("working"),
            "a settled view shows no working line"
        );
        app.prompt.loading = true;
        assert!(
            rendered(&app, 100, 24).contains("working"),
            "a loading view shows the working line"
        );
        app.prompt.follow = false;
        app.prompt.scroll = 0;
        assert!(
            rendered(&app, 100, 24).contains("working"),
            "the working line stays visible away from the tail"
        );
        app.prompt.loading = false;
        app.status = Some(json!({"tasks_running": 2}));
        assert!(
            rendered(&app, 100, 24).contains("working"),
            "a running task shows the working line"
        );
        app.status = Some(json!({"tasks_running": 0}));
        assert!(!rendered(&app, 100, 24).contains("working"));
    }

    #[test]
    fn the_s_key_collapses_the_system_layers_and_shows_them_again() {
        let (_home, mut app) = prompt_app(conversation());
        assert!(rendered(&app, 100, 60).contains(SOUL));
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        let hidden = rendered(&app, 100, 60);
        assert!(
            !hidden.contains(SOUL),
            "the system bodies are hidden: {hidden}"
        );
        assert!(
            hidden.contains("identity, guidance, rules, skills index, tools, recall"),
            "the summary line names the layers: {hidden}"
        );
        assert!(
            hidden.contains("tok"),
            "the summary names the cost: {hidden}"
        );
        assert!(
            hidden.contains("s system"),
            "the key hint stays visible: {hidden}"
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        assert!(
            rendered(&app, 100, 60).contains(SOUL),
            "the second press shows the layers again"
        );
    }

    #[test]
    fn brackets_move_the_viewport_to_the_previous_and_next_snapshot_marker() {
        let (_home, mut app) = prompt_app(long_conversation());
        let _ = rendered(&app, 100, 24);
        app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        let text = rendered(&app, 100, 24);
        assert!(
            text.lines().nth(1).unwrap_or_default().contains("▸── 2 · "),
            "the older marker moves to the top of the viewport: {text}"
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        let text = rendered(&app, 100, 24);
        assert!(
            text.lines().nth(1).unwrap_or_default().contains("▸── 1 · "),
            "the oldest marker moves to the top of the viewport: {text}"
        );
        app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        let text = rendered(&app, 100, 24);
        assert!(
            text.lines().nth(1).unwrap_or_default().contains("▸── 2 · "),
            "the newer marker moves to the top of the viewport: {text}"
        );
    }

    #[test]
    fn a_rewritten_history_is_not_hidden() {
        let mut exchanges = conversation();
        exchanges.insert(
            0,
            json!({
                "request": request(
                    json!([{"role": "user", "blocks": [{"type": "text", "text": "a fresh start"}]}]),
                    700
                ),
                "prompt_hash": "cccccccc-1111-2222-3333-444444444444",
                "requested_at": 1_700_000_020_000_000_i64,
                "response": {
                    "entry_id": "entry-3",
                    "created_at": 1_700_000_025_000_000_i64,
                    "blocks": [{"type": "text", "text": "starting again"}],
                    "tokens_est": 8
                },
                "usage": Value::Null
            }),
        );
        let (_home, app) = prompt_app(exchanges);
        let text = rendered(&app, 100, 60);
        let rewritten = line_of(&text, "history rewritten");
        let fresh = line_of(&text, "a fresh start");
        assert!(
            rewritten < fresh,
            "the note comes before the rewritten conversation: {text}"
        );
        assert!(text.contains("earlier messages dropped"), "{text}");
    }

    #[test]
    fn a_realistic_transcript_fits_eighty_and_one_hundred_and_twenty_columns() {
        for width in [80u16, 120] {
            let (_home, app) = prompt_app(conversation());
            let text = rendered(&app, width, 40);
            for line in cache_lines(&app) {
                assert!(
                    line.width() <= usize::from(width) - 2,
                    "line {line:?} does not fit {width} columns"
                );
            }
            for expected in ["system · identity", "you", "tool call · read_file", ANSWER] {
                assert!(
                    text.contains(expected),
                    "width {width} is missing {expected}: {text}"
                );
            }
            assert!(
                text.contains("[ ] snapshots · f follow · s system · r raw · o refresh"),
                "width {width} keeps the hint line: {text}"
            );
        }
    }

    #[test]
    fn a_narrow_width_keeps_every_line_inside_the_card() {
        for width in [48u16, 24, 12] {
            let (_home, app) = prompt_app(conversation());
            let _ = rendered(&app, width, 30);
            for line in cache_lines(&app) {
                assert!(
                    line.width() <= usize::from(width) - 2,
                    "line {line:?} overflows {width} columns"
                );
            }
        }
    }

    #[test]
    fn wrapping_keeps_leading_indentation_and_stays_inside_the_width() {
        let lines = wrap_text("    indented line that wraps at the margin", 18);
        assert!(
            lines[0].starts_with("    "),
            "the first line must keep its indentation: {lines:?}"
        );
        assert!(
            lines.len() > 1,
            "the long line should wrap into several lines: {lines:?}"
        );
        assert!(
            lines.iter().skip(1).all(|line| line.starts_with("    ")),
            "continuation lines must keep the same indentation: {lines:?}"
        );
        for line in &lines {
            assert!(line.width() <= 18, "{line:?} overflows the width");
        }
    }
}
