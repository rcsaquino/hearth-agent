//! Onboarding screens.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::App;
use crate::onboarding::{Step, TextInput};
use crate::providers::{PROVIDERS, THINKING};
use crate::theme;
use crate::ui::short;

const WORDMARK: [&str; 6] = [
    "██╗  ██╗███████╗ █████╗ ██████╗ ████████╗██╗  ██╗",
    "██║  ██║██╔════╝██╔══██╗██╔══██╗╚══██╔══╝██║  ██║",
    "███████║█████╗  ███████║██████╔╝   ██║   ███████║",
    "██╔══██║██╔══╝  ██╔══██║██╔══██╗   ██║   ██╔══██║",
    "██║  ██║███████╗██║  ██║██║  ██║   ██║   ██║  ██║",
    "╚═╝  ╚═╝╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝   ╚═╝   ╚═╝  ╚═╝",
];
const LABEL_WIDTH: usize = 17;
const PASTE_HINT: &str = "Paste: Ctrl+Shift+V (or Ctrl+V where supported)";

pub fn render(app: &App, frame: &mut Frame) {
    let area = frame.area();
    if area.width < 70 || area.height < 20 {
        frame.render_widget(
            Paragraph::new("Please enlarge the terminal to at least 70x20.")
                .block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    }
    let width = area.width.saturating_sub(6).min(78);
    let height = area.height.saturating_sub(2).min(28);
    let card = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_focus())
        .title(theme::heading(" Hearth setup "));
    let inner = block.inner(card);
    frame.render_widget(block, card);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(inner);
    render_progress(app, frame, rows[0]);
    render_step(app, frame, rows[1]);
    render_footer(app, frame, rows[2]);
}

