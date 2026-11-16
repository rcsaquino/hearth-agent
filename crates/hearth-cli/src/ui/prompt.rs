//! The Prompt tab as one conversation.
//!
//! The harness saves a full prompt for every request, so consecutive requests
//! differ only in their tail. This view merges the saved requests into a single
//! transcript and prints each request's delta under a bare rule. `s` hides the
//! system layers, long message blocks fold, and the turn being written is drawn
//! at the tail with its spinner on the message label.

use std::collections::BTreeSet;
use std::time::Instant;

use hearth_runtime::Zone;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, LiveReply, PromptView};
use crate::theme;
use crate::ui::{count, short};

/// Body lines moved by PageUp and PageDown.
const PAGE_LINES: usize = 15;
/// Message blocks longer than this start folded.
const COLLAPSE_LINES: usize = 12;
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
}

/// Scroll down by `lines`, resuming the live tail at the bottom.
pub(crate) fn scroll_down(prompt: &mut PromptView, lines: usize) {
    let max = prompt.max_scroll.get();
    let current = if prompt.follow { max } else { prompt.scroll };
    prompt.scroll = (current + lines).min(max);
    prompt.follow = prompt.scroll >= max;
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
}

pub(crate) fn scroll_bottom(prompt: &mut PromptView) {
    prompt.follow = true;
    prompt.scroll = prompt.max_scroll.get();
}

pub(crate) fn toggle_follow(prompt: &mut PromptView) {
    prompt.follow = !prompt.follow;
    if prompt.follow {
        prompt.scroll = prompt.max_scroll.get();
    }
}

/// Move the block cursor down one long message block.
pub(crate) fn next_block(prompt: &mut PromptView) {
    select_block(prompt, 1);
}

/// Move the block cursor up one long message block.
pub(crate) fn previous_block(prompt: &mut PromptView) {
    select_block(prompt, -1);
}

/// Fold or unfold the block under the cursor.
pub(crate) fn toggle_block(prompt: &mut PromptView) {
    let Some(key) = prompt.selected_block.clone() else {
        return;
    };
    if !prompt.expanded.remove(&key) {
        prompt.expanded.insert(key);
    }
}

fn select_block(prompt: &mut PromptView, step: isize) {
    let selection = {
        let guard = prompt.cache.borrow();
        let Some(cache) = guard.as_ref() else {
            return;
        };
        if cache.blocks.is_empty() {
            return;
        }
        let index = match prompt
            .selected_block
            .as_ref()
            .and_then(|key| cache.blocks.iter().position(|block| &block.key == key))
        {
            Some(index) if step > 0 => (index + 1).min(cache.blocks.len() - 1),
            Some(index) => index.saturating_sub(1),
            None => first_visible(prompt, cache),
        };
        (cache.blocks[index].key.clone(), cache.blocks[index].line)
    };
    prompt.selected_block = Some(selection.0);
    reveal(prompt, selection.1);
}

fn first_visible(prompt: &PromptView, cache: &PromptCache) -> usize {
    cache
        .blocks
        .iter()
        .position(|block| block.line >= prompt.scroll)
        .unwrap_or(cache.blocks.len() - 1)
}

