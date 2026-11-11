pub mod activity;
pub mod dashboard;
pub mod doctor;
pub mod editor;
pub mod memory;
pub mod onboarding;
pub mod prompt;
pub mod settings;
pub mod settings_form;
pub mod skills;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::{App, DaemonState, Screen, Tab};
use crate::theme;

pub fn render(app: &App, frame: &mut Frame) {
    if app.screen == Screen::Onboarding {
        onboarding::render(app, frame);
        if app.help {
            help(frame);
        }
        return;
    }
    let area = frame.area();
    if area.width < 80 || area.height < 24 {
        resize(frame, area);
        return;
    }
    if let Some(session) = &app.editor {
        editor::render(session, frame, area);
        return;
    }
    let tab_height = tab_rows(area.width);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(tab_height),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area);
    header(app, frame, layout[0]);
    tabs(app, frame, layout[1]);
    match app.tab {
        Tab::Overview => dashboard::render(app, frame, layout[2]),
        Tab::Activity => activity::render(app, frame, layout[2]),
        Tab::Doctor => doctor::render(app, frame, layout[2]),
        Tab::Skills => skills::render(app, frame, layout[2]),
        Tab::Prompt => prompt::render(app, frame, layout[2]),
        Tab::Settings => match &app.settings_form {
            Some(form) => settings_form::render(form, frame, layout[2]),
            None => settings::render(app, frame, layout[2]),
        },
        Tab::Memory => memory::render(app, frame, layout[2]),
    }
    footer(app, frame, layout[3]);
    if app.help {
        help(frame);
    }
}

