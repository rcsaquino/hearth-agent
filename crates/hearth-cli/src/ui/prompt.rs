//! The Prompt tab as a chat window.
//!
//! The harness saves a full prompt for every request, so consecutive requests
//! differ only in their tail. This view merges the saved requests into one chat:
//! every context layer, recall, message, and answer becomes a block with a
//! banded header, a gutted body, and one fold. `n`/`p` walk the blocks, Enter
//! folds the one under the cursor, and `e`/`c` open or close them all. The turn
//! being written is drawn below the cached document with its spinner on the
//! header.

use std::collections::BTreeMap;
use std::time::Instant;

use hearth_runtime::Zone;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as Card, Borders, Paragraph};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Folds, LiveReply, PromptView};
use crate::theme;
use crate::ui::{count, short};

/// Body lines moved by PageUp and PageDown.
const PAGE_LINES: usize = 15;
/// The bar and the space a block body sits behind.
const BODY_INDENT: usize = 2;
/// One frame per 100 ms, which is the event loop's budget.
const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

/// One stable prompt layer: its hash in `prefix_parts`, its saved text, its
/// token key, and the name a reader knows it by.
struct Layer {
    part: &'static str,
    text: &'static str,
    tokens: &'static str,
    label: &'static str,
}

/// The prefix layers in the order the model reads them.
const LAYERS: [Layer; 5] = [
    Layer {
        part: "identity",
        text: "identity",
        tokens: "identity",
        label: "SOUL.md",
    },
    Layer {
        part: "guidance",
        text: "guidance",
        tokens: "guidance",
        label: "guidance",
    },
    Layer {
        part: "rules",
        text: "rules",
        tokens: "rules",
        label: "rules",
    },
    Layer {
        part: "skills",
        text: "skills_index",
        tokens: "skills",
        label: "skills index",
    },
    Layer {
        part: "tools",
        text: "tools",
        tokens: "tools",
        label: "tools",
    },
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

/// Open every block, and every block that arrives after this key.
pub(crate) fn expand_all(prompt: &mut PromptView) {
    prompt.folds.open_all();
}

/// Fold every block down to its header.
pub(crate) fn collapse_all(prompt: &mut PromptView) {
    prompt.folds.close_all();
}

/// Move the cursor to the next block.
pub(crate) fn next_block(prompt: &mut PromptView) {
    let Some((key, line)) = step(prompt, true) else {
        return;
    };
    prompt.selected = Some(key);
    reveal(prompt, line);
}

/// Move the cursor to the previous block.
pub(crate) fn previous_block(prompt: &mut PromptView) {
    let Some((key, line)) = step(prompt, false) else {
        return;
    };
    prompt.selected = Some(key);
    reveal(prompt, line);
}

/// Fold or open the block under the cursor, from the fold it was built with.
pub(crate) fn toggle_block(prompt: &mut PromptView) {
    let Some(key) = prompt.selected.clone() else {
        return;
    };
    let default = {
        let guard = prompt.cache.borrow();
        let Some(cache) = guard.as_ref() else {
            return;
        };
        match cache.blocks.iter().find(|block| block.key == key) {
            Some(block) => block.default_open,
            None => return,
        }
    };
    prompt.folds.toggle(&key, default);
}

/// The block the cursor moves to: the neighbour of the selection, or the edge
/// of the visible window when nothing is selected.
fn step(prompt: &PromptView, forward: bool) -> Option<(String, usize)> {
    let guard = prompt.cache.borrow();
    let cache = guard.as_ref()?;
    if cache.blocks.is_empty() {
        return None;
    }
    let selected = prompt
        .selected
        .as_ref()
        .and_then(|key| cache.blocks.iter().position(|block| &block.key == key));
    let index = match (selected, forward) {
        (Some(index), true) => (index + 1).min(cache.blocks.len() - 1),
        (Some(index), false) => index.saturating_sub(1),
        (None, true) => first_visible(prompt, cache),
        (None, false) => last_visible(prompt, cache),
    };
    let block = &cache.blocks[index];
    Some((block.key.clone(), block.line))
}

fn viewport_top(prompt: &PromptView) -> usize {
    if prompt.follow {
        prompt.max_scroll.get()
    } else {
        prompt.scroll
    }
}

fn first_visible(prompt: &PromptView, cache: &PromptCache) -> usize {
    let top = viewport_top(prompt);
    cache
        .blocks
        .iter()
        .position(|block| block.line >= top)
        .unwrap_or(cache.blocks.len() - 1)
}

fn last_visible(prompt: &PromptView, cache: &PromptCache) -> usize {
    let bottom = viewport_top(prompt).saturating_add(prompt.viewport.get().max(1) - 1);
    cache
        .blocks
        .iter()
        .rposition(|block| block.line <= bottom)
        .unwrap_or(0)
}

/// Bring a block header on screen without moving a reader who already sees it.
fn reveal(prompt: &mut PromptView, line: usize) {
    let height = prompt.viewport.get().max(1);
    let top = viewport_top(prompt);
    if line < top {
        prompt.scroll = line;
        prompt.follow = false;
    } else if line >= top.saturating_add(height) {
        prompt.scroll = line.saturating_sub(height - 1);
        prompt.follow = false;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheKey {
    /// The loaded requests, named by hash and answering entry.
    signature: String,
    folds: Folds,
    width: u16,
    zone: String,
}

impl CacheKey {
    fn new(exchanges: &[Value], folds: &Folds, width: u16, zone: &Zone) -> Self {
        let mut signature = String::new();
        for exchange in exchanges {
            signature.push_str(exchange["prompt_hash"].as_str().unwrap_or("-"));
            signature.push(':');
            signature.push_str(exchange["response"]["entry_id"].as_str().unwrap_or("-"));
            signature.push('|');
        }
        Self {
            signature,
            folds: folds.clone(),
            width,
            zone: zone.name().to_owned(),
        }
    }
}

/// One block header, at its line in the cached document.
struct BlockSpan {
    key: String,
    line: usize,
    /// The fold this block was built with, so Enter toggles the right way.
    default_open: bool,
    /// The header row in the selection emphasis.
    selected: Line<'static>,
}

/// One built chat, kept until the loaded set, the folds, the width, or the
/// zone change.
pub(crate) struct PromptCache {
    key: CacheKey,
    summary: Summary,
    blocks: Vec<BlockSpan>,
    lines: Vec<Line<'static>>,
}

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let title = if app.prompt.loading {
        " Prompt · chat · refreshing "
    } else if app.prompt.follow {
        " Prompt · chat · live "
    } else {
        " Prompt · chat · paused "
    };
    let card = Card::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(theme::heading(title));
    let inner = card.inner(area);
    frame.render_widget(card, area);

    let width = usize::from(inner.width);
    let height = usize::from(inner.height);

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
        frame.render_widget(Paragraph::new(message), inner);
        return;
    }

    let key = CacheKey::new(
        &app.prompt.exchanges,
        &app.prompt.folds,
        inner.width,
        &app.zone,
    );
    let mut slot = app.prompt.cache.borrow_mut();
    if slot.as_ref().is_none_or(|cache| cache.key != key) {
        *slot = Some(build_cache(
            &app.prompt.exchanges,
            &key,
            &app.prompt.folds,
            width,
            &app.zone,
        ));
    }
    let cache = slot.as_mut().expect("prompt cache");

    // The header is drawn after the cache so it can name what the chat holds.
    let header_rows = usize::from(!app.prompt.exchanges.is_empty() && height >= 2);
    let body_rows = height.saturating_sub(header_rows);
    if header_rows > 0 {
        let row = Rect { height: 1, ..inner };
        frame.render_widget(Paragraph::new(status_line(&cache.summary, width)), row);
    }
    if body_rows == 0 {
        return;
    }
    let body = Rect {
        y: inner.y + header_rows as u16,
        height: body_rows as u16,
        ..inner
    };

    let live = app
        .prompt
        .live
        .as_ref()
        .map(|live| live_lines(live, width))
        .unwrap_or_default();
    let saved = cache.lines.len();
    let total = saved + live.len();
    let max_scroll = total.saturating_sub(body_rows);
    app.prompt.max_scroll.set(max_scroll);
    app.prompt.viewport.set(body_rows);
    let scroll = if app.prompt.follow {
        max_scroll
    } else {
        app.prompt.scroll.min(max_scroll)
    };
    let end = (scroll + body_rows).min(total);
    let selected = app
        .prompt
        .selected
        .as_deref()
        .and_then(|key| cache.blocks.iter().find(|block| block.key == key))
        .map(|block| block.line);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(end.saturating_sub(scroll));
    for index in scroll..end {
        if index >= saved {
            lines.push(live[index - saved].clone());
        } else if Some(index) == selected {
            let block = cache
                .blocks
                .iter()
                .find(|block| block.line == index)
                .expect("a selected block has a line");
            lines.push(block.selected.clone());
        } else {
            lines.push(cache.lines[index].clone());
        }
    }
    frame.render_widget(Paragraph::new(lines), body);
}

/// The model and the numbers a reader needs at a glance.
fn status_line(summary: &Summary, width: usize) -> Line<'static> {
    let model = if summary.model.is_empty() {
        "unknown model"
    } else {
        summary.model.as_str()
    };
    let mut row = Row::new();
    row.push(format!(" {model}"), theme::text());
    let mut tail = vec![format!(
        " · {} {} · {} {}",
        summary.turns,
        if summary.turns == 1 { "turn" } else { "turns" },
        summary.messages,
        if summary.messages == 1 {
            "message"
        } else {
            "messages"
        },
    )];
    if let Some(tokens) = summary.tokens {
        tail.push(format!(" · {} tok", count(tokens)));
    }
    if !summary.session.is_empty() {
        tail.push(format!(" · session {}", short(&summary.session, 8)));
    }
    if !summary.prefix.is_empty() {
        tail.push(format!(" · prefix {}", short(&summary.prefix, 8)));
    }
    // A narrow terminal drops the cost and the identifiers before the model.
    for text in tail {
        if row.width + text.width() > width {
            break;
        }
        row.push(text, theme::muted());
    }
    row.fit(width)
}

