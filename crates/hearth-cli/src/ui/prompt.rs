//! The Prompt tab as one conversation.
//!
//! The harness saves a full prompt for every request, so consecutive requests
//! differ only in their tail. This view merges the saved requests into a single
//! transcript and prints each request's delta under a snapshot marker. `[` and
//! `]` walk the markers, `s` hides the system layers, `r` shows the documents
//! as they were saved, and a spinner says when a turn is in flight.

use std::sync::OnceLock;
use std::time::Instant;

use hearth_runtime::Zone;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, PromptView};
use crate::theme;
use crate::ui::{count, short};

/// Body lines moved by PageUp and PageDown.
const PAGE_LINES: usize = 15;
/// Message and layer bodies sit this far under their label.
const BODY_INDENT: usize = 2;
/// One frame per 100 ms, which is the event loop's budget.
const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
/// The stable prefix layers, with the names a reader knows them by.
const PREFIX_LAYERS: [(&str, &str); 5] = [
    ("identity", "SOUL.md"),
    ("guidance", "AGENTS.md"),
    ("rules", "rules"),
    ("skills", "skills index"),
    ("tools", "tools"),
];

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
    prompt.selected = marker_count(prompt).saturating_sub(1);
}

pub(crate) fn toggle_follow(prompt: &mut PromptView) {
    prompt.follow = !prompt.follow;
    if prompt.follow {
        prompt.scroll = prompt.max_scroll.get();
    }
}

/// Show the next-older saved snapshot with its marker at the top.
pub(crate) fn older(prompt: &mut PromptView) -> bool {
    if prompt.cursor + 1 >= prompt.exchanges.len() {
        return false;
    }
    prompt.cursor += 1;
    prompt.follow = false;
    jump_to_cursor(prompt);
    true
}

/// Show the next-newer saved snapshot. The newest snapshot resumes the tail.
pub(crate) fn newer(prompt: &mut PromptView) -> bool {
    if prompt.cursor == 0 {
        return false;
    }
    prompt.cursor -= 1;
    prompt.follow = prompt.cursor == 0;
    jump_to_cursor(prompt);
    true
}

/// Put the selected snapshot's marker at the top of the viewport.
fn jump_to_cursor(prompt: &mut PromptView) {
    if prompt.follow {
        prompt.scroll = prompt.max_scroll.get();
    } else if let Some(line) = prompt
        .cache
        .borrow()
        .as_ref()
        .and_then(|cache| cache.marker_line(prompt.cursor))
    {
        prompt.scroll = line;
    } else {
        prompt.scroll = 0;
    }
    prompt.selected = marker_index(prompt);
}

/// The selected snapshot as an index into the chronological markers.
fn marker_index(prompt: &PromptView) -> usize {
    prompt
        .cache
        .borrow()
        .as_ref()
        .map(|cache| cache.markers.len().saturating_sub(prompt.cursor + 1))
        .unwrap_or(0)
}

fn marker_count(prompt: &PromptView) -> usize {
    prompt
        .cache
        .borrow()
        .as_ref()
        .map(|cache| cache.markers.len())
        .unwrap_or(0)
}