fn header(app: &App, frame: &mut Frame, area: Rect) {
    let (label, style) = match &app.daemon {
        DaemonState::Running => ("running".to_owned(), theme::good()),
        DaemonState::Starting => ("starting".to_owned(), theme::warn()),
        DaemonState::Stopped => ("stopped".to_owned(), theme::muted()),
        DaemonState::Error(message) => (format!("error: {}", short(message, 40)), theme::bad()),
    };
    let model = app
        .status
        .as_ref()
        .and_then(|status| status["model"].as_str())
        .or_else(|| {
            app.settings_data
                .as_ref()
                .and_then(|settings| settings.models.as_ref())
                .map(|models| models.model.as_str())
        })
        .unwrap_or("not configured");
    let uptime = app
        .status
        .as_ref()
        .and_then(|status| status["uptime_seconds"].as_u64())
        .map(format_uptime)
        .unwrap_or_default();
    let line = Line::from(vec![
        Span::styled("  HEARTH  ", theme::accent_style()),
        Span::raw(" "),
        Span::styled("●", style),
        Span::raw(" "),
        Span::styled(label, style),
        Span::raw("  ·  "),
        Span::styled(model, theme::text()),
        if uptime.is_empty() {
            Span::raw("")
        } else {
            Span::raw(format!("  ·  up {uptime}"))
        },
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border());
    frame.render_widget(Paragraph::new(line).block(block), area);
}

fn tabs(app: &App, frame: &mut Frame, area: Rect) {
    let labels: Vec<String> = Tab::ALL
        .iter()
        .enumerate()
        .map(|(index, tab)| format!(" {} {} ", index + 1, tab.title()))
        .collect();
    let rows: Vec<Vec<usize>> = if area.height >= 2 {
        let middle = Tab::ALL.len().div_ceil(2);
        vec![(0..middle).collect(), (middle..Tab::ALL.len()).collect()]
    } else {
        vec![(0..Tab::ALL.len()).collect()]
    };
    let lines: Vec<Line> = rows
        .into_iter()
        .map(|row| {
            let mut spans = Vec::new();
            for (position, index) in row.into_iter().enumerate() {
                if position > 0 {
                    spans.push(Span::raw("   "));
                }
                let label = labels[index].clone();
                if Tab::ALL[index] == app.tab {
                    spans.push(Span::styled(label, theme::selected()));
                } else {
                    spans.push(Span::styled(label, theme::muted()));
                }
            }
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn tab_rows(width: u16) -> u16 {
    let total = Tab::ALL
        .iter()
        .map(|tab| tab.title().len() + 4)
        .sum::<usize>()
        + Tab::ALL.len().saturating_sub(1) * 3;
    if total > width as usize { 2 } else { 1 }
}

fn footer(app: &App, frame: &mut Frame, area: Rect) {
    let hints = match app.tab {
        Tab::Overview => theme::hints(&[("tab", "sections"), ("?", "help"), ("q", "quit")]),
        Tab::Activity => theme::hints(&[
            ("↑↓", "select"),
            ("p", "pause"),
            ("g", "top"),
            ("q", "quit"),
        ]),
        Tab::Doctor => theme::hints(&[
            ("enter", "offline check"),
            ("l", "live check"),
            ("↑↓", "scroll"),
            ("q", "quit"),
        ]),
        Tab::Skills => theme::hints(&[
            ("↑↓", "select"),
            ("enter", "open"),
            ("e", "edit user skill"),
            ("n", "new skill"),
            ("q", "quit"),
        ]),
        Tab::Prompt => theme::hints(&[
            ("r", "raw/system"),
            ("o", "refresh"),
            ("↑↓", "scroll"),
            ("q", "quit"),
        ]),
        Tab::Settings => theme::hints(&[
            ("↑↓", "select"),
            ("enter", "change"),
            ("r", "reset to default"),
            ("q", "quit"),
        ]),
        Tab::Memory => theme::hints(&[
            ("↑↓", "select"),
            ("/", "search"),
            ("r", "refresh"),
            ("q", "quit"),
        ]),
    };
    let banner = app
        .settings
        .message
        .clone()
        .or_else(|| app.banner.clone())
        .unwrap_or_default();
    let style = if banner.is_empty() {
        theme::muted()
    } else {
        theme::warn()
    };
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(theme::border());
    frame.render_widget(
        Paragraph::new(vec![hints, Line::styled(format!(" {banner}"), style)]).block(block),
        area,
    );
}

fn resize(frame: &mut Frame, area: Rect) {
    let message = Paragraph::new(vec![
        Line::styled("Hearth needs a larger terminal", theme::accent_style()),
        Line::from("Please resize to at least 80 columns by 24 rows."),
    ])
    .wrap(Wrap { trim: true })
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme::border_focus()),
    );
    frame.render_widget(message, area);
}

fn help(frame: &mut Frame) {
    let area = frame.area();
    let width = area.width.saturating_sub(8).min(70);
    let height = 20.min(area.height.saturating_sub(2));
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let lines = vec![
        Line::styled("Hearth keyboard help", theme::accent_style()),
        Line::from(""),
        Line::from("  tab / shift-tab   move between sections"),
        Line::from("  1 .. 7            jump to a section"),
        Line::from("  q                 quit (Hearth respects the keep-running setting)"),
        Line::from("  ctrl-c            quit"),
        Line::from("  ?                 close this help"),
        Line::from(""),
        Line::styled("  Make it yours", theme::accent_style()),
        Line::from("    Settings   every option is shown; r resets a row"),
        Line::from("               edit SOUL.md and HEARTH.md from Settings"),
        Line::from("    Skills     e edits your own skill files"),
        Line::from("    Memory     press / to search saved memories"),
        Line::from(""),
        Line::styled(
            "  Conversations happen in Telegram. This window controls and observes the daemon.",
            theme::muted(),
        ),
    ];
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_focus())
        .title(theme::heading("Help"));
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}
pub fn format_uptime(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{seconds}s")
    }
}

pub fn event_summary(value: &serde_json::Value) -> String {
    let kind = value["kind"].as_str().unwrap_or("event");
    let data = &value["data"];
    match kind {
        "text_delta" => format!(
            "Reply · {}",
            short(data["text"].as_str().unwrap_or_default(), 100)
        ),
        "thinking_delta" => format!(
            "Thinking · {}",
            short(data["text"].as_str().unwrap_or_default(), 100)
        ),
        "tool_call_started" => format!("Tool call · {}", data["name"].as_str().unwrap_or("tool")),
        "tool_result" => {
            let prefix = if data["is_error"].as_bool().unwrap_or(false) {
                "Tool error"
            } else {
                "Tool result"
            };
            format!(
                "{prefix} · {}",
                short(data["content"].as_str().unwrap_or_default(), 100)
            )
        }
        "usage" => format!(
            "Usage · {} in · {} out",
            count(data["input_tokens"].as_u64().unwrap_or(0)),
            count(data["output_tokens"].as_u64().unwrap_or(0))
        ),
        "run_started" => "Run started".to_owned(),
        "run_settled" => format!("Run {}", data["status"].as_str().unwrap_or("settled")),
        "turn_started" => format!("Turn {} started", data["turn"]),
        "task_step" => format!(
            "Step · {} (turn {})",
            data["phase"].as_str().unwrap_or("working"),
            data["turn"]
        ),
        _ => kind.to_owned(),
    }
}