fn render_progress(app: &App, frame: &mut Frame, area: Rect) {
    if app.onboarding.step == Step::Welcome {
        return;
    }
    let (current, total) = step_number(app.onboarding.step);
    let mut spans = vec![
        Span::styled("Step ", theme::muted()),
        Span::styled(current.to_string(), theme::accent_style()),
        Span::styled(format!(" of {total}   "), theme::muted()),
    ];
    for index in 1..=total {
        let dot = if index <= current { "●" } else { "○" };
        let style = if index == current {
            theme::accent_style()
        } else if index < current {
            theme::good()
        } else {
            theme::faint()
        };
        spans.push(Span::styled(dot, style));
        spans.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn step_number(step: Step) -> (usize, usize) {
    match step {
        Step::Welcome => (1, 10),
        Step::Telegram => (2, 10),
        Step::Provider => (3, 10),
        Step::Credentials => (4, 10),
        Step::Model => (5, 10),
        Step::Thinking => (6, 10),
        Step::Agent => (7, 10),
        Step::Extras => (8, 10),
        Step::Daemon => (9, 10),
        Step::Review | Step::Creating | Step::Failed => (10, 10),
    }
}

fn render_step(app: &App, frame: &mut Frame, area: Rect) {
    match app.onboarding.step {
        Step::Welcome => welcome(frame, area),
        Step::Telegram => telegram(app, frame, area),
        Step::Provider => provider(app, frame, area),
        Step::Credentials => credentials(app, frame, area),
        Step::Model => model(app, frame, area),
        Step::Thinking => thinking(app, frame, area),
        Step::Agent => agent(app, frame, area),
        Step::Extras => extras(app, frame, area),
        Step::Daemon => daemon(app, frame, area),
        Step::Review => review(app, frame, area),
        Step::Creating => creating(frame, area),
        Step::Failed => failed(app, frame, area),
    }
}

fn welcome(frame: &mut Frame, area: Rect) {
    let mut lines = Vec::new();
    lines.push(Line::from(""));
    for line in WORDMARK {
        lines.push(Line::styled(line, theme::accent_style()));
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "A durable personal agent, built with Rust.",
        theme::text().add_modifier(Modifier::BOLD),
    ));
    lines.push(Line::styled(
        "It lives in Telegram, keeps its own record, and runs on this machine.",
        theme::muted(),
    ));
    lines.push(Line::from(""));
    lines.push(Line::styled("Press Enter to begin.", theme::good()));
    let paragraph = Paragraph::new(lines)
        .alignment(ratatui::layout::Alignment::Center)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn telegram(app: &App, frame: &mut Frame, area: Rect) {
    let mut lines = vec![
        title("Connect Telegram"),
        subtitle("Telegram is how you talk to Hearth."),
        Line::from(""),
        Line::styled("  1. Message @BotFather in Telegram.", theme::muted()),
        Line::styled("  2. Send /newbot and follow the prompts.", theme::muted()),
        Line::styled("  3. Copy the token it sends you.", theme::muted()),
        Line::from(""),
    ];
    let mut cursor = None;
    if let Some(found) = push_field(
        &mut lines,
        "Bot token",
        &app.onboarding.token,
        app.onboarding.focus == 0,
        area.width,
        "123456789:AA...",
    ) {
        cursor = Some(found);
    }
    lines.push(Line::from(""));
    lines.push(subtitle(
        "Then message @userinfobot and copy your numeric user id.",
    ));
    if let Some(found) = push_field(
        &mut lines,
        "Telegram user id",
        &app.onboarding.user_id,
        app.onboarding.focus == 1,
        area.width,
        "123456789",
    ) {
        cursor = Some(found);
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "Both values are stored locally in settings.json with mode 0600.",
        theme::faint(),
    ));
    frame.render_widget(Paragraph::new(lines), area);
    set_cursor(frame, area, cursor);
}

fn provider(app: &App, frame: &mut Frame, area: Rect) {
    let mut lines = vec![
        title("Choose your model provider"),
        subtitle("Pick a preset, or point Hearth at any OpenAI-compatible API."),
        Line::from(""),
    ];
    for (index, preset) in PROVIDERS.iter().enumerate() {
        let selected = index == app.onboarding.provider;
        let marker = if selected { "▸" } else { " " };
        let endpoint = preset.base_url.unwrap_or("custom URL");
        let text = format!(" {marker} {:<26} {endpoint}", preset.label);
        lines.push(if selected {
            Line::styled(text, theme::selected())
        } else {
            Line::styled(text, theme::text())
        });
    }
    lines.push(Line::from(""));
    lines.push(Line::styled("Use ↑/↓ and Enter.", theme::faint()));
    frame.render_widget(Paragraph::new(lines), area);
}

fn credentials(app: &App, frame: &mut Frame, area: Rect) {
    let preset = &PROVIDERS[app.onboarding.provider];
    let other = preset.base_url.is_none();
    let mut lines = vec![
        title(&format!("Connect to {}", preset.label)),
        subtitle("Hearth uses this endpoint for every model request."),
        Line::from(""),
    ];
    let mut cursor = None;
    if other {
        if let Some(found) = push_field(
            &mut lines,
            "Provider URL",
            &app.onboarding.base_url,
            app.onboarding.focus == 0,
            area.width,
            "https://example.com/v1",
        ) {
            cursor = Some(found);
        }
    } else {
        lines.push(readonly_field(
            "Provider URL",
            app.onboarding.base_url.value.as_str(),
            area.width,
        ));
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        crate::providers::key_hint(&app.onboarding.base_url.value),
        theme::faint(),
    ));
    lines.push(Line::from(""));
    if let Some(found) = push_field(
        &mut lines,
        "API key",
        &app.onboarding.api_key,
        app.onboarding.focus == 1,
        area.width,
        "sk-...",
    ) {
        cursor = Some(found);
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "The key is stored in settings.json with mode 0600.",
        theme::faint(),
    ));
    frame.render_widget(Paragraph::new(lines), area);
    set_cursor(frame, area, cursor);
}