/// The turn being written, drawn below the saved chat with its spinner on the
/// header and its thinking before its text.
fn live_lines(live: &LiveReply, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from("")];
    let frame = FRAMES[spinner_frame(live.started)];
    lines.push(band_line(
        vec![
            Span::styled(format!("{frame} "), theme::accent_style()),
            Span::styled(
                "HEARTH".to_owned(),
                theme::accent_style().add_modifier(Modifier::BOLD),
            ),
        ],
        "streaming",
        theme::faint(),
        width,
        Style::default(),
    ));
    if !live.thinking.is_empty() {
        lines.extend(body_block(
            "thinking",
            &live.thinking,
            theme::THINK,
            theme::THINK,
            None,
            width,
        ));
    }
    if !live.text.is_empty() {
        lines.extend(body_block(
            "",
            &live.text,
            theme::ACCENT,
            theme::TEXT,
            None,
            width,
        ));
    }
    lines
}

/// A sub-panel label and its wrapped body, as one block's rows.
fn body_block(
    label: &str,
    text: &str,
    accent: Color,
    fg: Color,
    bg: Option<Color>,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if !label.is_empty() {
        lines.push(label_line(label, accent, bg, width));
    }
    let inner = width.saturating_sub(BODY_INDENT).max(1);
    lines.extend(
        wrap_text(text, inner)
            .into_iter()
            .map(|line| body_line(&line, accent, fg, bg, width)),
    );
    lines
}

fn spinner_frame(started: Instant) -> usize {
    (started.elapsed().as_millis() / 100) as usize % FRAMES.len()
}

/// The merged chat and the numbers its header names.
#[derive(Default)]
struct Summary {
    model: String,
    session: String,
    prefix: String,
    tokens: Option<u64>,
    turns: usize,
    messages: usize,
}

#[derive(Default)]
struct Chat {
    summary: Summary,
    entries: Vec<Entry>,
}

/// One printed item of the chat.
enum Entry {
    /// A boundary, always one line.
    Notice(Notice),
    /// A foldable block.
    Block(Block),
}

enum Notice {
    /// A different conversation starts here.
    Conversation(String),
    /// Compaction dropped messages this view has already shown.
    Rewritten(usize),
    /// The request has no saved answer.
    NoReply,
}