/// Point `selected` at the marker nearest the top of the viewport.
fn sync_selected(prompt: &mut PromptView) {
    let index = prompt
        .cache
        .borrow()
        .as_ref()
        .map(|cache| {
            cache
                .markers
                .iter()
                .rposition(|marker| marker.line <= prompt.scroll)
                .unwrap_or(0)
        })
        .unwrap_or(0);
    prompt.selected = index;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheKey {
    /// The loaded requests, named by hash and answering entry.
    signature: String,
    raw: bool,
    show_system: bool,
    width: u16,
    zone: String,
}

impl CacheKey {
    fn new(exchanges: &[Value], width: u16, raw: bool, show_system: bool, zone: &Zone) -> Self {
        let mut signature = String::new();
        for exchange in exchanges {
            signature.push_str(exchange["prompt_hash"].as_str().unwrap_or("-"));
            signature.push(':');
            signature.push_str(exchange["response"]["entry_id"].as_str().unwrap_or("-"));
            signature.push('|');
        }
        Self {
            signature,
            raw,
            show_system,
            width,
            zone: zone.name().to_owned(),
        }
    }
}

/// A snapshot marker inside the built document.
struct Marker {
    /// Line of the marker in the document.
    line: usize,
    /// The same marker in the selection emphasis.
    selected: Line<'static>,
}

/// One built document, kept until the loaded set or the view flags change.
pub(crate) struct PromptCache {
    key: CacheKey,
    /// Markers in chronological order, oldest first.
    markers: Vec<Marker>,
    lines: Vec<Line<'static>>,
}

impl PromptCache {
    /// The marker line of the exchange at `cursor`, counted from the newest.
    fn marker_line(&self, cursor: usize) -> Option<usize> {
        self.markers
            .len()
            .checked_sub(cursor + 1)
            .map(|index| self.markers[index].line)
    }
}

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let title = if app.prompt.loading {
        " Prompt · refreshing "
    } else if app.prompt.raw {
        " Prompt · raw "
    } else if app.prompt.follow {
        " Prompt · conversation · live "
    } else {
        " Prompt · conversation · paused "
    };
    let card = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(theme::heading(title));
    let inner = card.inner(area);
    frame.render_widget(card, area);

    let busy = app.prompt.loading || running_tasks(app) > 0;
    let height = usize::from(inner.height);
    let header_rows = usize::from(!app.prompt.exchanges.is_empty() && height >= 2);
    let busy_rows = usize::from(busy && height > header_rows + 1);
    let body_rows = height.saturating_sub(header_rows + busy_rows);
    let header_area = Rect {
        height: header_rows as u16,
        ..inner
    };
    let body = Rect {
        y: inner.y + header_rows as u16,
        height: body_rows as u16,
        ..inner
    };
    let busy_area = Rect {
        y: inner.y + (header_rows + body_rows) as u16,
        height: busy_rows as u16,
        ..inner
    };

    if header_rows > 0 {
        frame.render_widget(
            Paragraph::new(header_line(app, usize::from(inner.width))),
            header_area,
        );
    }

    if app.prompt.exchanges.is_empty() {
        let message = if app.prompt.loading {
            vec![Line::styled("Loading saved prompts...", theme::warn())]
        } else {
            vec![
                Line::styled("No session yet.", theme::muted()),
                Line::from(""),
                Line::styled(
                    "Send Hearth a message. This view refreshes after every reply.",
                    theme::faint(),
                ),
            ]
        };
        frame.render_widget(Paragraph::new(message), body);
    } else if body_rows > 0 {
        draw_transcript(app, frame, body);
    }

    if busy_rows > 0 {
        frame.render_widget(Paragraph::new(working_line()), busy_area);
    }
}

/// The model, the selected snapshot, and the numbers a reader needs at a glance.
fn header_line(app: &App, width: usize) -> Line<'static> {
    let Some(exchange) = app.prompt.current() else {
        return Line::from("");
    };
    let request = &exchange["request"];
    let model = request["model"].as_str().unwrap_or("unknown model");
    let session = request["session_id"].as_str().unwrap_or("-");
    let prefix = request["prefix_hash"].as_str().unwrap_or("-");
    let snapshots = app.prompt.exchanges.len();
    let selected = snapshots.saturating_sub(app.prompt.cursor);
    let peers = app
        .prompt
        .exchanges
        .iter()
        .filter(|other| other["request"]["prefix_hash"] == request["prefix_hash"])
        .count();
    let mut row = Row::new();
    row.push(format!(" {model}"), theme::text());
    row.push(" · session ", theme::faint());
    row.push(short(session, 8), theme::muted());
    row.push(
        format!(" · snapshot {selected} of {snapshots}"),
        theme::muted(),
    );
    if let Some(tokens) = token(request, "total") {
        row.push(format!(" · {} tok", count(tokens)), theme::muted());
    }
    row.push(" · prefix ", theme::faint());
    row.push(short(prefix, 8), theme::muted());
    if peers > 1 {
        row.push(format!(" · {peers} peers"), theme::good());
    }
    row.fit(width)
}

fn running_tasks(app: &App) -> u64 {
    app.status
        .as_ref()
        .and_then(|status| status["tasks_running"].as_u64())
        .unwrap_or(0)
}