fn model(app: &App, frame: &mut Frame, area: Rect) {
    let onboarding = &app.onboarding;
    let mut lines = vec![
        title("Choose a model"),
        subtitle("This model answers your Telegram messages."),
        Line::from(""),
    ];
    let mut cursor = None;
    if onboarding.models_loading {
        lines.push(Line::styled(
            "Loading the provider model list...",
            theme::warn(),
        ));
        lines.push(Line::styled(
            "Press Esc to go back and check the key or URL.",
            theme::faint(),
        ));
    } else if onboarding.models.is_empty() {
        if let Some(error) = &onboarding.models_error {
            lines.push(Line::styled(
                format!("Could not load the model list: {error}"),
                theme::bad(),
            ));
        } else {
            lines.push(Line::styled(
                "The provider listed no models.",
                theme::warn(),
            ));
        }
        lines.push(Line::styled(
            "Type a model name and press Enter.",
            theme::muted(),
        ));
        lines.push(Line::from(""));
        if let Some(found) = push_field(
            &mut lines,
            "Model",
            &onboarding.model_manual,
            true,
            area.width,
            "deepseek-v4.1-flash",
        ) {
            cursor = Some(found);
        }
    } else {
        if let Some(found) = push_field(
            &mut lines,
            "Filter",
            &onboarding.model_filter,
            true,
            area.width,
            "type to search",
        ) {
            cursor = Some(found);
        }
        let filtered = onboarding.filtered_models();
        lines.push(Line::from(vec![
            Span::styled("  ", theme::faint()),
            Span::styled(
                format!("{} of {} models", filtered.len(), onboarding.models.len()),
                theme::faint(),
            ),
        ]));
        lines.push(Line::from(""));
        let list_area = area.height.saturating_sub(lines.len() as u16) as usize;
        let start = onboarding
            .model_selected
            .saturating_sub(list_area.saturating_sub(1) / 2);
        for (index, model) in filtered.iter().enumerate().skip(start).take(list_area) {
            let selected = index == onboarding.model_selected;
            let marker = if selected { "▸" } else { " " };
            let text = format!(" {marker} {}", short(model, area.width as usize - 4));
            lines.push(if selected {
                Line::styled(text, theme::selected())
            } else {
                Line::styled(text, theme::text())
            });
        }
        if filtered.is_empty() {
            lines.push(Line::styled(
                "  No models match the filter.",
                theme::faint(),
            ));
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
    set_cursor(frame, area, cursor);
}

fn thinking(app: &App, frame: &mut Frame, area: Rect) {
    let mut lines = vec![
        title("Default thinking"),
        subtitle("How much reasoning should the model do before answering?"),
        Line::from(""),
    ];
    for (index, (_, label)) in THINKING.iter().enumerate() {
        let selected = index == app.onboarding.thinking;
        let marker = if selected { "▸" } else { " " };
        let text = format!(" {marker} {label}");
        lines.push(if selected {
            Line::styled(text, theme::selected())
        } else {
            Line::styled(text, theme::text())
        });
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "Extra high and maximum send xhigh and max.",
        theme::faint(),
    ));
    lines.push(Line::styled(
        "Availability depends on the provider and model.",
        theme::faint(),
    ));
    frame.render_widget(Paragraph::new(lines), area);
}

fn agent(app: &App, frame: &mut Frame, area: Rect) {
    let mut lines = vec![
        title("Name your agent"),
        subtitle("This is how Hearth introduces itself."),
        Line::from(""),
    ];
    let cursor = push_field(
        &mut lines,
        "Name",
        &app.onboarding.agent_name,
        true,
        area.width,
        "Hearth",
    );
    frame.render_widget(Paragraph::new(lines), area);
    set_cursor(frame, area, cursor);
}

fn extras(app: &App, frame: &mut Frame, area: Rect) {
    let onboarding = &app.onboarding;
    let mut lines = vec![
        title("Optional services"),
        subtitle("Skip any you do not use. They can be changed later in settings.json."),
        Line::from(""),
        section("Image generation · OpenRouter"),
    ];
    let mut cursor = None;
    push_toggle(
        &mut lines,
        onboarding.extras_focus == 0,
        onboarding.image,
        "Enable image generation",
    );
    if let Some(found) = push_optional_field(
        &mut lines,
        "API key",
        &onboarding.image_key,
        onboarding.image,
        onboarding.extras_focus == 1,
        area.width,
        "sk-or-...",
    ) {
        cursor = Some(found);
    }
    if let Some(found) = push_optional_field(
        &mut lines,
        "Image model",
        &onboarding.image_model,
        onboarding.image,
        onboarding.extras_focus == 2,
        area.width,
        "openai/gpt-image-2.6-sunburst",
    ) {
        cursor = Some(found);
    }
    lines.push(Line::from(""));
    lines.push(section("Transcription · Groq"));
    push_toggle(
        &mut lines,
        onboarding.extras_focus == 3,
        onboarding.stt,
        "Enable transcription",
    );
    if let Some(found) = push_optional_field(
        &mut lines,
        "API key",
        &onboarding.stt_key,
        onboarding.stt,
        onboarding.extras_focus == 4,
        area.width,
        "gsk_...",
    ) {
        cursor = Some(found);
    }
    if let Some(found) = push_optional_field(
        &mut lines,
        "Transcription model",
        &onboarding.stt_model,
        onboarding.stt,
        onboarding.extras_focus == 5,
        area.width,
        "whisper-large-v3-turbo",
    ) {
        cursor = Some(found);
    }
    lines.push(Line::from(""));
    lines.push(section("Speech · ElevenLabs"));
    push_toggle(
        &mut lines,
        onboarding.extras_focus == 6,
        onboarding.tts,
        "Enable speech",
    );
    if let Some(found) = push_optional_field(
        &mut lines,
        "API key",
        &onboarding.tts_key,
        onboarding.tts,
        onboarding.extras_focus == 7,
        area.width,
        "xi-...",
    ) {
        cursor = Some(found);
    }
    if let Some(found) = push_optional_field(
        &mut lines,
        "Voice id",
        &onboarding.tts_voice,
        onboarding.tts,
        onboarding.extras_focus == 8,
        area.width,
        "voice id",
    ) {
        cursor = Some(found);
    }
    if let Some(found) = push_optional_field(
        &mut lines,
        "Speech model",
        &onboarding.tts_model,
        onboarding.tts,
        onboarding.extras_focus == 9,
        area.width,
        "eleven_v4",
    ) {
        cursor = Some(found);
    }
    lines.push(Line::from(""));
    let continue_focused = onboarding.extras_focus == 10;
    lines.push(if continue_focused {
        Line::styled(" ▸ Continue", theme::selected())
    } else {
        Line::styled("   Continue", theme::muted())
    });
    frame.render_widget(Paragraph::new(lines), area);
    set_cursor(frame, area, cursor);
}

fn daemon(app: &App, frame: &mut Frame, area: Rect) {
    let onboarding = &app.onboarding;
    let rows = [
        format!("Start: {}", onboarding.start_label()),
        format!(
            "Keep running after this window closes: {}",
            if onboarding.keep_running { "Yes" } else { "No" }
        ),
        "Continue".to_owned(),
    ];
    let mut lines = vec![
        title("Run Hearth"),
        subtitle("Choose when the daemon starts and whether it outlives this window."),
        Line::from(""),
    ];
    for (index, row) in rows.iter().enumerate() {
        let selected = index == onboarding.focus;
        let marker = if selected { "▸" } else { " " };
        let text = format!(" {marker} {row}");
        lines.push(if selected {
            Line::styled(text, theme::selected())
        } else {
            Line::styled(text, theme::text())
        });
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "At login and startup, Hearth installs a systemd user service.",
        theme::faint(),
    ));
    lines.push(Line::styled(
        "Keep running only affects a daemon started by this window.",
        theme::faint(),
    ));
    frame.render_widget(Paragraph::new(lines), area);
}