/// How a block folds until the reader says otherwise.
#[derive(Clone, Copy)]
enum Policy {
    Open,
    Closed,
    /// Open when the wrapped body has at most this many lines.
    Fit(usize),
}

/// One run of a block body.
struct Part {
    /// A sub-panel label, such as a tool call or a result inside another block.
    label: Option<String>,
    text: String,
}

/// One block of the chat, derived from the saved requests.
struct Block {
    /// Stable while its source request is loaded.
    key: String,
    /// The header label, uppercase.
    label: String,
    /// The right-aligned header meta, dropped first when the terminal is narrow.
    meta: String,
    /// The clock note that rode on this message. It stays visible when folded.
    note: Option<String>,
    parts: Vec<Part>,
    fold: Policy,
    /// Show the first body line while folded.
    preview: bool,
    /// The background band behind the header and body.
    bg: Option<Color>,
    /// The gutter and sub-panel label colour.
    accent: Color,
    /// The body text colour.
    fg: Color,
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
}

/// Merge the newest-first exchanges into one chat, oldest request first.
fn derive(exchanges: &[Value], zone: &Zone) -> Chat {
    let mut chat = Chat::default();
    let mut previous_sent: Vec<(String, String)> = Vec::new();
    let mut previous_parts: Option<Value> = None;
    let mut previous_session: Option<String> = None;
    let mut previous_recall: Option<String> = None;
    let mut call_names: BTreeMap<String, String> = BTreeMap::new();
    for exchange in exchanges.iter().rev() {
        chat.summary.turns += 1;
        let turn = chat.summary.turns;
        let request = &exchange["request"];
        let at = exchange["requested_at"].as_i64().unwrap_or(0);
        let hash = exchange["prompt_hash"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("@{at}"));
        let clock = zone.local(at).clock();
        let session = request["session_id"].as_str().unwrap_or("-").to_owned();
        if previous_session.as_ref() != Some(&session) {
            if previous_session.is_some() {
                chat.entries
                    .push(Entry::Notice(Notice::Conversation(session.clone())));
            }
            previous_sent.clear();
            previous_parts = None;
            previous_recall = None;
            call_names.clear();
        }
        previous_session = Some(session);

        // Every layer on the first request of a session, only the changed ones
        // after that.
        let changed: Vec<&Layer> = match previous_parts.as_ref() {
            None => LAYERS.iter().collect(),
            Some(previous) => LAYERS
                .iter()
                .filter(|layer| previous[layer.part] != request["prefix_parts"][layer.part])
                .collect(),
        };
        for layer in changed {
            let text = layer_text(request, layer);
            if text.trim().is_empty() {
                continue;
            }
            chat.entries.push(Entry::Block(Block {
                key: format!("{hash}:ctx:{}", layer.part),
                label: format!("SYSTEM · {}", layer.label),
                meta: cost(token(request, layer.tokens)),
                note: None,
                parts: vec![Part { label: None, text }],
                fold: Policy::Closed,
                preview: false,
                bg: Some(theme::CTX_BG),
                accent: theme::MUTED,
                fg: theme::MUTED,
            }));
        }
        previous_parts = Some(request["prefix_parts"].clone());

        let recall = request["recall"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_owned();
        if !recall.is_empty() && previous_recall.as_deref() != Some(recall.as_str()) {
            chat.entries.push(Entry::Block(Block {
                key: format!("{hash}:memory"),
                label: "MEMORY".to_owned(),
                meta: cost(token(request, "recall")),
                note: None,
                parts: vec![Part {
                    label: None,
                    text: recall.clone(),
                }],
                fold: Policy::Fit(6),
                preview: false,
                bg: Some(theme::CTX_BG),
                accent: theme::GOOD,
                fg: theme::MUTED,
            }));
            previous_recall = Some(recall);
        }

        let messages = request["transcript"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for message in &messages {
            remember_calls(&mut call_names, message);
        }
        let transcript: Vec<(String, String)> = messages.iter().map(wire_message).collect();
        let kept = common_prefix(&transcript, &previous_sent);
        if kept < previous_sent.len() {
            chat.entries
                .push(Entry::Notice(Notice::Rewritten(previous_sent.len() - kept)));
        }
        for (index, message) in messages.iter().enumerate().skip(kept) {
            let note = message["runtime_note"]
                .as_str()
                .filter(|note| !note.trim().is_empty())
                .map(str::to_owned);
            let role = Role::parse(message["role"].as_str().unwrap_or_default());
            let blocks = message["blocks"].as_array().cloned().unwrap_or_default();
            chat.entries.push(Entry::Block(message_block(
                format!("{hash}:m{index}"),
                role,
                &blocks,
                &call_names,
                note,
                turn,
                &clock,
            )));
            chat.summary.messages += 1;
        }

        let response = &exchange["response"];
        if response.is_null() {
            chat.entries.push(Entry::Notice(Notice::NoReply));
        } else {
            let blocks = response["blocks"].as_array().cloned().unwrap_or_default();
            let answered_at = response["created_at"].as_i64().unwrap_or(at);
            chat.entries.push(Entry::Block(response_block(
                format!("{hash}:reply"),
                &blocks,
                &call_names,
                turn,
                &zone.local(answered_at).clock(),
            )));
            chat.summary.messages += 1;
            remember_calls(&mut call_names, response);
        }

        previous_sent = transcript;
        if let Some(wire) = wire_form(response) {
            previous_sent.push(wire);
        }
    }

    if let Some(exchange) = exchanges.first() {
        let request = &exchange["request"];
        chat.summary.model = request["model"].as_str().unwrap_or_default().to_owned();
        chat.summary.session = request["session_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        chat.summary.prefix = request["prefix_hash"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        chat.summary.tokens = token(request, "total");
    }
    chat
}

/// One message as the next request carries it, as a role and its blocks.
fn wire_message(message: &Value) -> (String, String) {
    let role = message["role"].as_str().unwrap_or_default().to_owned();
    let blocks = message
        .get("blocks")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    (role, blocks.to_string())
}

/// The answer as the next request carries it.
///
/// A reading of the saved JSON that mirrors `hearth_core::prompt::carries_request`:
/// the CLI reads blobs, not provider blocks, so it keeps the rule locally rather
/// than depending on core for one predicate. Thinking is never sent back, and a
/// turn that asked nothing was never written as an entry.
fn wire_form(response: &Value) -> Option<(String, String)> {
    let blocks: Vec<Value> = response["blocks"]
        .as_array()?
        .iter()
        .filter(|block| block["type"] != "thinking")
        .cloned()
        .collect();
    let asked = blocks.iter().any(|block| match block["type"].as_str() {
        Some("thinking") => false,
        Some("text") => block["text"]
            .as_str()
            .is_some_and(|text| !text.trim().is_empty()),
        _ => true,
    });
    if !asked {
        return None;
    }
    Some(("assistant".to_owned(), Value::Array(blocks).to_string()))
}

fn common_prefix(current: &[(String, String)], previous: &[(String, String)]) -> usize {
    current
        .iter()
        .zip(previous)
        .take_while(|(now, before)| now == before)
        .count()
}

fn remember_calls(calls: &mut BTreeMap<String, String>, message: &Value) {
    for block in message["blocks"].as_array().into_iter().flatten() {
        if block["type"] != "tool_call" {
            continue;
        }
        if let (Some(id), Some(name)) = (block["id"].as_str(), block["name"].as_str()) {
            calls
                .entry(id.to_owned())
                .or_insert_with(|| name.to_owned());
        }
    }
}

fn message_block(
    key: String,
    role: Role,
    blocks: &[Value],
    calls: &BTreeMap<String, String>,
    note: Option<String>,
    turn: usize,
    clock: &str,
) -> Block {
    let visible = visible(blocks);
    let only_calls =
        !visible.is_empty() && visible.iter().all(|block| block["type"] == "tool_call");
    let failed = visible
        .iter()
        .any(|block| block["is_error"].as_bool() == Some(true));
    let (label, bg, accent, fold, preview) = match role {
        Role::User => (
            "YOU".to_owned(),
            Some(theme::USER_BG),
            theme::ACCENT,
            Policy::Fit(12),
            false,
        ),
        Role::Assistant if only_calls => (
            call_label(&visible),
            Some(theme::TOOL_BG),
            theme::WARN,
            Policy::Open,
            false,
        ),
        Role::Assistant => (
            "HEARTH".to_owned(),
            None,
            theme::ACCENT,
            Policy::Open,
            false,
        ),
        Role::Tool => (
            format!("TOOL RESULT · {}", result_name(&visible, calls)),
            Some(if failed {
                theme::TOOL_BAD_BG
            } else {
                theme::TOOL_OK_BG
            }),
            if failed { theme::BAD } else { theme::GOOD },
            Policy::Fit(8),
            true,
        ),
        Role::System => (
            "SYSTEM".to_owned(),
            None,
            theme::MUTED,
            Policy::Fit(8),
            true,
        ),
    };
    Block {
        key,
        label,
        meta: format!("turn {turn} · {clock}"),
        note,
        parts: parts_of(&visible, role, calls),
        fold,
        preview,
        bg,
        accent,
        fg: theme::TEXT,
    }
}

fn response_block(
    key: String,
    blocks: &[Value],
    calls: &BTreeMap<String, String>,
    turn: usize,
    clock: &str,
) -> Block {
    let visible = visible(blocks);
    let only_calls =
        !visible.is_empty() && visible.iter().all(|block| block["type"] == "tool_call");
    let (label, bg, accent) = if only_calls {
        (call_label(&visible), Some(theme::TOOL_BG), theme::WARN)
    } else {
        ("HEARTH".to_owned(), None, theme::ACCENT)
    };
    Block {
        key,
        label,
        meta: format!("turn {turn} · {clock}"),
        note: None,
        parts: parts_of(&visible, Role::Assistant, calls),
        fold: Policy::Open,
        preview: false,
        bg,
        accent,
        fg: theme::TEXT,
    }
}

/// The blocks a reader sees: thinking is never rendered.
fn visible(blocks: &[Value]) -> Vec<Value> {
    blocks
        .iter()
        .filter(|block| block["type"] != "thinking")
        .cloned()
        .collect()
}

fn parts_of(blocks: &[Value], role: Role, calls: &BTreeMap<String, String>) -> Vec<Part> {
    let mut parts = Vec::new();
    for block in blocks {
        let part = match block["type"].as_str().unwrap_or_default() {
            "text" => Part {
                label: None,
                text: block["text"].as_str().unwrap_or_default().to_owned(),
            },
            "tool_call" => Part {
                label: Some(format!("⚙ {}", block["name"].as_str().unwrap_or("tool"))),
                text: arguments(&block["arguments"]),
            },
            "tool_result" => {
                let name = call_name(block["call_id"].as_str(), calls);
                let failed = block["is_error"].as_bool() == Some(true);
                Part {
                    label: (role != Role::Tool)
                        .then(|| format!("{name} {}", if failed { "✗" } else { "✓" })),
                    text: block["content"].as_str().unwrap_or_default().to_owned(),
                }
            }
            "file" => Part {
                label: Some(format!(
                    "file · {}",
                    block["name"].as_str().unwrap_or("file")
                )),
                text: block["path"].as_str().unwrap_or("-").to_owned(),
            },
            "image" => Part {
                label: Some("image".to_owned()),
                text: block["path"].as_str().unwrap_or("-").to_owned(),
            },
            _ => continue,
        };
        parts.push(part);
    }
    parts
}

fn call_label(blocks: &[Value]) -> String {
    match blocks {
        [block] => format!("TOOL CALL · {}", block["name"].as_str().unwrap_or("tool")),
        _ => format!("TOOL CALL · {} calls", blocks.len()),
    }
}

fn result_name(blocks: &[Value], calls: &BTreeMap<String, String>) -> String {
    blocks
        .iter()
        .find(|block| block["type"] == "tool_result")
        .map(|block| call_name(block["call_id"].as_str(), calls))
        .unwrap_or_else(|| "tool".to_owned())
}

fn call_name(id: Option<&str>, calls: &BTreeMap<String, String>) -> String {
    id.and_then(|id| calls.get(id).cloned())
        .unwrap_or_else(|| "tool".to_owned())
}

/// Tool arguments as a reader can audit them: pretty when they are a structure,
/// compact when they are one value.
fn arguments(arguments: &Value) -> String {
    match arguments {
        Value::Null => String::new(),
        Value::Object(_) | Value::Array(_) => {
            serde_json::to_string_pretty(arguments).unwrap_or_default()
        }
        value => value.to_string(),
    }
}

/// A layer's text. The tool declarations are a JSON array in the document, so
/// they are printed as names, descriptions, and argument fields instead.
fn layer_text(request: &Value, layer: &Layer) -> String {
    if layer.part == "tools" {
        return tools_text(request);
    }
    request[layer.text].as_str().unwrap_or_default().to_owned()
}

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

fn token(document: &Value, name: &str) -> Option<u64> {
    document["tokens"][name].as_u64()
}

fn cost(tokens: Option<u64>) -> String {
    tokens
        .map(|tokens| format!("{} tok", count(tokens)))
        .unwrap_or_default()
}

fn build_cache(
    exchanges: &[Value],
    key: &CacheKey,
    folds: &Folds,
    width: usize,
    zone: &Zone,
) -> PromptCache {
    let chat = derive(exchanges, zone);
    let mut painter = Painter::new(width, folds);
    for entry in &chat.entries {
        match entry {
            Entry::Notice(notice) => painter.notice(notice),
            Entry::Block(block) => painter.block(block),
        }
    }
    PromptCache {
        key: key.clone(),
        summary: chat.summary,
        blocks: painter.blocks,
        lines: painter.lines,
    }
}

/// One row of a block body.
enum Content {
    /// A sub-panel label, such as a tool call inside an assistant block.
    Label(String),
    Text(String),
    /// The hidden-count stub under a folded block.
    Stub(String),
    /// A blank row between two parts, so a panel band stays solid.
    Gap,
}

fn content(block: &Block, inner: usize) -> Vec<Content> {
    let mut rows = Vec::new();
    for (index, part) in block.parts.iter().enumerate() {
        if part.label.is_none() && part.text.trim().is_empty() {
            continue;
        }
        if index > 0 && !rows.is_empty() {
            rows.push(Content::Gap);
        }
        if let Some(label) = &part.label {
            rows.push(Content::Label(label.clone()));
        }
        rows.extend(wrap_text(&part.text, inner).into_iter().map(Content::Text));
    }
    rows
}

/// The chat, drawn as lines with a fixed width.
struct Painter<'a> {
    width: usize,
    folds: &'a Folds,
    lines: Vec<Line<'static>>,
    /// The block headers, in document order.
    blocks: Vec<BlockSpan>,
}

impl<'a> Painter<'a> {
    fn new(width: usize, folds: &'a Folds) -> Self {
        Self {
            width: width.max(1),
            folds,
            lines: Vec::new(),
            blocks: Vec::new(),
        }
    }

    fn gap(&mut self) {
        if !self.lines.is_empty() {
            self.lines.push(Line::from(""));
        }
    }

    fn notice(&mut self, notice: &Notice) {
        self.gap();
        let width = self.width;
        let line = match notice {
            Notice::Conversation(session) => Line::styled(
                clip(&format!(" NEW CONVERSATION · {}", short(session, 8)), width),
                theme::faint(),
            ),
            Notice::Rewritten(dropped) => {
                let mut row = Row::new();
                row.push(" HISTORY REWRITTEN".to_owned(), theme::warn());
                row.push(
                    format!(" · {dropped} earlier messages dropped"),
                    theme::faint(),
                );
                row.fit(width)
            }
            Notice::NoReply => Line::styled(clip("  no reply saved", width), theme::faint()),
        };
        self.lines.push(line);
    }

    fn block(&mut self, block: &Block) {
        let inner = self.width.saturating_sub(BODY_INDENT).max(1);
        let rows = content(block, inner);
        let default_open = match block.fold {
            Policy::Open => true,
            Policy::Closed => rows.is_empty(),
            Policy::Fit(lines) => rows.len() <= lines,
        };
        let open = self.folds.open(&block.key, default_open);
        self.gap();
        let line = self.lines.len();
        let lead = format!("▌ {}", block.label);
        let label_style = tinted(block.accent, block.bg).add_modifier(Modifier::BOLD);
        let header = band_line(
            vec![Span::styled(lead.clone(), label_style)],
            &block.meta,
            faded(block.bg),
            self.width,
            fill(block.bg),
        );
        // The selection only changes the band, so the row still opens with its
        // own label and the draw can swap it in without rebuilding.
        let selected = band_line(
            vec![Span::styled(lead, theme::selected())],
            &block.meta,
            theme::selected(),
            self.width,
            theme::panel_fill(theme::SELECT_BG),
        );
        self.lines.push(header);
        if rows.is_empty() {
            self.content(block, &Content::Text("(empty)".to_owned()));
        } else if open {
            for row in &rows {
                self.content(block, row);
            }
        } else {
            let shown = usize::from(block.preview);
            if shown == 1 {
                self.content(block, &rows[0]);
            }
            let hidden = rows.len() - shown;
            let stub = if hidden == 1 {
                "… 1 line hidden · enter to open".to_owned()
            } else {
                format!("… {hidden} lines hidden · enter to open")
            };
            for line in wrap_text(&stub, inner) {
                self.content(block, &Content::Stub(line));
            }
        }
        // The note reads as the last line of the message it rode on, and it
        // stays visible when the body is folded.
        if let Some(note) = &block.note {
            self.note(block, note);
        }
        self.blocks.push(BlockSpan {
            key: block.key.clone(),
            line,
            default_open,
            selected,
        });
    }

    fn content(&mut self, block: &Block, row: &Content) {
        let line = match row {
            Content::Label(text) => label_line(text, block.accent, block.bg, self.width),
            Content::Text(text) => body_line(text, block.accent, block.fg, block.bg, self.width),
            Content::Stub(text) => {
                body_line(text, block.accent, theme::FAINT, block.bg, self.width)
            }
            Content::Gap => body_line("", block.accent, block.fg, block.bg, self.width),
        };
        self.lines.push(line);
    }

    /// The clock note that rode on the message, always visible.
    fn note(&mut self, block: &Block, note: &str) {
        let indent = " ".repeat(BODY_INDENT);
        let inner = self.width.saturating_sub(BODY_INDENT + 2).max(1);
        for (index, line) in wrap_text(note, inner).iter().enumerate() {
            let lead = if index == 0 {
                format!("{indent}◷ ")
            } else {
                format!("{indent}  ")
            };
            let style = faded(block.bg);
            let spans = vec![Span::styled(lead, style), Span::styled(line.clone(), style)];
            self.lines.push(fit_line(spans, self.width, fill(block.bg)));
        }
    }
}

/// A header band: the block label, then its meta against the right edge.
fn band_line(
    mut spans: Vec<Span<'static>>,
    meta: &str,
    meta_style: Style,
    width: usize,
    fill: Style,
) -> Line<'static> {
    // A trailing pad keeps the meta off the card border without ending the band.
    let meta = if meta.is_empty() {
        String::new()
    } else {
        format!("{meta} ")
    };
    let used: usize = spans.iter().map(|span| span.content.width()).sum();
    if !meta.is_empty() && used + 1 + meta.width() <= width {
        spans.push(Span::styled(
            " ".repeat(width - used - meta.width()),
            meta_style,
        ));
        spans.push(Span::styled(meta, meta_style));
    }
    fit_line(spans, width, fill)
}

/// A body row: the accent gutter, then the text in the body style.
fn body_line(
    text: &str,
    accent: Color,
    fg: Color,
    bg: Option<Color>,
    width: usize,
) -> Line<'static> {
    let spans = vec![
        Span::styled("▏ ".to_owned(), tinted(accent, bg)),
        Span::styled(text.to_owned(), tinted(fg, bg)),
    ];
    fit_line(spans, width, fill(bg))
}