pub fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::new();
    for (index, character) in digits.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(character);
    }
    grouped.chars().rev().collect()
}

pub fn short(text: &str, width: usize) -> String {
    let mut value: String = text.chars().take(width).collect();
    if text.chars().count() > width {
        value.push('…');
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Screen, Tab};
    use crate::daemon_client::DaemonState;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use hearth_config::Daemon;
    use hearth_runtime::{InitOptions, init};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use serde_json::json;
    use tempfile::TempDir;

    fn fresh_app() -> (TempDir, App) {
        let home = TempDir::new().expect("temp home");
        let app = App::new(home.path().to_path_buf());
        (home, app)
    }

    fn initialized_app() -> (TempDir, App) {
        let home = TempDir::new().expect("temp home");
        init(
            home.path(),
            InitOptions {
                telegram_token: "123:fake-token".to_owned(),
                allowed_user_ids: vec![42],
                base_url: "https://example.com/v1".to_owned(),
                api_key: "fake-key".to_owned(),
                model: "fake-model".to_owned(),
                thinking: "default".to_owned(),
                agent_name: "Hearth".to_owned(),
                timezone: Some("UTC".to_owned()),
                media: None,
                context_window: None,
                daemon: Daemon::default(),
            },
        )
        .expect("init");
        let app = App::new(home.path().to_path_buf());
        (home, app)
    }

    fn buffer_text(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| render(app, frame)).expect("draw");
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..height {
            for x in 0..width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn welcome_screen_renders_the_setup_entry() {
        let (_home, app) = fresh_app();
        let text = buffer_text(&app, 100, 32);
        assert!(
            text.contains("A durable personal agent, built with Rust."),
            "{text}"
        );
        assert!(text.contains("Press Enter to begin"), "{text}");
    }

    #[test]
    fn thinking_screen_lists_xhigh_and_max() {
        let (_home, mut app) = fresh_app();
        app.onboarding.step = crate::onboarding::Step::Thinking;
        let text = buffer_text(&app, 100, 32);
        assert!(text.contains("Extra high"), "{text}");
        assert!(text.contains("Maximum"), "{text}");
    }

    #[test]
    fn onboarding_cursor_sits_inside_the_focused_field() {
        let (_home, mut app) = fresh_app();
        app.onboarding.step = crate::onboarding::Step::Telegram;
        app.onboarding.focus = 1;
        app.onboarding.user_id.value = "123456789".to_owned();
        app.onboarding.user_id.cursor = 4;
        let backend = TestBackend::new(100, 32);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| render(&app, frame)).expect("draw");
        let position = terminal.backend().cursor_position();
        let cell = &terminal.backend().buffer()[(position.x, position.y)];
        assert_eq!(cell.symbol(), "5");
        assert_eq!(cell.bg, crate::theme::FIELD_FOCUS_BG);
    }

    #[test]
    fn provider_screen_lists_the_presets() {
        let (_home, mut app) = fresh_app();
        app.onboarding.step = crate::onboarding::Step::Provider;
        let text = buffer_text(&app, 100, 32);
        assert!(text.contains("DeepSeek"), "{text}");
        assert!(text.contains("Command Code"), "{text}");
        assert!(text.contains("Other OpenAI-compatible"), "{text}");
    }

    #[test]
    fn dashboard_renders_status_and_tabs() {
        let (_home, mut app) = initialized_app();
        app.daemon = DaemonState::Running;
        app.local_offset = time::UtcOffset::UTC;
        app.events
            .push_back(json!({"ts": 0, "kind": "run_started", "task_id": "t", "data": {}}));
        app.status = Some(json!({
            "model": "fake-model",
            "uptime_seconds": 90,
            "tasks_running": 0,
            "tasks_asleep": 0,
            "usage": {
                "today": {"input_tokens": 10, "output_tokens": 5, "cache_read_tokens": 4},
                "week": {"input_tokens": 2000, "output_tokens": 800, "cache_read_tokens": 0},
                "month": {"input_tokens": 30000, "output_tokens": 9000, "cache_read_tokens": 0},
                "total": {"input_tokens": 400000, "output_tokens": 100000, "cache_read_tokens": 0}
            },
            "output_tokens": 5,
            "cache_hit_rate": 0.5,
            "memory_items": 3,
            "standing_memory_version": 1,
        }));
        let text = buffer_text(&app, 110, 34);
        assert!(text.contains("Overview"), "{text}");
        assert!(text.contains("Activity"), "{text}");
        assert!(text.contains("Runtime"), "{text}");
        assert!(text.contains("fake-model"), "{text}");
        assert!(text.contains("Usage"), "{text}");
        assert!(text.contains("1 week"), "{text}");
        assert!(text.contains("1 month"), "{text}");
        assert!(text.contains("total"), "{text}");
        assert!(text.contains("cache"), "{text}");
        assert!(text.contains("Work & memory"), "{text}");
        assert!(text.contains("Connections"), "{text}");
        assert!(text.contains("Telegram"), "{text}");
        assert!(text.contains("turn limit 120"), "{text}");
        assert!(text.contains("00:00:00"), "{text}");
    }

    #[test]
    fn hovering_a_setting_shows_its_tooltip() {
        let (_home, mut app) = initialized_app();
        app.handle_key(KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE));
        let index = app
            .settings_form
            .as_ref()
            .expect("form")
            .rows
            .iter()
            .position(|row| row.label == "Max turns")
            .expect("max turns row");
        app.settings_form.as_mut().expect("form").selected = index;
        let text = buffer_text(&app, 110, 34);
        assert!(text.contains("Each tool call"), "{text}");
    }

    #[test]
    fn the_timezone_setting_renders_a_picker() {
        let (_home, mut app) = initialized_app();
        app.handle_key(KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE));
        let index = app
            .settings_form
            .as_ref()
            .expect("form")
            .rows
            .iter()
            .position(|row| row.label == "Timezone")
            .expect("timezone row");
        app.settings_form.as_mut().expect("form").selected = index;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let text = buffer_text(&app, 110, 34);
        assert!(text.contains("Timezone"), "{text}");
        assert!(text.contains("UTC"), "{text}");
        assert!(text.contains("choose"), "{text}");
    }

    #[test]
    fn memory_tab_shows_saved_memories() {
        use hearth_core::Harness;
        use hearth_memory::{MemoryKind, MemoryService, NewMemoryInput, SourceKind};

        let home = TempDir::new().expect("temp home");
        init(
            home.path(),
            InitOptions {
                telegram_token: "123:fake-token".to_owned(),
                allowed_user_ids: vec![42],
                base_url: "https://example.com/v1".to_owned(),
                api_key: "fake-key".to_owned(),
                model: "fake-model".to_owned(),
                thinking: "default".to_owned(),
                agent_name: "Hearth".to_owned(),
                timezone: Some("UTC".to_owned()),
                media: None,
                context_window: None,
                daemon: Daemon::default(),
            },
        )
        .expect("init");
        {
            let harness = Harness::open(home.path()).expect("harness");
            let memory = MemoryService::new(harness.session().clone());
            memory
                .save(NewMemoryInput {
                    content: "The release checklist lives in docs/.".to_owned(),
                    kind: MemoryKind::Fact,
                    tags: vec!["release".to_owned()],
                    importance: 80,
                    source_kind: SourceKind::User,
                    source_ref: None,
                    supersedes: None,
                    valid_from: None,
                    valid_to: None,
                })
                .expect("memory");
        }
        let mut app = App::new(home.path().to_path_buf());
        app.tab = Tab::Memory;
        app.memory.reload(home.path());
        let text = buffer_text(&app, 110, 34);
        assert!(text.contains("Memory"), "{text}");
        assert!(text.contains("release checklist"), "{text}");
        assert!(text.contains("FACT"), "{text}");
    }

    #[test]
    fn settings_tab_shows_the_form_without_a_shortcut() {
        let (_home, mut app) = initialized_app();
        app.handle_key(KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE));
        assert_eq!(app.tab, Tab::Settings);
        assert!(app.settings_form.is_some());
        let text = buffer_text(&app, 110, 38);
        assert!(text.contains("All settings"), "{text}");
        assert!(text.contains("Agent"), "{text}");
        assert!(text.contains("Model provider"), "{text}");
        assert!(text.contains("Keep running after close"), "{text}");
        assert!(text.contains("Edit SOUL.md"), "{text}");
        assert!(text.contains("Edit HEARTH.md"), "{text}");
        assert!(text.contains("Context window"), "{text}");
        assert!(text.contains("Thinking"), "{text}");
    }

    #[test]
    fn settings_can_open_the_soul_editor() {
        let (_home, mut app) = initialized_app();
        app.handle_key(KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE));
        let index = app
            .settings_form
            .as_ref()
            .expect("form")
            .rows
            .iter()
            .position(|row| row.label == "Edit SOUL.md")
            .expect("SOUL action");
        app.settings_form.as_mut().expect("form").selected = index;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.editor.is_some());
        let text = buffer_text(&app, 110, 34);
        assert!(text.contains("Edit · SOUL.md"), "{text}");
    }

    #[test]
    fn editing_a_setting_shows_a_tooltip() {
        let (_home, mut app) = initialized_app();
        app.handle_key(KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE));
        let index = app
            .settings_form
            .as_ref()
            .expect("form")
            .rows
            .iter()
            .position(|row| row.label == "Max turns")
            .expect("max turns row");
        app.settings_form.as_mut().expect("form").selected = index;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let text = buffer_text(&app, 110, 34);
        assert!(text.contains("Tip"), "{text}");
        assert!(text.contains("model requests"), "{text}");
    }

    #[test]
    fn settings_form_saves_a_friendly_edit_and_only_keeps_changes() {
        let (home, mut app) = initialized_app();
        app.handle_key(KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE));
        let index = app
            .settings_form
            .as_ref()
            .expect("form")
            .rows
            .iter()
            .position(|row| row.label == "Name")
            .expect("name row");
        app.settings_form.as_mut().expect("form").selected = index;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        {
            let edit = app
                .settings_form
                .as_mut()
                .expect("form")
                .edit
                .as_mut()
                .expect("edit");
            edit.input.value = "Ember".to_owned();
            edit.input.cursor = 5;
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let settings = hearth_config::load(home.path()).expect("settings");
        assert_eq!(settings.identity.agent_name, "Ember");
        let text = std::fs::read_to_string(home.path().join("settings.json")).expect("settings");
        let value: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert!(value.get("agent").is_none(), "{text}");
        assert!(value.get("tasks").is_none(), "{text}");
        assert_eq!(value["identity"]["agent_name"], "Ember");
        assert!(value["identity"].get("timezone").is_none(), "{text}");
        assert!(matches!(
            app.settings.message.as_deref(),
            Some(message) if message.contains("saved")
        ));
    }

    #[test]
    fn the_raw_editor_also_compacts_defaults_on_save() {
        let (home, mut app) = initialized_app();
        app.handle_key(KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE));
        let index = app
            .settings_form
            .as_ref()
            .expect("form")
            .rows
            .iter()
            .position(|row| row.label == "Edit settings.json")
            .expect("raw editor row");
        app.settings_form.as_mut().expect("form").selected = index;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let path = home.path().join("settings.json");
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("settings")).expect("json");
        value["identity"] = json!({
            "agent_name": "Hearth",
            "timezone": "UTC",
            "soul_file": "SOUL.md"
        });
        value["agent"] = json!({"max_turns": 120});
        app.editor
            .as_mut()
            .expect("editor")
            .text
            .replace_text(serde_json::to_string_pretty(&value).expect("json"));
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        let text = std::fs::read_to_string(&path).expect("saved settings");
        let value: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert!(value.get("identity").is_none(), "{text}");
        assert!(value.get("agent").is_none(), "{text}");
        assert_eq!(value["models"]["model"], "fake-model");
    }

    #[test]
    fn skills_tab_creates_a_new_user_skill() {
        let (_home, mut app) = initialized_app();
        app.tab = Tab::Skills;
        app.skills.reload();
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        {
            let input = app.skills.new_name.as_mut().expect("name input");
            input.value = "Research Notes".to_owned();
            input.cursor = input.value.chars().count();
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.editor.is_some());
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(
            app.skills
                .skills
                .iter()
                .any(|skill| skill.name == "research-notes")
        );
    }

    #[test]
    fn narrow_dashboard_wraps_the_seven_tabs() {
        let (_home, app) = initialized_app();
        let text = buffer_text(&app, 80, 24);
        assert!(text.contains("1 Overview"), "{text}");
        assert!(text.contains("3 Memory"), "{text}");
        assert!(text.contains("4 Skills"), "{text}");
        assert!(text.contains("6 Doctor"), "{text}");
        assert!(text.contains("7 Settings"), "{text}");
    }
    #[test]
    fn small_terminal_shows_a_resize_message() {
        let (_home, app) = initialized_app();
        let text = buffer_text(&app, 60, 20);
        assert!(text.contains("larger terminal"), "{text}");
    }

    #[test]
    fn prompt_tab_renders_the_saved_sections() {
        let (_home, mut app) = initialized_app();
        app.tab = Tab::Prompt;
        app.prompt.document = Some(json!({
            "identity": "SOUL TEXT",
            "guidance": "PROJECT RULES",
            "runtime": "RUNTIME NOTE"
        }));
        let text = buffer_text(&app, 100, 32);
        assert!(text.contains("<identity>"), "{text}");
        assert!(text.contains("SOUL TEXT"), "{text}");
        assert!(text.contains("<project_instructions>"), "{text}");
        assert!(text.contains("<runtime>"), "{text}");
    }

    #[test]
    fn prompt_scroll_stops_at_the_end_and_resets_on_raw_toggle() {
        let (_home, mut app) = initialized_app();
        app.tab = Tab::Prompt;
        let body = (0..60)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.prompt.document = Some(json!({"identity": body}));
        let _ = buffer_text(&app, 100, 24);
        let max = app.prompt.max_scroll.get();
        assert!(max > 0, "expected a scroll range");
        for _ in 0..100 {
            app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        assert_eq!(app.prompt.scroll, max);
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        assert!(app.prompt.raw);
        assert_eq!(app.prompt.scroll, 0);
    }

    #[test]
    fn settings_tab_renders_the_start_modes() {
        let (_home, mut app) = initialized_app();
        app.handle_key(KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE));
        let text = buffer_text(&app, 100, 34);
        assert!(text.contains("Start"), "{text}");
        assert!(text.contains("When Hearth opens"), "{text}");
        assert!(text.contains("Keep running after close"), "{text}");
        assert!(text.contains("fake-model"), "{text}");
    }

    #[test]
    fn key_routing_switches_tabs_and_quits() {
        let (_home, mut app) = initialized_app();
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.tab, Tab::Activity);
        app.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
        assert_eq!(app.tab, Tab::Memory);
        app.handle_key(KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE));
        assert_eq!(app.tab, Tab::Settings);
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(app.quit);
    }

    #[test]
    fn skills_tab_renders_the_library() {
        let home = TempDir::new().expect("temp home");
        std::fs::create_dir_all(home.path().join("skills")).expect("skills dir");
        std::fs::write(
            home.path().join("skills/research.md"),
            "# Research\n\nSearch carefully.\n",
        )
        .expect("write skill");
        let mut app = App::new(home.path().to_path_buf());
        app.screen = Screen::Dashboard;
        app.tab = Tab::Skills;
        app.skills.reload();
        let text = buffer_text(&app, 100, 32);
        assert!(text.contains("research"), "{text}");
    }

    #[test]
    fn dashboard_is_shown_for_an_existing_home() {
        let (_home, app) = initialized_app();
        assert_eq!(app.screen, Screen::Dashboard);
    }
}