/// Keep the selected block on screen without moving a reader who already sees it.
fn reveal(prompt: &mut PromptView, line: usize) {
    let height = prompt.viewport.get().max(1);
    let top = if prompt.follow {
        prompt.max_scroll.get()
    } else {
        prompt.scroll
    };
    prompt.scroll = if line < top {
        line
    } else if line >= top.saturating_add(height) {
        line.saturating_sub(height - 1)
    } else {
        top
    };
    prompt.follow = false;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheKey {
    /// The loaded requests, named by hash and answering entry.
    signature: String,
    show_system: bool,
    /// The long blocks the reader has opened, sorted by key.
    expanded: Vec<String>,
    width: u16,
    zone: String,
}

impl CacheKey {
    fn new(
        exchanges: &[Value],
        width: u16,
        show_system: bool,
        expanded: &BTreeSet<String>,
        zone: &Zone,
    ) -> Self {
        let mut signature = String::new();
        for exchange in exchanges {
            signature.push_str(exchange["prompt_hash"].as_str().unwrap_or("-"));
            signature.push(':');
            signature.push_str(exchange["response"]["entry_id"].as_str().unwrap_or("-"));
            signature.push('|');
        }
        Self {
            signature,
            show_system,
            expanded: expanded.iter().cloned().collect(),
            width,
            zone: zone.name().to_owned(),
        }
    }
}

/// One long message block, at the line of its label.
struct BlockSpan {
    key: String,
    line: usize,
    /// The label row in the selection emphasis.
    selected: Line<'static>,
}

/// One built document, kept until the loaded set, the open blocks, or the
/// view flags change.
pub(crate) struct PromptCache {
    key: CacheKey,
    blocks: Vec<BlockSpan>,
    lines: Vec<Line<'static>>,
}

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let title = if app.prompt.loading {
        " Prompt · refreshing "
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

    let height = usize::from(inner.height);
    let header_rows = usize::from(!app.prompt.exchanges.is_empty() && height >= 2);
    let body_rows = height.saturating_sub(header_rows);
    let header_area = Rect {
        height: header_rows as u16,
        ..inner
    };
    let body = Rect {
        y: inner.y + header_rows as u16,
        height: body_rows as u16,
        ..inner
    };

    if header_rows > 0 {
        frame.render_widget(
            Paragraph::new(header_line(app, usize::from(inner.width))),
            header_area,
        );
    }

    if app.prompt.exchanges.is_empty() && app.prompt.live.is_none() {
        let message = if app.prompt.loading {
            vec![Line::styled("Loading saved prompts...", theme::warn())]
        } else {
            vec![
                Line::styled("No session yet.", theme::muted()),
                Line::from(""),
                Line::styled(
                    "Send Hearth a message. This view refreshes after every request.",
                    theme::faint(),
                ),
            ]
        };
        frame.render_widget(Paragraph::new(message), body);
    } else if body_rows > 0 {
        draw_transcript(app, frame, body);
    }
}

/// The model and the numbers a reader needs at a glance.
fn header_line(app: &App, width: usize) -> Line<'static> {
    let Some(exchange) = app.prompt.exchanges.first() else {
        return Line::from("");
    };
    let request = &exchange["request"];
    let model = request["model"].as_str().unwrap_or("unknown model");
    let session = request["session_id"].as_str().unwrap_or("-");
    let prefix = request["prefix_hash"].as_str().unwrap_or("-");
    let mut row = Row::new();
    row.push(format!(" {model}"), theme::text());
    row.push(" · session ", theme::faint());
    row.push(short(session, 8), theme::muted());
    if let Some(tokens) = token(request, "total") {
        row.push(format!(" · {} tok", count(tokens)), theme::muted());
    }
    row.push(" · prefix ", theme::faint());
    row.push(short(prefix, 8), theme::muted());
    row.fit(width)
}