/// A sub-panel label inside a body, in the block accent.
fn label_line(text: &str, accent: Color, bg: Option<Color>, width: usize) -> Line<'static> {
    let style = tinted(accent, bg).add_modifier(Modifier::BOLD);
    let spans = vec![
        Span::styled("▏ ".to_owned(), tinted(accent, bg)),
        Span::styled(text.to_owned(), style),
    ];
    fit_line(spans, width, fill(bg))
}

/// Text on a block's band, or on the page when it has none.
fn tinted(fg: Color, bg: Option<Color>) -> Style {
    match bg {
        Some(bg) => theme::panel_text(bg).fg(fg),
        None => Style::default().fg(fg),
    }
}

/// Secondary text: meta, notes, and the fold stub.
fn faded(bg: Option<Color>) -> Style {
    match bg {
        Some(bg) => theme::panel_stub(bg),
        None => theme::faint(),
    }
}

/// A panel line is filled to the full width, so a band never stops mid-row.
fn fill(bg: Option<Color>) -> Style {
    match bg {
        Some(bg) => theme::panel_fill(bg),
        None => Style::default(),
    }
}

/// Cut composed spans to `width` and fill the remainder.
fn fit_line(spans: Vec<Span<'static>>, width: usize, fill: Style) -> Line<'static> {
    let mut fitted = Vec::with_capacity(spans.len() + 1);
    let mut used = 0;
    for span in spans {
        if used >= width {
            break;
        }
        let mut content = String::new();
        for character in span.content.chars() {
            let character_width = character.width().unwrap_or(0);
            if used + character_width > width {
                break;
            }
            used += character_width;
            content.push(character);
        }
        if !content.is_empty() {
            fitted.push(Span::styled(content, span.style));
        }
    }
    if used < width {
        fitted.push(Span::styled(" ".repeat(width - used), fill));
    }
    Line::from(fitted)
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
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;
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

    /// Draw the Prompt tab alone and keep the cells, so a test can read styles.
    fn draw(app: &App, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                let area = frame.area();
                render(app, frame, area);
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    /// The card border is not part of what the reader sees inside it.
    fn rendered(app: &App, width: u16, height: u16) -> String {
        let buffer = draw(app, width, height);
        let mut text = String::new();
        for row in 0..height {
            for column in 1..width.saturating_sub(1) {
                text.push_str(buffer[(column, row)].symbol());
            }
            text.push('\n');
        }
        text
    }

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

    /// The first row whose visible text opens with `label`.
    fn row_of(buffer: &Buffer, width: u16, label: &str) -> u16 {
        (0..buffer.area.height)
            .find(|row| {
                let text: String = (1..width.saturating_sub(1))
                    .map(|column| buffer[(column, *row)].symbol())
                    .collect();
                text.starts_with(label)
            })
            .unwrap_or_else(|| panic!("missing row {label}"))
    }

    fn header_bg(buffer: &Buffer, width: u16, label: &str) -> Option<Color> {
        let row = row_of(buffer, width, label);
        buffer[(1, row)].style().bg
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

    fn long_result() -> String {
        let mut lines = vec!["package hearth-cli".to_owned()];
        for index in 0..30 {
            lines.push(format!("result line {index}"));
        }
        lines.join("\n")
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
            "blocks": [{"type": "tool_result", "call_id": "c1", "content": long_result()}]
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

    /// One saved request with no prefix layers, so the chat holds one block.
    fn single_user(message: &str) -> Vec<Value> {
        vec![json!({
            "request": {
                "transcript": [{"role": "user", "blocks": [{"type": "text", "text": message}]}]
            },
            "prompt_hash": "only-hash",
            "requested_at": 1_700_000_000_000_000_i64,
            "response": Value::Null,
            "usage": Value::Null
        })]
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
    fn a_saved_conversation_is_a_chat_of_labelled_blocks() {
        let (_home, app) = prompt_app(conversation());
        let text = rendered(&app, 100, 80);
        let system = starting(&text, "▌ SYSTEM · SOUL.md");
        assert_eq!(system.len(), 1, "the identity is one context block: {text}");
        assert_eq!(starting(&text, "▌ MEMORY").len(), 1, "{text}");
        assert_eq!(starting(&text, "▌ YOU").len(), 1, "{text}");
        assert_eq!(
            starting(&text, "▌ TOOL CALL · read_file").len(),
            1,
            "the tool call is its own block: {text}"
        );
        let result = starting(&text, "▌ TOOL RESULT · read_file");
        assert_eq!(result.len(), 1, "the tool result is its own block: {text}");
        let answer = starting(&text, "▌ HEARTH");
        assert_eq!(answer.len(), 1, "the answer is its own block: {text}");
        assert!(
            system[0] < line_of(&text, QUESTION) && line_of(&text, QUESTION) < result[0],
            "blocks follow the order the model saw: {text}"
        );
        assert!(!text.contains(SOUL), "a context body starts folded: {text}");
    }

    #[test]
    fn e_expands_every_block_and_c_collapses_every_block() {
        let (_home, mut app) = prompt_app(conversation());
        let folded = rendered(&app, 100, 80);
        assert!(!folded.contains(SOUL), "context starts folded: {folded}");
        assert!(
            !folded.contains("result line 29"),
            "the long tool result starts folded: {folded}"
        );
        assert!(
            folded.contains("hidden") && folded.to_lowercase().contains("enter"),
            "a folded block names what it hides and the key: {folded}"
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        let open = rendered(&app, 100, 120);
        assert!(open.contains(SOUL), "e opens every block: {open}");
        assert!(
            open.contains("result line 29"),
            "e opens the tool result: {open}"
        );
        assert!(open.contains(ANSWER), "the answer is still there: {open}");

        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        let closed = rendered(&app, 100, 80);
        assert!(
            !closed.contains(QUESTION) && !closed.contains(ANSWER),
            "c folds messages too: {closed}"
        );
        assert!(
            !closed.contains("result line 0") && !closed.contains(SOUL),
            "c folds tool and context bodies: {closed}"
        );
        for label in [
            "▌ SYSTEM · SOUL.md",
            "▌ YOU",
            "▌ TOOL RESULT · read_file",
            "▌ HEARTH",
        ] {
            assert!(
                closed.contains(label),
                "c keeps every header visible: missing {label} in {closed}"
            );
        }
    }

    #[test]
    fn n_and_p_move_a_cursor_over_every_block() {
        let (_home, mut app) = prompt_app(conversation());
        let _ = rendered(&app, 100, 80);
        app.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        let _ = rendered(&app, 100, 80);

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        let buffer = draw(&app, 100, 80);
        assert_eq!(
            header_bg(&buffer, 100, "▌ SYSTEM · SOUL.md"),
            Some(theme::SELECT_BG),
            "n selects the first block in the document"
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        let buffer = draw(&app, 100, 80);
        assert_eq!(
            header_bg(&buffer, 100, "▌ SYSTEM · guidance"),
            Some(theme::SELECT_BG),
            "n reaches the next block"
        );
        assert_ne!(
            header_bg(&buffer, 100, "▌ SYSTEM · SOUL.md"),
            Some(theme::SELECT_BG),
            "only the current block is selected"
        );

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let text = rendered(&app, 100, 80);
        assert!(
            text.contains("PROJECT RULES"),
            "enter opens the selected block: {text}"
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let text = rendered(&app, 100, 80);
        assert!(
            text.contains(SOUL),
            "p returns to the first block and enter opens it: {text}"
        );
    }

    #[test]
    fn a_short_message_is_a_foldable_block() {
        let (_home, mut app) = prompt_app(single_user("a short question"));
        let text = rendered(&app, 100, 40);
        assert!(text.contains("a short question"), "{text}");
        app.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let text = rendered(&app, 100, 40);
        assert!(
            !text.contains("a short question"),
            "enter folds a short message too: {text}"
        );
        assert!(text.contains("▌ YOU"), "the header stays visible: {text}");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            rendered(&app, 100, 40).contains("a short question"),
            "enter opens it again"
        );
    }

    #[test]
    fn a_block_header_carries_its_turn_and_local_time() {
        let (_home, app) = prompt_app(conversation());
        let text = rendered(&app, 100, 80);
        let user = text
            .lines()
            .find(|line| line.starts_with("▌ YOU"))
            .expect("the user header");
        assert!(user.contains("turn 1"), "the header names its turn: {user}");
        assert!(
            user.contains("22:13:20"),
            "the header names the request time in the configured zone: {user}"
        );
        let answer = text
            .lines()
            .find(|line| line.starts_with("▌ HEARTH"))
            .expect("the answer header");
        assert!(answer.contains("turn 2"), "{answer}");
        assert!(
            answer.contains("22:13:35"),
            "the answer names the response time: {answer}"
        );
    }

    #[test]
    fn user_and_tool_blocks_are_panels_and_assistant_text_is_not() {
        let (_home, app) = prompt_app(conversation());
        let buffer = draw(&app, 100, 80);
        let user = header_bg(&buffer, 100, "▌ YOU");
        let tool = header_bg(&buffer, 100, "▌ TOOL RESULT");
        let assistant = header_bg(&buffer, 100, "▌ HEARTH");
        assert!(user.is_some(), "the user block is a banded panel");
        assert!(tool.is_some(), "the tool block is a banded panel");
        assert_ne!(user, tool, "user and tool panels read differently");
        assert_eq!(
            assistant,
            Some(Color::Reset),
            "assistant text sits on the page, like a chat"
        );
    }

    #[test]
    fn the_status_row_names_model_turns_messages_and_tokens() {
        let (_home, app) = prompt_app(conversation());
        let text = rendered(&app, 100, 40);
        for expected in ["test-model", "2 turns", "4 messages", "620 tok"] {
            assert!(text.contains(expected), "missing {expected}: {text}");
        }
    }

    #[test]
    fn the_saved_transcript_is_merged_without_repeats() {
        let (_home, app) = prompt_app(conversation());
        let text = rendered(&app, 100, 120);
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
        assert_eq!(
            text.matches("▌ TOOL CALL · read_file").count(),
            1,
            "the call is stored once: {text}"
        );
        assert_eq!(
            text.matches(ANSWER).count(),
            1,
            "the answer is stored once: {text}"
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
        let text = rendered(&app, 100, 80);
        let lowered = text.to_lowercase();
        let rewritten = line_of(&lowered, "rewritten");
        let fresh = line_of(&text, "a fresh start");
        assert!(
            rewritten < fresh,
            "the note comes before the rewritten conversation: {text}"
        );
        assert!(lowered.contains("earlier messages dropped"), "{text}");
    }

    #[test]
    fn the_runtime_note_sits_under_the_message_that_carried_it() {
        let (_home, app) = prompt_app(conversation());
        let text = rendered(&app, 100, 120);
        let question = line_of(&text, QUESTION);
        let note = line_of(&text, "CLOCK NOTE seven seconds past");
        let call = line_of(&text, "TOOL CALL · read_file");
        assert!(
            question < note && note < call,
            "the note sits under the question it rode with: {text}"
        );
        let result = line_of(&text, "TOOL RESULT · read_file");
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
    fn the_live_turn_is_a_streaming_block_after_the_saved_chat() {
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
        let text = rendered(&app, 100, 80);
        let answer = line_of(&text, ANSWER);
        let live = line_of(&text, "STREAMING REPLY");
        assert!(
            live > answer,
            "the live block sits after the saved answer: {text}"
        );
        assert!(
            line_of(&text, "WEIGHING IT") < live,
            "thinking comes before the text it precedes: {text}"
        );
        assert!(text.contains("streaming"), "{text}");
        assert!(
            spun(&text, "HEARTH"),
            "the spinner sits on the live block header: {text}"
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        let text = rendered(&app, 100, 80);
        assert!(
            text.contains("STREAMING REPLY"),
            "collapse all never folds the turn being written: {text}"
        );
        app.apply_event(AppEvent::Live(json!({
            "kind": "run_settled",
            "data": {"status": "done"}
        })));
        assert!(
            !rendered(&app, 100, 80).contains("STREAMING REPLY"),
            "the saved document replaces the live turn"
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
        let text = rendered(&app, 100, 80);
        assert!(
            !text.contains("OLD STREAM"),
            "a new turn does not inherit the last turn's text: {text}"
        );
        assert!(text.contains("NEW STREAM"), "{text}");
    }

    #[test]
    fn a_refresh_alone_does_not_animate() {
        let (_home, mut app) = prompt_app(conversation());
        app.prompt.loading = true;
        let text = rendered(&app, 100, 24);
        assert!(!spun(&text, "HEARTH"), "a reload is not a turn: {text}");
        app.prompt.loading = false;
        app.status = Some(json!({"tasks_running": 2}));
        let text = rendered(&app, 100, 24);
        assert!(
            text.contains(ANSWER),
            "a status poll must not replace the conversation: {text}"
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
        let text = rendered(&app, 100, 80);
        assert!(
            text.to_lowercase().contains("conversation · session-"),
            "{text}"
        );
        assert!(text.contains("a different conversation"), "{text}");
        assert!(
            !text.to_lowercase().contains("rewritten"),
            "a new conversation is not a rewritten one: {text}"
        );
        assert_eq!(text.matches(QUESTION).count(), 1, "{text}");
    }

    #[test]
    fn a_request_with_no_saved_reply_says_so() {
        let mut exchanges = conversation();
        exchanges[0]["response"] = Value::Null;
        let (_home, app) = prompt_app(exchanges);
        let text = rendered(&app, 100, 80);
        assert!(text.contains("no reply saved"), "{text}");
    }

    #[test]
    fn a_response_with_thinking_is_not_reprinted_by_the_next_request() {
        let user = json!({
            "role": "user",
            "blocks": [{"type": "text", "text": QUESTION}]
        });
        let oldest = json!({
            "request": request(json!([user.clone()]), 480),
            "prompt_hash": "thinking-one",
            "requested_at": 1_700_000_000_000_000_i64,
            "response": {
                "entry_id": "entry-thinking",
                "created_at": 1_700_000_005_000_000_i64,
                "blocks": [
                    {"type": "thinking", "text": "A PRIVATE THOUGHT"},
                    {"type": "text", "text": "FIRST ANSWER"}
                ],
                "tokens_est": 8
            },
            "usage": Value::Null
        });
        let newest = json!({
            "request": request(
                json!([
                    user,
                    {"role": "assistant", "blocks": [{"type": "text", "text": "FIRST ANSWER"}]},
                    {"role": "user", "blocks": [{"type": "text", "text": "SECOND QUESTION"}]}
                ]),
                560
            ),
            "prompt_hash": "thinking-two",
            "requested_at": 1_700_000_010_000_000_i64,
            "response": Value::Null,
            "usage": Value::Null
        });
        let (_home, app) = prompt_app(vec![newest, oldest]);
        let text = rendered(&app, 100, 80);
        let lowered = text.to_lowercase();
        assert_eq!(
            text.matches("FIRST ANSWER").count(),
            1,
            "the answered turn is not reprinted without its thinking: {text}"
        );
        assert!(
            !lowered.contains("rewritten"),
            "a thinking block is not a rewritten history: {text}"
        );
        assert!(text.contains("SECOND QUESTION"), "{text}");
    }

    #[test]
    fn memory_is_shown_again_only_when_it_changes() {
        let mut exchanges = conversation();
        exchanges[0]["request"]["recall"] = json!("FRESH MEMORY");
        let (_home, app) = prompt_app(exchanges);
        let text = rendered(&app, 100, 120);
        assert_eq!(
            text.matches("MEMORY HIT").count(),
            1,
            "the unchanged recall is not repeated: {text}"
        );
        assert_eq!(
            text.matches("FRESH MEMORY").count(),
            1,
            "the changed recall is shown at its request: {text}"
        );
        assert!(
            line_of(&text, "MEMORY HIT") < line_of(&text, "FRESH MEMORY"),
            "recall follows the request order: {text}"
        );
    }

    #[test]
    fn the_card_carries_no_key_legend() {
        let (_home, app) = prompt_app(conversation());
        let text = rendered(&app, 120, 40);
        assert!(
            !text.contains("e/c") && !text.contains("n/p"),
            "the card must not repeat the key legend: {text}"
        );
    }

    #[test]
    fn a_realistic_chat_fits_eighty_and_one_hundred_and_twenty_columns() {
        for width in [80u16, 120] {
            let (_home, app) = prompt_app(conversation());
            let text = rendered(&app, width, 80);
            for line in cache_lines(&app) {
                assert!(
                    line.width() <= usize::from(width) - 2,
                    "line {line:?} does not fit {width} columns"
                );
            }
            for expected in [
                "▌ SYSTEM · SOUL.md",
                "▌ YOU",
                "▌ TOOL RESULT · read_file",
                ANSWER,
            ] {
                assert!(
                    text.contains(expected),
                    "width {width} is missing {expected}: {text}"
                );
            }
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