fn review(app: &App, frame: &mut Frame, area: Rect) {
    let onboarding = &app.onboarding;
    let media = [
        if onboarding.image { "images" } else { "" },
        if onboarding.stt { "transcription" } else { "" },
        if onboarding.tts { "speech" } else { "" },
    ]
    .into_iter()
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>()
    .join(", ");
    let lines = vec![
        title("Ready to create your home"),
        subtitle("Review the setup before Hearth writes settings and starts the daemon."),
        Line::from(""),
        summary("Telegram token", onboarding.token.masked()),
        summary("Telegram id", onboarding.user_id.value.clone()),
        summary("Provider", onboarding.base_url.value.clone()),
        summary(
            "Model",
            onboarding.selected_model.clone().unwrap_or_default(),
        ),
        summary(
            "Context",
            match (&onboarding.model_limits, onboarding.model_limits_loading) {
                (Some(limits), _) => format!("{} tokens (models.dev)", limits.context_window),
                (None, true) => "Looking up at models.dev...".to_owned(),
                (None, false) if onboarding.model_limits_error.is_some() => {
                    "Unavailable; using the default".to_owned()
                }
                (None, false) => "Provider default".to_owned(),
            },
        ),
        summary("Thinking", THINKING[onboarding.thinking].1.to_owned()),
        summary("Agent name", onboarding.agent_name.value.clone()),
        summary("Start", onboarding.start_label().to_owned()),
        summary(
            "Keep running",
            if onboarding.keep_running { "Yes" } else { "No" }.to_owned(),
        ),
        summary(
            "Optional",
            if media.is_empty() {
                "none"
            } else {
                media.as_str()
            }
            .to_owned(),
        ),
        Line::from(""),
        Line::styled(
            "Press Enter to create the home and start Hearth.",
            theme::good(),
        ),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn creating(frame: &mut Frame, area: Rect) {
    let lines = vec![
        title("Creating your Hearth home..."),
        Line::from(""),
        subtitle(
            "Writing settings, building the database, applying the start mode, and starting the daemon.",
        ),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn failed(app: &App, frame: &mut Frame, area: Rect) {
    let error = app
        .onboarding
        .error
        .clone()
        .unwrap_or_else(|| "Setup failed.".to_owned());
    let lines = vec![
        Line::styled(
            "Setup could not finish",
            theme::bad().add_modifier(Modifier::BOLD),
        ),
        Line::from(""),
        Line::styled(error, theme::bad()),
        Line::from(""),
        Line::styled(
            "Enter retries. Esc goes back to the review.",
            theme::faint(),
        ),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn render_footer(app: &App, frame: &mut Frame, area: Rect) {
    let hints = match app.onboarding.step {
        Step::Welcome => theme::hints(&[("enter", "begin"), ("esc", "quit")]),
        Step::Creating => theme::hints(&[("wait", "creating the home")]),
        Step::Failed => theme::hints(&[("enter", "retry"), ("esc", "back")]),
        Step::Provider | Step::Thinking => {
            theme::hints(&[("↑↓", "choose"), ("enter", "next"), ("esc", "back")])
        }
        Step::Daemon => theme::hints(&[("↑↓", "choose"), ("enter", "change"), ("esc", "back")]),
        Step::Review => theme::hints(&[("enter", "create"), ("esc", "back")]),
        Step::Extras => theme::hints(&[("enter", "toggle"), ("tab", "field"), ("esc", "back")]),
        _ => theme::hints(&[("enter", "next"), ("tab", "field"), ("esc", "back")]),
    };
    let mut lines = vec![hints];
    if let Some(error) = &app.onboarding.error {
        lines.push(Line::styled(error.clone(), theme::bad()));
    } else if has_fields(app.onboarding.step) {
        lines.push(Line::styled(PASTE_HINT, theme::faint()));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn has_fields(step: Step) -> bool {
    matches!(
        step,
        Step::Telegram | Step::Credentials | Step::Model | Step::Agent | Step::Extras
    )
}

fn title(text: &str) -> Line<'static> {
    Line::styled(text.to_owned(), theme::text().add_modifier(Modifier::BOLD))
}

fn subtitle(text: &str) -> Line<'static> {
    Line::styled(text.to_owned(), theme::muted())
}

fn section(text: &str) -> Line<'static> {
    Line::styled(text.to_owned(), theme::accent_style())
}

fn summary(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<17}"), theme::muted()),
        Span::styled(short(&value, 48), theme::text()),
    ])
}

fn push_toggle(lines: &mut Vec<Line<'static>>, focused: bool, enabled: bool, label: &str) {
    let marker = if focused { "▸" } else { " " };
    let check = if enabled { "✓" } else { " " };
    let text = format!(" {marker} [{check}] {label}");
    lines.push(if focused {
        Line::styled(text, theme::selected())
    } else if enabled {
        Line::styled(text, theme::good())
    } else {
        Line::styled(text, theme::muted())
    });
}

fn push_field(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    input: &TextInput,
    focused: bool,
    width: u16,
    placeholder: &str,
) -> Option<(u16, u16)> {
    let row = lines.len() as u16;
    let field = field_line(label, input, focused, width, placeholder);
    lines.push(field);
    focused.then_some((field_cursor_col(label, input, focused, width), row))
}

fn push_optional_field(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    input: &TextInput,
    enabled: bool,
    focused: bool,
    width: u16,
    placeholder: &str,
) -> Option<(u16, u16)> {
    if !enabled {
        let text = format!("   {label:<width$} {placeholder}", width = LABEL_WIDTH);
        lines.push(Line::styled(text, theme::faint()));
        return None;
    }
    push_field(lines, label, input, focused, width, placeholder)
}

fn field_line(
    label: &str,
    input: &TextInput,
    focused: bool,
    width: u16,
    placeholder: &str,
) -> Line<'static> {
    let marker = if focused { "▸" } else { " " };
    let label_style = if focused {
        theme::accent_style()
    } else {
        theme::muted()
    };
    let prefix = format!("{marker} {label:<LABEL_WIDTH$} ");
    let available = width
        .saturating_sub(UnicodeWidthStr::width(prefix.as_str()) as u16 + 2)
        .max(4) as usize;
    let empty = input.value.is_empty();
    let (visible, _) = visible_value(input, available);
    let shown = if empty && !focused {
        placeholder.to_owned()
    } else {
        visible
    };
    let style = if focused {
        theme::field_focus()
    } else if empty {
        Style::default().fg(theme::FAINT).bg(theme::FIELD_BG)
    } else {
        theme::field()
    };
    Line::from(vec![
        Span::styled(prefix, label_style),
        Span::styled(format!(" {shown:<available$} "), style),
    ])
}

fn field_cursor_col(label: &str, input: &TextInput, focused: bool, width: u16) -> u16 {
    let marker = if focused { "▸" } else { " " };
    let prefix = format!("{marker} {label:<LABEL_WIDTH$} ");
    let prefix_width = UnicodeWidthStr::width(prefix.as_str());
    let available = width.saturating_sub(prefix_width as u16 + 2).max(4) as usize;
    let (_, cursor) = visible_value(input, available);
    prefix_width as u16 + 1 + cursor
}

fn readonly_field(label: &str, value: &str, width: u16) -> Line<'static> {
    let prefix = format!("  {label:<LABEL_WIDTH$} ");
    let available = width
        .saturating_sub(UnicodeWidthStr::width(prefix.as_str()) as u16 + 2)
        .max(4) as usize;
    Line::from(vec![
        Span::styled(prefix, theme::muted()),
        Span::styled(
            format!(" {:<available$} ", short(value, available)),
            theme::faint(),
        ),
    ])
}

fn visible_value(input: &TextInput, max_width: usize) -> (String, u16) {
    let masked = input.masked();
    let chars: Vec<char> = masked.chars().collect();
    let cursor = input.cursor.min(chars.len());
    let total = width_of(&chars);
    if total <= max_width {
        return (masked, width_of(&chars[..cursor]) as u16);
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

fn set_cursor(frame: &mut Frame, area: Rect, cursor: Option<(u16, u16)>) {
    if let Some((column, row)) = cursor {
        frame.set_cursor_position((area.x + column, area.y + row));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_value_scrolls_to_keep_the_cursor_visible() {
        let mut input = TextInput::with_value("abcdefghij", false);
        input.cursor = 8;
        let (shown, cursor) = visible_value(&input, 6);
        assert_eq!(shown, "…efgh…");
        assert_eq!(cursor, 5);
        assert!(cursor as usize <= 6);
    }
}