fn draw_transcript(app: &App, frame: &mut Frame, area: Rect) {
    let width = usize::from(area.width);
    let height = usize::from(area.height);
    app.prompt.viewport.set(height);
    let live = app
        .prompt
        .live
        .as_ref()
        .map(|live| live_lines(live, width))
        .unwrap_or_default();
    let key = CacheKey::new(
        &app.prompt.exchanges,
        area.width,
        app.prompt.show_system,
        &app.prompt.expanded,
        &app.zone,
    );
    let mut slot = app.prompt.cache.borrow_mut();
    if slot.as_ref().is_none_or(|cache| cache.key != key) {
        *slot = Some(build_cache(
            &app.prompt.exchanges,
            &key,
            &app.prompt.expanded,
            width,
        ));
    }
    let cache = slot.as_mut().expect("prompt cache");

    let saved = cache.lines.len();
    let total = saved + live.len();
    let max_scroll = total.saturating_sub(height);
    app.prompt.max_scroll.set(max_scroll);
    let scroll = if app.prompt.follow {
        max_scroll
    } else {
        app.prompt.scroll.min(max_scroll)
    };
    let end = (scroll + height).min(total);
    let selected = app
        .prompt
        .selected_block
        .as_ref()
        .and_then(|key| cache.blocks.iter().find(|block| &block.key == key));
    let lines: Vec<Line<'static>> = (scroll..end)
        .map(|index| {
            if index >= saved {
                return live[index - saved].clone();
            }
            match selected {
                Some(block) if block.line == index => block.selected.clone(),
                _ => cache.lines[index].clone(),
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// The turn being written, drawn below the saved document with its spinner on
/// the first label.
fn live_lines(live: &LiveReply, width: usize) -> Vec<Line<'static>> {
    let mut painter = Painter::new(width);
    painter.message_start(Role::Assistant);
    let frame = spinner_frame(live.started);
    let mut spinning = true;
    if !live.thinking.is_empty() {
        painter.live_part("thinking", theme::think(), &live.thinking, Some(frame));
        spinning = false;
    }
    if !live.text.is_empty() {
        let frame = spinning.then_some(frame);
        painter.live_part(
            Role::Assistant.label(),
            Role::Assistant.style(),
            &live.text,
            frame,
        );
        spinning = false;
    }
    if spinning {
        painter.live_label(Role::Assistant.label(), Role::Assistant.style(), frame);
    }
    painter.message_end();
    painter.lines
}

fn spinner_frame(started: Instant) -> usize {
    (started.elapsed().as_millis() / 100) as usize % FRAMES.len()
}

/// One block of a conversation, derived from the saved requests.
enum Entry {
    /// Where one saved request ends and the next begins.
    Divider,
    /// The stable layers, printed once and again only when they change.
    System { parts: Vec<SystemPart>, tokens: u64 },
    /// The memory injected for this request, before its new input.
    Recall { text: String, tokens: u64 },
    /// A different conversation starts here.
    Conversation { id: String },
    /// A message that reached the model.
    Message {
        /// Stable name for the blocks this message prints.
        key: String,
        role: Role,
        blocks: Vec<Value>,
    },
    /// The clock note that rode on the message before it.
    Runtime { note: String },
    /// Compaction dropped messages this view has already shown.
    Rewritten { dropped: usize },
    /// What the model answered.
    Response {
        key: String,
        blocks: Option<Vec<Value>>,
    },
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
        let hash = exchange["prompt_hash"].as_str().unwrap_or("-");
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
        entries.push(Entry::Divider);
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
        for (index, message) in messages.iter().enumerate().skip(kept) {
            entries.push(Entry::Message {
                key: format!("{hash}:{index}"),
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
            key: format!("{hash}:reply"),
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

fn build_cache(
    exchanges: &[Value],
    key: &CacheKey,
    expanded: &BTreeSet<String>,
    width: usize,
) -> PromptCache {
    let mut painter = Painter::new(width);
    for entry in derive(exchanges) {
        push_entry(&mut painter, &entry, key.show_system, expanded);
    }
    PromptCache {
        key: key.clone(),
        blocks: painter.blocks,
        lines: painter.lines,
    }
}

fn push_entry(
    painter: &mut Painter,
    entry: &Entry,
    show_system: bool,
    expanded: &BTreeSet<String>,
) {
    match entry {
        Entry::Divider => painter.divider(),
        Entry::System { parts, tokens } => push_system(painter, parts, *tokens, show_system),
        Entry::Recall { text, tokens } => push_recall(painter, text, *tokens, show_system),
        Entry::Conversation { id } => painter.conversation(id),
        Entry::Message { key, role, blocks } => {
            push_blocks(painter, key, *role, blocks, expanded);
        }
        Entry::Runtime { note } => painter.note(note),
        Entry::Rewritten { dropped } => painter.rewritten(*dropped),
        Entry::Response { key, blocks } => match blocks {
            Some(blocks) => push_blocks(painter, key, Role::Assistant, blocks, expanded),
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

fn push_blocks(
    painter: &mut Painter,
    key: &str,
    role: Role,
    blocks: &[Value],
    expanded: &BTreeSet<String>,
) {
    painter.message_start(role);
    if blocks.is_empty() {
        painter.label(role.label(), role.style(), "");
        painter.body("(empty)", theme::faint());
    }
    for (index, block) in blocks.iter().enumerate() {
        let key = format!("{key}/{index}");
        match block["type"].as_str().unwrap_or_default() {
            "thinking" => painter.part(
                &key,
                "thinking",
                theme::think(),
                block["text"].as_str().unwrap_or_default(),
                theme::think(),
                expanded,
            ),
            "text" => painter.part(
                &key,
                role.label(),
                role.style(),
                block["text"].as_str().unwrap_or_default(),
                role.style(),
                expanded,
            ),
            "tool_call" => {
                let name = block["name"].as_str().unwrap_or("tool");
                let arguments = block
                    .get("arguments")
                    .map(|arguments| arguments.to_string())
                    .unwrap_or_default();
                painter.part(
                    &key,
                    &format!("tool call · {name}"),
                    theme::warn(),
                    &arguments,
                    theme::muted(),
                    expanded,
                );
            }
            "tool_result" => {
                let failed = block["is_error"].as_bool().unwrap_or(false);
                let style = if failed { theme::bad() } else { theme::good() };
                painter.part(
                    &key,
                    "tool result",
                    style,
                    block["content"].as_str().unwrap_or_default(),
                    style,
                    expanded,
                );
            }
            "file" => painter.part(
                &key,
                &format!("file · {}", block["name"].as_str().unwrap_or("file")),
                theme::faint(),
                block["path"].as_str().unwrap_or("-"),
                theme::muted(),
                expanded,
            ),
            "image" => painter.part(
                &key,
                "image",
                theme::faint(),
                block["path"].as_str().unwrap_or("-"),
                theme::muted(),
                expanded,
            ),
            _ => {}
        }
    }
    painter.message_end();
}

/// The bar that opens a message and each of its parts.
const LEAD: &str = "▌ ";
/// The bar that runs down a message body.
const BODY_LEAD: &str = "▏";

/// The typed document, drawn as lines with a fixed width.
struct Painter {
    width: usize,
    lines: Vec<Line<'static>>,
    /// The long message blocks, in document order.
    blocks: Vec<BlockSpan>,
    /// The role bar while a message is open, and nothing outside one.
    gutter: Option<Style>,
    /// A blank separator is already in the document.
    gap_open: bool,
}

impl Painter {
    fn new(width: usize) -> Self {
        Self {
            width: width.max(1),
            lines: Vec::new(),
            blocks: Vec::new(),
            gutter: None,
            gap_open: false,
        }
    }

    fn message_start(&mut self, role: Role) {
        self.gap();
        self.gutter = Some(role.style());
    }

    fn message_end(&mut self) {
        self.gutter = None;
    }

    fn push(&mut self, line: Line<'static>) {
        self.lines.push(line);
        self.gap_open = false;
    }

    fn gap(&mut self) {
        if self.lines.is_empty() || self.gap_open {
            return;
        }
        match self.gutter {
            Some(style) => self.lines.push(Line::styled(BODY_LEAD, style)),
            None => self.lines.push(Line::from("")),
        }
        self.gap_open = true;
    }

    /// A role or layer label, with its cost against the right edge.
    fn label(&mut self, text: &str, style: Style, meta: &str) {
        self.gap();
        if self.gutter.is_some() {
            self.role_label(text, style);
            return;
        }
        let mut row = Row::new();
        row.push(text.to_owned(), style);
        if !meta.is_empty() && row.width + meta.width() + BODY_INDENT <= self.width {
            row.pad_to(self.width.saturating_sub(meta.width()));
            row.push(meta.to_owned(), theme::faint());
        }
        let line = row.fit(self.width);
        self.push(line);
    }

    /// A message block: its label, its body, and a stub when folded.
    fn part(
        &mut self,
        key: &str,
        label: &str,
        label_style: Style,
        text: &str,
        body_style: Style,
        expanded: &BTreeSet<String>,
    ) {
        self.gap();
        let line = self.lines.len();
        self.role_label(label, label_style);
        let inner = self.width.saturating_sub(BODY_INDENT).max(1);
        let wrapped = wrap_text(text, inner);
        let long = wrapped.len() > COLLAPSE_LINES;
        if long && !expanded.contains(key) {
            self.body_lines(&wrapped[..1], body_style);
            let hidden = wrapped.len() - 1;
            let stub = wrap_text(&format!("… {hidden} lines hidden · enter to open"), inner);
            self.body_lines(&stub, theme::faint());
        } else {
            self.body_lines(&wrapped, body_style);
        }
        if long {
            self.blocks.push(BlockSpan {
                key: key.to_owned(),
                line,
                selected: self.selected_label(label),
            });
        }
    }

    /// One part of the live turn. The spinner replaces the role bar on the
    /// label that is being written.
    fn live_part(&mut self, label: &str, style: Style, text: &str, frame: Option<usize>) {
        self.gap();
        match frame {
            Some(frame) => self.live_label(label, style, frame),
            None => self.role_label(label, style),
        }
        let inner = self.width.saturating_sub(BODY_INDENT).max(1);
        let wrapped = wrap_text(text, inner);
        self.body_lines(&wrapped, style);
    }

    fn live_label(&mut self, label: &str, style: Style, frame: usize) {
        let lead = self.gutter.unwrap_or(style);
        let body = clip(label, self.width.saturating_sub(BODY_INDENT));
        self.push(Line::from(vec![
            Span::styled(format!("{} ", FRAMES[frame]), lead),
            Span::styled(body, style),
        ]));
    }

    /// The bar and label that open a message part.
    fn role_label(&mut self, text: &str, style: Style) {
        let lead = self.gutter.unwrap_or(style);
        let body = clip(text, self.width.saturating_sub(BODY_INDENT));
        self.push(Line::from(vec![
            Span::styled(LEAD.to_owned(), lead),
            Span::styled(body, style),
        ]));
    }

    /// The same label row drawn in the selection emphasis.
    fn selected_label(&self, text: &str) -> Line<'static> {
        let body = clip(text, self.width.saturating_sub(BODY_INDENT));
        Line::styled(
            clip(&format!("{LEAD}{body}"), self.width),
            theme::selected(),
        )
    }

    /// One line that names what it leaves out, with its cost inline.
    fn summary(&mut self, text: &str, style: Style, cost: &str) {
        self.gap();
        let mut row = Row::new();
        row.push(text.to_owned(), style);
        row.push(cost.to_owned(), theme::faint());
        let line = row.fit(self.width);
        self.push(line);
    }

    /// A wrapped body under its label, with a hanging indent.
    fn body(&mut self, text: &str, style: Style) {
        let inner = self.width.saturating_sub(BODY_INDENT).max(1);
        let lines = wrap_text(text, inner);
        self.body_lines(&lines, style);
    }

    fn body_lines(&mut self, lines: &[String], style: Style) {
        let gutter = self.gutter;
        let indent = " ".repeat(BODY_INDENT);
        for line in lines {
            match gutter {
                Some(bar) => {
                    let body = clip(line, self.width.saturating_sub(BODY_INDENT));
                    self.push(Line::from(vec![
                        Span::styled(format!("{BODY_LEAD} "), bar),
                        Span::styled(body, style),
                    ]));
                }
                None if line.is_empty() => self.push(Line::from("")),
                None => self.push(Line::styled(
                    clip(&format!("{indent}{line}"), self.width),
                    style,
                )),
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
            self.push(Line::styled(
                clip(&format!("{lead}{line}"), self.width),
                theme::faint(),
            ));
        }
    }

    /// A short faint line, used when there is nothing to show.
    fn faint(&mut self, text: &str) {
        self.gap();
        let indent = " ".repeat(BODY_INDENT);
        self.push(Line::styled(
            clip(&format!("{indent}{text}"), self.width),
            theme::faint(),
        ));
    }

    fn conversation(&mut self, id: &str) {
        self.gap();
        self.push(Line::styled(
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
        self.push(line);
    }

    /// A bare rule where one saved request ends and the next begins.
    fn divider(&mut self) {
        self.gap();
        self.push(Line::styled("─".repeat(self.width), theme::faint()));
    }
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
    use crate::app::{App, AppEvent, Screen, Tab};
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

    /// A body of `lines` distinct lines, named so tests can find one.
    fn block_body(name: &str, lines: usize) -> String {
        (0..lines)
            .map(|index| format!("{name} line {index}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_long_block_collapses_to_a_stub_and_opens_with_enter() {
        let body = block_body("BODY", 40);
        let identity = block_body("IDENTITY", 40);
        let mut exchanges = vec![json!({
            "request": request(
                json!([{"role": "user", "blocks": [{"type": "text", "text": body}]}]),
                500
            ),
            "prompt_hash": "hash-collapse",
            "requested_at": 1,
            "response": Value::Null,
            "usage": Value::Null
        })];
        exchanges[0]["request"]["identity"] = json!(identity);
        let (_home, mut app) = prompt_app(exchanges);
        let _ = rendered(&app, 100, 40);
        let lines = cache_lines(&app);
        assert!(
            lines.iter().any(|line| line.contains("BODY line 0")),
            "a collapsed block keeps its first line as a preview: {lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("BODY line 39")),
            "the long body is collapsed: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("hidden") && line.contains("enter")),
            "the stub names the hidden lines and the key: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("IDENTITY line 39")),
            "a system layer is not a message block: {lines:?}"
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let _ = rendered(&app, 100, 80);
        let lines = cache_lines(&app);
        assert!(
            lines.iter().any(|line| line.contains("BODY line 39")),
            "enter opens the selected block: {lines:?}"
        );
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let _ = rendered(&app, 100, 40);
        let lines = cache_lines(&app);
        assert!(
            !lines.iter().any(|line| line.contains("BODY line 39")),
            "enter closes it again: {lines:?}"
        );
    }

    #[test]
    fn the_block_cursor_is_highlighted() {
        let alpha = block_body("ALPHA", 30);
        let beta = block_body("BETA", 30);
        let exchanges = vec![json!({
            "request": request(
                json!([
                    {"role": "user", "blocks": [{"type": "text", "text": alpha}]},
                    {"role": "user", "blocks": [{"type": "text", "text": beta}]}
                ]),
                500
            ),
            "prompt_hash": "hash-highlight",
            "requested_at": 1,
            "response": Value::Null,
            "usage": Value::Null
        })];
        let (_home, mut app) = prompt_app(exchanges);
        let _ = rendered(&app, 100, 40);
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        let backend = TestBackend::new(100, 80);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                let area = frame.area();
                render(&app, frame, area);
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let labels: Vec<u16> = (0..80)
            .filter(|row| {
                let text: String = (1..99)
                    .map(|column| buffer[(column, *row)].symbol())
                    .collect();
                text.starts_with("▌ you")
            })
            .collect();
        assert_eq!(labels.len(), 2, "both long blocks keep their label");
        assert_eq!(
            buffer[(3, labels[0])].style().bg,
            Some(theme::SELECT_BG),
            "the first long block is highlighted after n"
        );
        assert_ne!(
            buffer[(3, labels[1])].style().bg,
            Some(theme::SELECT_BG),
            "only the selected block is highlighted"
        );
    }

    #[test]
    fn n_and_p_move_the_block_cursor() {
        let alpha = block_body("ALPHA", 30);
        let beta = block_body("BETA", 30);
        let exchanges = vec![json!({
            "request": request(
                json!([
                    {"role": "user", "blocks": [{"type": "text", "text": alpha}]},
                    {"role": "user", "blocks": [{"type": "text", "text": beta}]}
                ]),
                500
            ),
            "prompt_hash": "hash-cursor",
            "requested_at": 1,
            "response": Value::Null,
            "usage": Value::Null
        })];
        let (_home, mut app) = prompt_app(exchanges);
        let _ = rendered(&app, 100, 40);
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let _ = rendered(&app, 100, 40);
        let lines = cache_lines(&app);
        assert!(
            lines.iter().any(|line| line.contains("ALPHA line 29")),
            "the first long block opens: {lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("BETA line 29")),
            "the second block stays closed: {lines:?}"
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let _ = rendered(&app, 100, 40);
        let lines = cache_lines(&app);
        assert!(
            lines.iter().any(|line| line.contains("BETA line 29")),
            "n reaches the next long block: {lines:?}"
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let _ = rendered(&app, 100, 40);
        let lines = cache_lines(&app);
        assert!(
            !lines.iter().any(|line| line.contains("ALPHA line 29")),
            "p returns to the first block and enter closes it: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("BETA line 29")),
            "the second block stays open: {lines:?}"
        );
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
        let user = starting(&text, "▌ you");
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
            starting(&text, "▌ hearth").len() == 1,
            "the answer is labelled as Hearth: {text}"
        );
    }

    #[test]
    fn a_message_is_enclosed_by_a_role_bar() {
        let (_home, app) = prompt_app(vec![json!({
            "request": request(
                json!([
                    {"role": "user", "blocks": [{"type": "text", "text": "FIRST QUESTION"}]},
                    {"role": "assistant", "blocks": [{"type": "text", "text": "FIRST ANSWER"}]},
                    {"role": "user", "blocks": [{"type": "text", "text": "SECOND QUESTION"}]}
                ]),
                500
            ),
            "prompt_hash": "hash-bar",
            "requested_at": 1,
            "response": {
                "entry_id": "entry-bar",
                "created_at": 2,
                "blocks": [
                    {"type": "thinking", "text": "A LONG THOUGHT"},
                    {"type": "text", "text": "SECOND ANSWER"}
                ],
                "tokens_est": 3
            },
            "usage": Value::Null
        })]);
        let text = rendered(&app, 100, 60);
        let row = |needle: &str| -> String {
            text.lines()
                .find(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("missing {needle}: {text}"))
                .to_owned()
        };
        assert!(row("▌ you").starts_with("▌ you"), "{text}");
        assert!(row("FIRST QUESTION").starts_with("▏ "), "{text}");
        assert!(row("FIRST ANSWER").starts_with("▏ "), "{text}");
        assert!(row("▌ thinking").starts_with("▌ thinking"), "{text}");
        assert!(row("A LONG THOUGHT").starts_with("▏ "), "{text}");
        assert!(row("▌ hearth").starts_with("▌ hearth"), "{text}");
        assert!(row("SECOND ANSWER").starts_with("▏ "), "{text}");
        let system = row(SOUL);
        assert!(
            !system.starts_with('▌') && !system.starts_with('▏'),
            "a system layer is not a message: {system:?}"
        );

        let thought = line_of(&text, "A LONG THOUGHT");
        let answer = line_of(&text, "SECOND ANSWER");
        assert!(
            text.lines()
                .skip(thought + 1)
                .take(answer - thought - 1)
                .any(|line| line.trim_end() == "▏"),
            "the bar continues across two parts of one message: {text}"
        );

        let first_answer = line_of(&text, "FIRST ANSWER");
        let second_label = text
            .lines()
            .enumerate()
            .skip(first_answer)
            .find(|(_, line)| line.starts_with("▌ you"))
            .map(|(index, _)| index)
            .expect("the second question keeps its label");
        assert_eq!(
            text.lines()
                .nth(second_label - 1)
                .unwrap_or_default()
                .trim(),
            "",
            "a blank line separates one message from the next: {text}"
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
    fn saved_requests_are_separated_without_snapshot_vocabulary() {
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
        let question = line_of(&text, QUESTION);
        let answer = line_of(&text, ANSWER);
        let rule = text
            .lines()
            .enumerate()
            .find(|(index, line)| {
                *index > question
                    && *index < answer
                    && !line.trim().is_empty()
                    && line.trim().chars().all(|character| character == '─')
            })
            .map(|(index, _)| index);
        assert!(
            rule.is_some(),
            "a bare rule marks where the next request begins: {text}"
        );
        assert!(!text.contains("snapshot"), "no snapshot vocabulary: {text}");
        assert!(
            !text.contains("aaaaaa") && !text.contains("bbbbbb"),
            "no request hashes: {text}"
        );
        assert!(
            !text.contains("22:13:20") && !text.contains("22:13:30"),
            "no marker stamps: {text}"
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
        let first = starting(&text, "▌ you")[0];
        let note = line_of(&text, "CLOCK NOTE seven seconds past");
        let call = line_of(&text, "tool call · read_file");
        assert!(
            first < note && note < call,
            "the note sits under the question it rode with: {text}"
        );
        let result = starting(&text, "▌ tool result")[0];
        let second = line_of(&text, "CLOCK NOTE eight seconds past");
        let answer = line_of(&text, ANSWER);
        assert!(
            result < second && second < answer,
            "the note sits under the tool result it rode with: {text}"
        );
    }

    /// True when a line opens with one of the spinner frames and `label`.
    fn spun(text: &str, label: &str) -> bool {
        text.lines().any(|line| {
            FRAMES
                .iter()
                .any(|frame| line.starts_with(&format!("{frame} {label}")))
        })
    }

    #[test]
    fn the_live_turn_is_drawn_after_the_saved_answer() {
        let (_home, mut app) = prompt_app(conversation());
        app.apply_event(AppEvent::Live(json!({
            "kind": "turn_started",
            "data": {"turn": 1}
        })));
        app.apply_event(AppEvent::Live(json!({
            "kind": "text_delta",
            "data": {"text": "STREAMING REPLY"}
        })));
        let text = rendered(&app, 100, 60);
        let answer = line_of(&text, ANSWER);
        let live = line_of(&text, "STREAMING REPLY");
        assert!(
            live > answer,
            "the live reply sits after the saved answer: {text}"
        );
        assert!(
            text.lines().skip(live).any(|line| line.starts_with("▏ ")),
            "the live body keeps the message bar: {text}"
        );
        assert!(
            spun(&text, "hearth"),
            "the spinner sits on the live message label: {text}"
        );
        assert!(
            !text.contains("working"),
            "the detached working line is gone: {text}"
        );
    }

    #[test]
    fn the_live_turn_shows_its_thinking_before_its_text() {
        let (_home, mut app) = prompt_app(conversation());
        app.apply_event(AppEvent::Live(json!({
            "kind": "turn_started",
            "data": {"turn": 1}
        })));
        app.apply_event(AppEvent::Live(json!({
            "kind": "thinking_delta",
            "data": {"text": "WEIGHING IT"}
        })));
        app.apply_event(AppEvent::Live(json!({
            "kind": "text_delta",
            "data": {"text": "STREAMING REPLY"}
        })));
        let text = rendered(&app, 100, 60);
        assert!(
            line_of(&text, "WEIGHING IT") < line_of(&text, "STREAMING REPLY"),
            "thinking comes before the text it precedes: {text}"
        );
        assert!(
            spun(&text, "thinking"),
            "the spinner marks the first live part: {text}"
        );
        assert_eq!(
            text.matches("▌ hearth").count(),
            2,
            "the saved answer and the live text both carry the role label: {text}"
        );
    }

    #[test]
    fn a_new_turn_drops_the_previous_live_text() {
        let (_home, mut app) = prompt_app(conversation());
        app.apply_event(AppEvent::Live(json!({
            "kind": "text_delta",
            "data": {"text": "OLD STREAM"}
        })));
        app.apply_event(AppEvent::Live(json!({
            "kind": "turn_started",
            "data": {"turn": 2}
        })));
        app.apply_event(AppEvent::Live(json!({
            "kind": "text_delta",
            "data": {"text": "NEW STREAM"}
        })));
        let text = rendered(&app, 100, 60);
        assert!(
            !text.contains("OLD STREAM"),
            "a new turn does not inherit the last turn's text: {text}"
        );
        assert!(text.contains("NEW STREAM"), "{text}");
    }

    #[test]
    fn a_settled_run_removes_the_live_text() {
        let (_home, mut app) = prompt_app(conversation());
        app.apply_event(AppEvent::Live(json!({
            "kind": "text_delta",
            "data": {"text": "STREAMING REPLY"}
        })));
        assert!(
            rendered(&app, 100, 60).contains("STREAMING REPLY"),
            "the partial reply is visible while it streams"
        );
        app.apply_event(AppEvent::Live(json!({
            "kind": "run_settled",
            "data": {"status": "done"}
        })));
        assert!(
            !rendered(&app, 100, 60).contains("STREAMING REPLY"),
            "the saved document replaces the live turn"
        );
    }

    #[test]
    fn a_refresh_alone_does_not_animate_working() {
        let (_home, mut app) = prompt_app(conversation());
        app.prompt.loading = true;
        let text = rendered(&app, 100, 24);
        assert!(!text.contains("working"), "a reload is not a turn: {text}");
        assert!(
            !spun(&text, "hearth"),
            "a reload draws no spinner on a message: {text}"
        );
        app.prompt.loading = false;
        app.status = Some(json!({"tasks_running": 2}));
        let text = rendered(&app, 100, 24);
        assert!(
            !text.contains("working"),
            "a scheduled task is not this conversation's turn: {text}"
        );
        assert!(app.prompt.follow, "a status poll must not move the reader");
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
    fn the_header_names_the_model_session_and_numbers() {
        let (_home, app) = prompt_app(conversation());
        let text = rendered(&app, 100, 30);
        assert!(text.contains("test-model"), "{text}");
        assert!(text.contains("session session-"), "{text}");
        assert!(text.contains("620 tok"), "{text}");
        assert!(text.contains("prefix prefix-o"), "{text}");
        assert!(
            !text.contains("snapshot"),
            "the header keeps no snapshot count: {text}"
        );
    }

    #[test]
    fn the_header_carries_the_newest_request_when_it_arrives() {
        let (_home, mut app) = prompt_app(conversation());
        let text = rendered(&app, 100, 30);
        assert!(text.contains("620 tok"), "{text}");
        app.prompt.exchanges[0]["request"]["tokens"]["total"] = json!(999);
        let text = rendered(&app, 100, 30);
        assert!(
            text.contains("999 tok"),
            "the header follows the newest saved request: {text}"
        );
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