fn draw_transcript(app: &App, frame: &mut Frame, area: Rect) {
    let width = usize::from(area.width);
    let height = usize::from(area.height);
    let key = CacheKey::new(
        &app.prompt.exchanges,
        area.width,
        app.prompt.raw,
        app.prompt.show_system,
        &app.zone,
    );
    let mut slot = app.prompt.cache.borrow_mut();
    if slot.as_ref().is_none_or(|cache| cache.key != key) {
        *slot = Some(build_cache(&app.prompt.exchanges, &key, &app.zone, width));
    }
    let cache = slot.as_mut().expect("prompt cache");

    let total = cache.lines.len();
    let max_scroll = total.saturating_sub(height);
    app.prompt.max_scroll.set(max_scroll);
    let scroll = if app.prompt.follow {
        max_scroll
    } else {
        app.prompt.scroll.min(max_scroll)
    };
    let end = (scroll + height).min(total);
    let selected = cache.markers.len().checked_sub(app.prompt.cursor + 1);
    let lines: Vec<Line<'static>> = (scroll..end)
        .map(|index| match selected {
            Some(marker) if cache.markers[marker].line == index => {
                cache.markers[marker].selected.clone()
            }
            _ => cache.lines[index].clone(),
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn working_line() -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {} ", FRAMES[spinner_frame()]),
            theme::accent_style(),
        ),
        Span::styled("working", theme::muted()),
    ])
}

fn spinner_frame() -> usize {
    static START: OnceLock<Instant> = OnceLock::new();
    let elapsed = START.get_or_init(Instant::now).elapsed().as_millis();
    (elapsed / 100) as usize % FRAMES.len()
}

/// One block of a conversation, derived from the saved requests.
enum Entry {
    /// A saved request, printed before its delta.
    Snapshot {
        number: usize,
        at: i64,
        hash: String,
        tokens: Option<u64>,
        changed: Vec<String>,
    },
    /// The stable layers, printed once and again only when they change.
    System { parts: Vec<SystemPart>, tokens: u64 },
    /// The memory injected for this request, before its new input.
    Recall { text: String, tokens: u64 },
    /// A different conversation starts here.
    Conversation { id: String },
    /// A message that reached the model.
    Message { role: Role, blocks: Vec<Value> },
    /// The clock note that rode on the message before it.
    Runtime { note: String },
    /// Compaction dropped messages this view has already shown.
    Rewritten { dropped: usize },
    /// What the model answered.
    Response { blocks: Option<Vec<Value>> },
}

struct SystemPart {
    label: &'static str,
    text: String,
    tokens: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    User,
    Assistant,
    Tool,
    System,
}

impl Role {
    fn parse(value: &str) -> Self {
        match value {
            "user" => Self::User,
            "assistant" => Self::Assistant,
            "tool" => Self::Tool,
            _ => Self::System,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::User => "you",
            Self::Assistant => "hearth",
            Self::Tool => "tool",
            Self::System => "system",
        }
    }

    fn style(self) -> Style {
        match self {
            Self::User => theme::accent_style(),
            Self::Assistant => theme::text(),
            Self::Tool => theme::good(),
            Self::System => theme::muted(),
        }
    }
}

/// Build the document once, with every system layer where it changed, the
/// delta of every snapshot, and the answer that followed it.
fn derive(exchanges: &[Value]) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut previous_sent: Vec<(String, String)> = Vec::new();
    let mut previous_parts: Option<Value> = None;
    let mut previous_session: Option<String> = None;
    for (position, exchange) in exchanges.iter().rev().enumerate() {
        let request = &exchange["request"];
        let session = request["session_id"].as_str().unwrap_or("-").to_owned();
        let new_session = previous_session
            .as_deref()
            .is_some_and(|previous| previous != session);
        if new_session {
            entries.push(Entry::Conversation {
                id: session.clone(),
            });
            previous_sent.clear();
            previous_parts = None;
        }
        let changed = previous_parts
            .as_ref()
            .map(|previous| changed_parts(previous, request))
            .unwrap_or_default();
        if position == 0 || new_session || !changed.is_empty() {
            let parts = system_parts(request);
            let tokens = parts.iter().map(|part| part.tokens).sum();
            entries.push(Entry::System { parts, tokens });
        }
        entries.push(Entry::Snapshot {
            number: position + 1,
            at: exchange["requested_at"].as_i64().unwrap_or(0),
            hash: exchange["prompt_hash"].as_str().unwrap_or("-").to_owned(),
            tokens: token(request, "total"),
            changed,
        });
        let recall = request["recall"].as_str().unwrap_or_default().to_owned();
        if !recall.trim().is_empty() {
            entries.push(Entry::Recall {
                text: recall,
                tokens: token(request, "recall").unwrap_or(0),
            });
        }

        let messages = request["transcript"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let transcript: Vec<(String, String)> = messages.iter().map(normalized).collect();
        let kept = transcript
            .iter()
            .zip(&previous_sent)
            .take_while(|(current, previous)| current == previous)
            .count();
        if kept < previous_sent.len() {
            entries.push(Entry::Rewritten {
                dropped: previous_sent.len() - kept,
            });
        }
        for message in &messages[kept..] {
            entries.push(Entry::Message {
                role: Role::parse(message["role"].as_str().unwrap_or_default()),
                blocks: message["blocks"].as_array().cloned().unwrap_or_default(),
            });
            if let Some(note) = message["runtime_note"].as_str()
                && !note.trim().is_empty()
            {
                entries.push(Entry::Runtime {
                    note: note.to_owned(),
                });
            }
        }
        entries.push(Entry::Response {
            blocks: response_blocks(&exchange["response"]),
        });
        previous_sent = sent_messages(&transcript, &exchange["response"]);
        previous_parts = Some(request["prefix_parts"].clone());
        previous_session = Some(session);
    }
    entries
}

/// The prefix layers that differ from the previous saved request.
fn changed_parts(previous: &Value, request: &Value) -> Vec<String> {
    let current = &request["prefix_parts"];
    let mut changed = Vec::new();
    for (part, label) in PREFIX_LAYERS {
        if current[part] != previous[part] {
            changed.push(label.to_owned());
        }
    }
    changed
}

fn system_parts(request: &Value) -> Vec<SystemPart> {
    let mut parts = vec![
        layer(request, "identity"),
        layer(request, "guidance"),
        layer(request, "rules"),
        // The saved key and the token key differ for the skills index.
        SystemPart {
            label: "skills index",
            text: text_of(request, "skills_index"),
            tokens: token(request, "skills").unwrap_or(0),
        },
        SystemPart {
            label: "tools",
            text: tools_text(request),
            tokens: token(request, "tools").unwrap_or(0),
        },
    ];
    parts.retain(|part| !part.text.trim().is_empty());
    parts
}

/// A layer whose saved key and token key agree.
fn layer(request: &Value, name: &'static str) -> SystemPart {
    SystemPart {
        label: name,
        text: text_of(request, name),
        tokens: token(request, name).unwrap_or(0),
    }
}

fn text_of(request: &Value, key: &str) -> String {
    request[key].as_str().unwrap_or_default().to_owned()
}

/// The tool declarations as one readable block.
fn tools_text(request: &Value) -> String {
    let tools = request["tools"].as_array().cloned().unwrap_or_default();
    let mut lines = Vec::new();
    for tool in &tools {
        lines.push(tool["name"].as_str().unwrap_or("tool").to_owned());
        if let Some(description) = tool["description"].as_str()
            && !description.trim().is_empty()
        {
            lines.push(description.to_owned());
        }
        if let Some(parameters) = tool.get("parameters").filter(|value| !value.is_null()) {
            let fields = schema_fields(parameters);
            if !fields.is_empty() {
                lines.push("args".to_owned());
                lines.extend(fields.into_iter().map(|field| format!("  {field}")));
            }
        }
    }
    lines.join("\n")
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
                field.push_str(&format!(" · {description}"));
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

/// One message as it was compared, as a role and its blocks, without the
/// audit note.
fn normalized(message: &Value) -> (String, String) {
    let role = message["role"].as_str().unwrap_or_default().to_owned();
    let blocks = message
        .get("blocks")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    (role, blocks.to_string())
}

/// The transcript the request carried, plus the answer that came back.
fn sent_messages(transcript: &[(String, String)], response: &Value) -> Vec<(String, String)> {
    let mut sent = transcript.to_vec();
    if let Some(blocks) = response["blocks"].as_array() {
        sent.push((
            "assistant".to_owned(),
            Value::Array(blocks.clone()).to_string(),
        ));
    }
    sent
}

fn response_blocks(response: &Value) -> Option<Vec<Value>> {
    if response.is_null() {
        return None;
    }
    response["blocks"].as_array().cloned()
}

fn token(document: &Value, name: &str) -> Option<u64> {
    document["tokens"][name].as_u64()
}

fn build_cache(exchanges: &[Value], key: &CacheKey, zone: &Zone, width: usize) -> PromptCache {
    let mut painter = Painter::new(width);
    if key.raw {
        for (position, exchange) in exchanges.iter().rev().enumerate() {
            painter.marker(
                position + 1,
                exchange["requested_at"].as_i64().unwrap_or(0),
                exchange["prompt_hash"].as_str().unwrap_or("-"),
                token(&exchange["request"], "total"),
                &[],
                zone,
            );
            let text =
                serde_json::to_string_pretty(exchange).unwrap_or_else(|_| exchange.to_string());
            for line in text.lines() {
                painter.body(line, theme::text());
            }
        }
    } else {
        for entry in derive(exchanges) {
            push_entry(&mut painter, &entry, key.show_system, zone);
        }
    }
    PromptCache {
        key: key.clone(),
        markers: painter.markers,
        lines: painter.lines,
    }
}

fn push_entry(painter: &mut Painter, entry: &Entry, show_system: bool, zone: &Zone) {
    match entry {
        Entry::Snapshot {
            number,
            at,
            hash,
            tokens,
            changed,
        } => painter.marker(*number, *at, hash, *tokens, changed, zone),
        Entry::System { parts, tokens } => push_system(painter, parts, *tokens, show_system),
        Entry::Recall { text, tokens } => push_recall(painter, text, *tokens, show_system),
        Entry::Conversation { id } => painter.conversation(id),
        Entry::Message { role, blocks } => push_blocks(painter, *role, blocks),
        Entry::Runtime { note } => painter.note(note),
        Entry::Rewritten { dropped } => painter.rewritten(*dropped),
        Entry::Response { blocks } => match blocks {
            Some(blocks) => push_blocks(painter, Role::Assistant, blocks),
            None => painter.faint("no reply saved"),
        },
    }
}

fn push_system(painter: &mut Painter, parts: &[SystemPart], tokens: u64, show: bool) {
    if !show {
        let names = parts
            .iter()
            .map(|part| part.label)
            .collect::<Vec<_>>()
            .join(", ");
        painter.summary(
            &format!("system · {names}"),
            theme::muted(),
            &format!(" · {} tok", count(tokens)),
        );
        return;
    }
    for part in parts {
        painter.label(
            &format!("system · {}", part.label),
            theme::muted(),
            &format!("{} tok", count(part.tokens)),
        );
        painter.body(&part.text, theme::muted());
    }
}

fn push_recall(painter: &mut Painter, text: &str, tokens: u64, show: bool) {
    if show {
        painter.label(
            "system · memory",
            theme::good(),
            &format!("{} tok", count(tokens)),
        );
        painter.body(text, theme::good());
    } else {
        painter.summary(
            "system · memory",
            theme::good(),
            &format!(" · {} tok", count(tokens)),
        );
    }
}

fn push_blocks(painter: &mut Painter, role: Role, blocks: &[Value]) {
    if blocks.is_empty() {
        painter.label(role.label(), role.style(), "");
        painter.body("(empty)", theme::faint());
        return;
    }
    for block in blocks {
        match block["type"].as_str().unwrap_or_default() {
            "thinking" => {
                painter.label("thinking", theme::think(), "");
                painter.body(block["text"].as_str().unwrap_or_default(), theme::think());
            }
            "text" => {
                painter.label(role.label(), role.style(), "");
                painter.body(block["text"].as_str().unwrap_or_default(), role.style());
            }
            "tool_call" => {
                let name = block["name"].as_str().unwrap_or("tool");
                painter.label(&format!("tool call · {name}"), theme::warn(), "");
                let arguments = block
                    .get("arguments")
                    .map(|arguments| arguments.to_string())
                    .unwrap_or_default();
                painter.body(&arguments, theme::muted());
            }
            "tool_result" => {
                let failed = block["is_error"].as_bool().unwrap_or(false);
                let style = if failed { theme::bad() } else { theme::good() };
                painter.label("tool result", style, "");
                painter.body(block["content"].as_str().unwrap_or_default(), style);
            }
            "file" => {
                painter.label(
                    &format!("file · {}", block["name"].as_str().unwrap_or("file")),
                    theme::faint(),
                    "",
                );
                painter.body(block["path"].as_str().unwrap_or("-"), theme::muted());
            }
            "image" => {
                painter.label("image", theme::faint(), "");
                painter.body(block["path"].as_str().unwrap_or("-"), theme::muted());
            }
            _ => {}
        }
    }
}

/// The typed document, drawn as lines with a fixed width.
struct Painter {
    width: usize,
    lines: Vec<Line<'static>>,
    markers: Vec<Marker>,
}

impl Painter {
    fn new(width: usize) -> Self {
        Self {
            width: width.max(1),
            lines: Vec::new(),
            markers: Vec::new(),
        }
    }

    fn gap(&mut self) {
        if !self.lines.is_empty() {
            self.lines.push(Line::from(""));
        }
    }

    /// A role or layer label, with its cost against the right edge.
    fn label(&mut self, text: &str, style: Style, meta: &str) {
        self.gap();
        let mut row = Row::new();
        row.push(text.to_owned(), style);
        if !meta.is_empty() && row.width + meta.width() + BODY_INDENT <= self.width {
            row.pad_to(self.width.saturating_sub(meta.width()));
            row.push(meta.to_owned(), theme::faint());
        }
        let line = row.fit(self.width);
        self.lines.push(line);
    }

    /// One line that names what it leaves out, with its cost inline.
    fn summary(&mut self, text: &str, style: Style, cost: &str) {
        self.gap();
        let mut row = Row::new();
        row.push(text.to_owned(), style);
        row.push(cost.to_owned(), theme::faint());
        let line = row.fit(self.width);
        self.lines.push(line);
    }

    /// A wrapped body under its label, with a hanging indent.
    fn body(&mut self, text: &str, style: Style) {
        let indent = " ".repeat(BODY_INDENT);
        let inner = self.width.saturating_sub(BODY_INDENT).max(1);
        for line in wrap_text(text, inner) {
            if line.is_empty() {
                self.lines.push(Line::from(""));
            } else {
                self.lines.push(Line::styled(
                    clip(&format!("{indent}{line}"), self.width),
                    style,
                ));
            }
        }
    }

    /// The clock note that rode on the message above it.
    fn note(&mut self, note: &str) {
        let indent = " ".repeat(BODY_INDENT);
        let inner = self.width.saturating_sub(BODY_INDENT + 2).max(1);
        for (index, line) in wrap_text(note, inner).iter().enumerate() {
            let lead = if index == 0 {
                format!("{indent}◷ ")
            } else {
                format!("{indent}  ")
            };
            self.lines.push(Line::styled(
                clip(&format!("{lead}{line}"), self.width),
                theme::faint(),
            ));
        }
    }

    /// A short faint line, used when there is nothing to show.
    fn faint(&mut self, text: &str) {
        self.gap();
        let indent = " ".repeat(BODY_INDENT);
        self.lines.push(Line::styled(
            clip(&format!("{indent}{text}"), self.width),
            theme::faint(),
        ));
    }

    fn conversation(&mut self, id: &str) {
        self.gap();
        self.lines.push(Line::styled(
            clip(&format!("  conversation · {}", short(id, 8)), self.width),
            theme::faint(),
        ));
    }

    fn rewritten(&mut self, dropped: usize) {
        self.gap();
        let mut row = Row::new();
        row.push("history rewritten", theme::warn());
        row.push(
            format!(" · {dropped} earlier messages dropped"),
            theme::faint(),
        );
        let line = row.fit(self.width);
        self.lines.push(line);
    }

    /// One separator line naming the request, with the selection emphasis kept
    /// for the snapshot the reader is on.
    fn marker(
        &mut self,
        number: usize,
        at: i64,
        hash: &str,
        tokens: Option<u64>,
        changed: &[String],
        zone: &Zone,
    ) {
        self.gap();
        let mut body = vec![number.to_string(), zone.iso(at), short(hash, 8)];
        if let Some(tokens) = tokens {
            body.push(format!("{} tok", count(tokens)));
        }
        if !changed.is_empty() {
            body.push(format!("changed {}", changed.join(", ")));
        }
        let body = body.join(" · ");
        let line = self.lines.len();
        self.lines
            .push(separator("── ", &body, self.width, theme::faint()));
        let selected = separator("▸── ", &body, self.width, theme::accent_style());
        self.markers.push(Marker { line, selected });
    }
}

fn separator(lead: &str, body: &str, width: usize, style: Style) -> Line<'static> {
    let text = format!("{lead}{body} ");
    let pad = width.saturating_sub(text.width());
    Line::styled(clip(&format!("{text}{}", "─".repeat(pad)), width), style)
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

    /// A line that never draws past `width`, whatever the terminal size.
    fn fit(self, width: usize) -> Line<'static> {
        if self.width <= width {
            Line::from(self.spans)
        } else {
            let style = self.spans.last().map(|span| span.style).unwrap_or_default();
            let text = self
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            Line::styled(clip(&text, width), style)
        }
    }
}

/// Cut a line at `width` display columns.
fn clip(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for character in text.chars() {
        let character_width = character.width().unwrap_or(0);
        if used + character_width > width {
            break;
        }
        used += character_width;
        out.push(character);
    }
    out
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
            // The card border is not part of what the reader sees inside it.
            for column in 1..width.saturating_sub(1) {
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
        assert!(rendered(&app, 80, 60).contains(SOUL));
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        let hidden = rendered(&app, 80, 60);
        assert!(
            !hidden.contains(SOUL),
            "the system bodies are hidden: {hidden}"
        );
        assert!(
            hidden.contains("identity, guidance, rules, skills index, tools"),
            "the summary line names the stable layers: {hidden}"
        );
        assert!(
            !hidden.contains("MEMORY HIT"),
            "the recall body is hidden with the system layers: {hidden}"
        );
        assert!(
            hidden.contains("system · memory") && hidden.contains("70 tok"),
            "the recall keeps its one-line cost: {hidden}"
        );
        let summary = hidden
            .lines()
            .find(|line| line.contains("identity, guidance, rules, skills index, tools"))
            .expect("the summary line");
        assert!(
            summary.contains("tok"),
            "the summary names the cost: {hidden}"
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        assert!(
            rendered(&app, 80, 60).contains(SOUL),
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
            text.lines().nth(2).unwrap_or_default().contains("▸── 2 · "),
            "the older marker moves to the top of the viewport: {text}"
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        let text = rendered(&app, 100, 24);
        assert!(
            text.lines().nth(2).unwrap_or_default().contains("▸── 1 · "),
            "the oldest marker moves to the top of the viewport: {text}"
        );
        app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        let text = rendered(&app, 100, 24);
        assert!(
            text.lines().nth(2).unwrap_or_default().contains("▸── 2 · "),
            "the newer marker moves to the top of the viewport: {text}"
        );
    }

    #[test]
    fn the_header_names_the_snapshot_and_the_numbers_at_a_glance() {
        let (_home, mut app) = prompt_app(conversation());
        let text = rendered(&app, 100, 30);
        assert!(text.contains("test-model"), "{text}");
        assert!(text.contains("session session-"), "{text}");
        assert!(text.contains("snapshot 2 of 2"), "{text}");
        assert!(text.contains("620 tok"), "{text}");
        assert!(text.contains("prefix prefix-o"), "{text}");
        app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        let text = rendered(&app, 100, 30);
        assert!(text.contains("snapshot 1 of 2"), "{text}");
    }

    #[test]
    fn a_new_conversation_starts_a_fresh_transcript() {
        let mut exchanges = conversation();
        let mut newest = exchanges[0].clone();
        newest["request"]["session_id"] = json!("session-two");
        newest["request"]["transcript"] = json!([
            {"role": "user", "blocks": [{"type": "text", "text": "a different conversation"}]}
        ]);
        exchanges.insert(0, newest);
        let (_home, app) = prompt_app(exchanges);
        let text = rendered(&app, 100, 60);
        assert!(text.contains("conversation · session-"), "{text}");
        assert!(text.contains("a different conversation"), "{text}");
        assert!(
            !text.contains("history rewritten"),
            "a new conversation is not a rewritten one: {text}"
        );
        assert_eq!(text.matches(QUESTION).count(), 1, "{text}");
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
            let text = rendered(&app, width, 80);
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
        }
    }

    #[test]
    fn the_tab_leaves_the_key_legend_to_the_footer() {
        let (_home, app) = prompt_app(conversation());
        let text = rendered(&app, 120, 80);
        assert!(
            !text.contains("snapshots ·"),
            "the card must not repeat the key legend: {text}"
        );
        assert!(
            !text.contains("f follow"),
            "the footer owns the tab keys: {text}"
        );
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
