//! The Hearth TUI application.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use crossterm::execute;
use ratatui::DefaultTerminal;
use serde_json::Value;
use time::UtcOffset;

use hearth_config::{Daemon, SETTINGS_FILE, Settings};
use hearth_runtime::inspect::MemoryRecord;
use hearth_runtime::skills::{Skill, SkillLibrary, SkillScope, SkillStats};

pub use crate::daemon_client::DaemonState;
use crate::daemon_client::{self, Ownership};
use crate::editor::{self, EditorKind, OpenEditor, TextEditor};
use crate::onboarding::{Action, Onboarding, TextInput};
use crate::settings_form::{FormAction, SettingsForm};
use crate::ui;
use crate::workers::{AppEvent, DaemonAction, OnboardResult, Workers};

pub const MAX_EVENTS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Activity,
    Doctor,
    Skills,
    Prompt,
    Settings,
    Memory,
}

impl Tab {
    pub const ALL: [Tab; 7] = [
        Tab::Overview,
        Tab::Activity,
        Tab::Memory,
        Tab::Skills,
        Tab::Prompt,
        Tab::Doctor,
        Tab::Settings,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Activity => "Activity",
            Tab::Doctor => "Doctor",
            Tab::Skills => "Skills",
            Tab::Prompt => "Prompt",
            Tab::Settings => "Settings",
            Tab::Memory => "Memory",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }

    fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    fn previous(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Onboarding,
    Dashboard,
}

#[derive(Default)]
pub struct DoctorView {
    pub running: bool,
    pub live: bool,
    pub output: Option<String>,
    pub scroll: usize,
}

pub struct SkillsView {
    pub library: SkillLibrary,
    pub new_name: Option<TextInput>,
    pub skills: Vec<Skill>,
    pub selected: usize,
    pub body: Option<String>,
    pub scroll: usize,
    pub max_scroll: Cell<usize>,
    pub stats: Option<SkillStats>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct PromptView {
    /// Newest-first prompt/response exchanges loaded by the worker. The view
    /// always shows index 0.
    pub exchanges: Vec<Value>,
    /// Selected prompt section.
    pub selected: usize,
    /// Sections the user expanded to full length.
    pub expanded: BTreeSet<usize>,
    pub raw: bool,
    pub scroll: usize,
    pub max_scroll: Cell<usize>,
    pub loading: bool,
    pub(crate) cache: RefCell<Option<crate::ui::prompt::PromptCache>>,
}

impl PromptView {
    pub fn current(&self) -> Option<&Value> {
        self.exchanges.first()
    }

    pub fn older(&self) -> Option<&Value> {
        self.exchanges.get(1)
    }
}

pub struct SettingsView {
    pub daemon: Daemon,
    pub focus: usize,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Soul,
    Guidance,
}

impl FileKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Soul => "SOUL.md",
            Self::Guidance => "HEARTH.md",
        }
    }

    fn editor_kind(self) -> EditorKind {
        match self {
            Self::Soul => EditorKind::Soul,
            Self::Guidance => EditorKind::Guidance,
        }
    }

    fn path(self, home: &Path) -> PathBuf {
        match self {
            Self::Soul => home.join("SOUL.md"),
            Self::Guidance => home.join("HEARTH.md"),
        }
    }
}

#[derive(Default)]
pub struct MemoryView {
    pub records: Vec<MemoryRecord>,
    pub selected: usize,
    pub filter: String,
    pub filter_mode: bool,
    pub error: Option<String>,
}

/// The store a memory row came from.
pub fn store_label(record: &MemoryRecord) -> &'static str {
    if record.standing {
        "standing"
    } else {
        "long-term"
    }
}

impl MemoryView {
    pub fn reload(&mut self, home: &Path) {
        match hearth_runtime::inspect::list_memories(home, 1000) {
            Ok(records) => {
                self.records = records;
                self.error = None;
                self.selected = self.selected.min(self.filtered().len().saturating_sub(1));
            }
            Err(error) => {
                self.records.clear();
                self.error = Some(error.to_string());
            }
        }
    }

    pub fn filtered(&self) -> Vec<&MemoryRecord> {
        let filter = self.filter.to_lowercase();
        self.records
            .iter()
            .filter(|record| {
                filter.is_empty()
                    || record.content.to_lowercase().contains(&filter)
                    || record.source.to_lowercase().contains(&filter)
                    || store_label(record).to_lowercase().contains(&filter)
                    || record
                        .tags
                        .iter()
                        .any(|tag| tag.to_lowercase().contains(&filter))
            })
            .collect()
    }

    pub fn selected_record(&self) -> Option<&MemoryRecord> {
        self.filtered().get(self.selected).copied()
    }
}

pub struct App {
    pub home: PathBuf,
    pub screen: Screen,
    pub tab: Tab,
    pub onboarding: Onboarding,
    pub daemon: DaemonState,
    pub status: Option<Value>,
    pub ownership: Option<Ownership>,
    pub events: VecDeque<Value>,
    pub local_offset: UtcOffset,
    pub activity_selected: usize,
    pub activity_paused: bool,
    pub doctor: DoctorView,
    pub skills: SkillsView,
    pub prompt: PromptView,
    pub settings: SettingsView,
    pub settings_data: Option<Settings>,
    pub settings_form: Option<SettingsForm>,
    pub memory: MemoryView,
    pub editor: Option<OpenEditor>,
    pub banner: Option<String>,
    pub help: bool,
    pub quit: bool,
    pub workers: Workers,
    pub workers_started: bool,
}

impl App {
    pub fn new(home: PathBuf) -> Self {
        let (screen, settings_data, settings, mut banner) = load_dashboard(&home);
        let settings_form = if screen == Screen::Dashboard {
            match SettingsForm::open(&home.join(SETTINGS_FILE)) {
                Ok(form) => Some(form),
                Err(error) => {
                    if banner.is_none() {
                        banner = Some(error);
                    }
                    None
                }
            }
        } else {
            None
        };
        let skills = SkillsView {
            library: SkillLibrary::new(home.clone()),
            skills: Vec::new(),
            selected: 0,
            body: None,
            scroll: 0,
            max_scroll: Cell::new(0),
            stats: None,
            error: None,
            new_name: None,
        };
        Self {
            home,
            screen,
            tab: Tab::Overview,
            onboarding: Onboarding::default(),
            daemon: DaemonState::Stopped,
            status: None,
            ownership: None,
            events: VecDeque::new(),
            local_offset: UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC),
            activity_selected: 0,
            activity_paused: false,
            doctor: DoctorView::default(),
            skills,
            prompt: PromptView::default(),
            settings,
            settings_data,
            settings_form,
            memory: MemoryView::default(),
            editor: None,
            banner,
            help: false,
            quit: false,
            workers: Workers::new(),
            workers_started: false,
        }
    }

    pub fn start(&mut self) {
        self.skills.reload();
        if self.screen == Screen::Dashboard {
            self.start_dashboard_workers();
        }
    }

    fn start_dashboard_workers(&mut self) {
        if self.workers_started {
            return;
        }
        self.workers_started = true;
        self.workers.spawn_status(self.home.clone());
        self.workers.spawn_watch(self.home.clone());
        let runnable = self.banner.is_none()
            && self
                .settings_data
                .as_ref()
                .is_some_and(|settings| settings.validate_for_run().is_ok());
        if !runnable {
            if self.ownership.is_none() {
                self.daemon = DaemonState::Error(
                    "Settings need attention. Open Doctor for details.".to_owned(),
                );
            }
            self.sync_daemon_controls();
            return;
        }
        if self.ownership.is_some() {
            self.daemon = DaemonState::Running;
            self.sync_daemon_controls();
            return;
        }
        self.daemon = DaemonState::Starting;
        self.sync_daemon_controls();
        let start = self.settings.daemon.start.clone();
        self.workers.spawn_daemon_action(
            self.home.clone(),
            DaemonAction::Start,
            Ownership::Attached,
            start,
        );
    }

    pub fn handle_paste(&mut self, text: String) {
        if let Some(editor) = &mut self.editor {
            editor.text.insert_text(&text);
            return;
        }
        if self.tab == Tab::Settings
            && let Some(form) = &mut self.settings_form
        {
            form.handle_paste(&text);
            return;
        }
        if self.screen == Screen::Onboarding && !self.help {
            self.onboarding.handle_paste(&text);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.editor.is_some() {
            self.handle_editor_key(key);
            return;
        }
        if self.settings_form.is_some() && self.tab == Tab::Settings {
            let editing = self
                .settings_form
                .as_ref()
                .is_some_and(|form| form.edit.is_some());
            let leaves_form = !editing
                && matches!(
                    key.code,
                    KeyCode::Tab
                        | KeyCode::BackTab
                        | KeyCode::Left
                        | KeyCode::Right
                        | KeyCode::Char('1'..='7')
                        | KeyCode::Char('q')
                        | KeyCode::Char('?')
                );
            if !leaves_form {
                self.handle_settings_form_key(key);
                return;
            }
        }
        if self.help {
            if key.code == KeyCode::Esc {
                self.help = false;
            }
            return;
        }
        if self.screen == Screen::Onboarding {
            match self.onboarding.handle_key(key) {
                Action::None => {}
                Action::Quit => self.quit = true,
                Action::FetchModels => {
                    let base_url = self.onboarding.base_url.value.trim().to_owned();
                    let api_key = self.onboarding.api_key.value.trim().to_owned();
                    self.workers.spawn_models(base_url, api_key);
                }
                Action::FetchModelInfo => {
                    let base_url = self.onboarding.base_url.value.trim().to_owned();
                    let model = self.onboarding.selected_model.clone().unwrap_or_default();
                    self.workers.spawn_model_info(base_url, model);
                }
                Action::Finish(options) => {
                    self.workers.spawn_onboard(self.home.clone(), *options);
                }
            }
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Tab | KeyCode::Right => self.switch_tab(self.tab.next()),
            KeyCode::BackTab | KeyCode::Left => self.switch_tab(self.tab.previous()),
            KeyCode::Char(character @ '1'..='7') => {
                let index = character as usize - '1' as usize;
                self.switch_tab(Tab::ALL[index]);
            }
            _ => self.handle_tab_key(key),
        }
    }

    fn switch_tab(&mut self, tab: Tab) {
        self.tab = tab;
        match tab {
            Tab::Prompt => self.refresh_prompt(),
            Tab::Skills => {
                if self.skills.skills.is_empty() {
                    self.skills.reload();
                } else if self.skills.body.is_none() {
                    self.skills.load_selected();
                }
            }
            Tab::Memory => self.memory.reload(&self.home),
            Tab::Settings => self.ensure_settings_form(),
            _ => {}
        }
    }

    fn handle_tab_key(&mut self, key: KeyEvent) {
        match self.tab {
            Tab::Overview => {}
            Tab::Activity => self.handle_activity(key),
            Tab::Doctor => self.handle_doctor(key),
            Tab::Skills => self.handle_skills(key),
            Tab::Prompt => self.handle_prompt(key),
            Tab::Settings => self.handle_settings(key),
            Tab::Memory => self.handle_memory(key),
        }
    }

    fn handle_editor_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            self.save_editor();
            return;
        }
        if key.code == KeyCode::Esc {
            self.close_editor();
            return;
        }
        if let Some(editor) = &mut self.editor {
            editor.text.handle_key(key);
        }
    }

    fn save_editor(&mut self) {
        let mut saved = None;
        if let Some(editor) = self.editor.as_mut() {
            let mut text = editor.text.text();
            if matches!(editor.kind, EditorKind::Settings) {
                if let Err(error) = validate_settings_text(&text) {
                    editor.text.error = Some(error);
                    return;
                }
                text = match compact_settings_text(&text) {
                    Ok(text) => text,
                    Err(error) => {
                        editor.text.error = Some(error);
                        return;
                    }
                };
                editor.text.replace_text(text.clone());
                if let Err(error) = validate_settings_text(&text) {
                    editor.text.error = Some(error);
                    return;
                }
            }
            if let Err(error) = editor.text.save() {
                editor.text.error = Some(error.to_string());
                return;
            }
            editor.text.dirty = false;
            editor.text.error = None;
            editor.text.notice = Some("Saved.".to_owned());
            saved = Some(editor.kind.clone());
        }
        if let Some(kind) = saved {
            self.after_editor_save(kind);
        }
    }

    fn close_editor(&mut self) {
        let discard = self.editor.as_mut().is_some_and(|editor| {
            if editor.text.dirty && !editor.text.confirm_discard {
                editor.text.confirm_discard = true;
                editor.text.notice =
                    Some("Unsaved changes. Press Esc again to discard them.".to_owned());
                false
            } else {
                true
            }
        });
        if discard {
            self.editor = None;
        }
    }

    fn open_editor(&mut self, kind: EditorKind, path: PathBuf, title: &str) {
        match TextEditor::open(&path, title) {
            Ok(text) => self.editor = Some(OpenEditor { kind, text }),
            Err(error) => {
                self.banner = Some(format!("Could not open {}: {error}", path.display()));
            }
        }
    }

    fn after_editor_save(&mut self, kind: EditorKind) {
        match kind {
            EditorKind::Settings => {
                self.reload_settings();
                self.reload_settings_form();
                self.settings.message =
                    Some("settings.json saved. Restart Hearth to apply all changes.".to_owned());
            }
            EditorKind::Soul | EditorKind::Guidance => {
                self.settings.message =
                    Some("File saved. Restart Hearth to apply prompt changes.".to_owned());
            }
            EditorKind::Skill { name } => {
                self.skills.reload();
                if let Some(index) = self
                    .skills
                    .skills
                    .iter()
                    .position(|skill| skill.name == name)
                {
                    self.skills.selected = index;
                    self.skills.body = self.skills.library.load_body(&name).ok().flatten();
                }
                self.settings.message = Some("Skill saved.".to_owned());
            }
        }
    }

    fn ensure_settings_form(&mut self) {
        if self.settings_form.is_some() {
            return;
        }
        self.reload_settings_form();
    }

    fn reload_settings_form(&mut self) {
        match SettingsForm::open(&self.home.join(SETTINGS_FILE)) {
            Ok(form) => {
                self.settings_form = Some(form);
                self.banner = None;
            }
            Err(error) => {
                self.settings_form = None;
                self.banner = Some(error);
            }
        }
        self.sync_daemon_controls();
    }

    /// Keep the daemon action rows in step with the live daemon state.
    fn sync_daemon_controls(&mut self) {
        let running = !matches!(self.daemon, DaemonState::Stopped | DaemonState::Error(_));
        if let Some(form) = &mut self.settings_form {
            form.set_daemon_running(running);
        }
    }

    fn open_settings_editor(&mut self) {
        self.open_editor(
            EditorKind::Settings,
            self.home.join(SETTINGS_FILE),
            "settings.json",
        );
    }

    fn handle_settings_form_key(&mut self, key: KeyEvent) {
        let action = self
            .settings_form
            .as_mut()
            .map(|form| form.handle_key(key))
            .unwrap_or(FormAction::None);
        match action {
            FormAction::None | FormAction::Close => {}
            FormAction::RawJson => self.open_settings_editor(),
            FormAction::Changed => self.save_settings_form(),
            FormAction::EditSoul => self.edit_identity_file(FileKind::Soul),
            FormAction::EditGuidance => self.edit_identity_file(FileKind::Guidance),
            FormAction::StartDaemon => self.start_daemon(),
            FormAction::StopDaemon => self.stop_daemon(),
            FormAction::RestartDaemon => self.restart_daemon(),
            FormAction::LookupModel => self.lookup_model_info(),
        }
    }

    fn lookup_model_info(&mut self) {
        let Some(form) = self.settings_form.as_mut() else {
            return;
        };
        let base_url = form.document["models"]["base_url"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let model = form.document["models"]["model"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if base_url.is_empty() || model.is_empty() {
            form.error = Some("Set a base URL and model before looking it up.".to_owned());
            return;
        }
        form.notice = Some(format!("Looking up {model} at models.dev..."));
        form.error = None;
        self.workers.spawn_model_info(base_url, model);
    }

    fn edit_identity_file(&mut self, kind: FileKind) {
        self.open_editor(kind.editor_kind(), kind.path(&self.home), kind.title());
    }

    fn save_settings_form(&mut self) {
        let previous_daemon = self
            .settings_data
            .as_ref()
            .map(|settings| settings.daemon.clone());
        let Some(form) = self.settings_form.as_ref() else {
            return;
        };
        let text = form.text();
        if let Err(error) = editor::write_private(&self.home.join(SETTINGS_FILE), &text) {
            if let Some(form) = self.settings_form.as_mut() {
                form.error = Some(error.to_string());
            }
            return;
        }
        self.reload_settings();
        if let Some(form) = self.settings_form.as_mut() {
            form.error = None;
            form.notice =
                Some("Saved. Restart Hearth to apply daemon or model changes.".to_owned());
        }
        let start_changed = previous_daemon
            .as_ref()
            .zip(self.settings_data.as_ref())
            .is_some_and(|(previous, current)| previous.start != current.daemon.start);
        if start_changed {
            let start = self.settings.daemon.start.clone();
            let ownership = self.ownership.clone().unwrap_or(Ownership::Attached);
            self.settings.message = Some(format!("Applying {}...", start_label(&start)));
            self.daemon = DaemonState::Starting;
            self.sync_daemon_controls();
            self.workers
                .spawn_service(self.home.clone(), start, ownership);
        } else {
            self.settings.message = Some("All settings saved.".to_owned());
        }
    }

    fn reload_settings(&mut self) {
        match hearth_config::load(&self.home) {
            Ok(settings) => {
                self.banner = settings
                    .validate_for_run()
                    .err()
                    .map(|error| error.to_string());
                self.settings.daemon = settings.daemon.clone();
                self.settings_data = Some(settings);
            }
            Err(error) => {
                self.settings_data = None;
                self.banner = Some(error.to_string());
            }
        }
    }

    fn handle_memory(&mut self, key: KeyEvent) {
        if self.memory.filter_mode {
            match key.code {
                KeyCode::Enter => {
                    self.memory.filter_mode = false;
                    self.memory.selected = 0;
                }
                KeyCode::Esc => {
                    self.memory.filter.clear();
                    self.memory.filter_mode = false;
                    self.memory.selected = 0;
                }
                KeyCode::Backspace => {
                    self.memory.filter.pop();
                    self.memory.selected = 0;
                }
                KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.memory.filter.push(character);
                    self.memory.selected = 0;
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Up => {
                self.memory.selected = self.memory.selected.saturating_sub(1);
            }
            KeyCode::Down => {
                if self.memory.selected + 1 < self.memory.filtered().len() {
                    self.memory.selected += 1;
                }
            }
            KeyCode::Char('/') => {
                self.memory.filter_mode = true;
                self.memory.filter.clear();
                self.memory.selected = 0;
            }
            KeyCode::Char('r') => self.memory.reload(&self.home),
            KeyCode::Char('g') => self.memory.selected = 0,
            KeyCode::Esc if !self.memory.filter.is_empty() => {
                self.memory.filter.clear();
                self.memory.selected = 0;
            }
            _ => {}
        }
    }

    fn handle_activity(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => {
                self.activity_selected = self.activity_selected.saturating_sub(1);
                self.activity_paused = true;
            }
            KeyCode::Down => {
                if self.activity_selected + 1 < crate::ui::activity::row_count(&self.events) {
                    self.activity_selected += 1;
                }
                self.activity_paused = true;
            }
            KeyCode::Char('p') => self.activity_paused = !self.activity_paused,
            KeyCode::Char('g') => {
                self.activity_selected = 0;
                self.activity_paused = true;
            }
            _ => {}
        }
    }

    fn handle_doctor(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter if !self.doctor.running => {
                self.doctor.running = true;
                self.doctor.live = false;
                self.doctor.output = None;
                self.workers.spawn_doctor(self.home.clone(), false);
            }
            KeyCode::Char('l') if !self.doctor.running => {
                self.doctor.running = true;
                self.doctor.live = true;
                self.doctor.output = None;
                self.workers.spawn_doctor(self.home.clone(), true);
            }
            KeyCode::Up => self.doctor.scroll = self.doctor.scroll.saturating_sub(1),
            KeyCode::Down => self.doctor.scroll += 1,
            _ => {}
        }
    }

    fn handle_skills(&mut self, key: KeyEvent) {
        if self.skills.new_name.is_some() {
            match key.code {
                KeyCode::Enter => self.create_user_skill(),
                KeyCode::Esc => {
                    self.skills.new_name = None;
                    self.skills.error = None;
                }
                _ => {
                    if let Some(input) = &mut self.skills.new_name
                        && input.handle(key)
                    {
                        self.skills.error = None;
                    }
                }
            }
            return;
        }
        match key.code {
            KeyCode::Up => {
                self.skills.selected = self.skills.selected.saturating_sub(1);
                self.skills.load_selected();
            }
            KeyCode::Down => {
                if self.skills.selected + 1 < self.skills.skills.len() {
                    self.skills.selected += 1;
                }
                self.skills.load_selected();
            }
            KeyCode::Enter => self.skills.load_selected(),
            KeyCode::PageUp => self.skills.scroll = self.skills.scroll.saturating_sub(15),
            KeyCode::PageDown => {
                self.skills.scroll = (self.skills.scroll + 15).min(self.skills.max_scroll.get());
            }
            KeyCode::Home => self.skills.scroll = 0,
            KeyCode::End => self.skills.scroll = self.skills.max_scroll.get(),
            KeyCode::Char('n') => {
                self.skills.new_name = Some(TextInput::new(false));
                self.skills.error = None;
            }
            KeyCode::Char('e') => {
                let skill = self.skills.skills.get(self.skills.selected).cloned();
                match skill {
                    Some(skill) if skill.scope == SkillScope::User => self.open_editor(
                        EditorKind::Skill {
                            name: skill.name.clone(),
                        },
                        skill.path.clone(),
                        &format!("skills/{}.md", skill.name),
                    ),
                    Some(_) => {
                        self.settings.message = Some(
                            "Learned skills are managed by Hearth. Edit a user skill instead."
                                .to_owned(),
                        );
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    fn create_user_skill(&mut self) {
        let Some(input) = self.skills.new_name.take() else {
            return;
        };
        let name = slug(&input.value);
        if name.is_empty() {
            self.skills.error = Some("Give the skill a name with letters or numbers.".to_owned());
            self.skills.new_name = Some(input);
            return;
        }
        let path = self.home.join("skills").join(format!("{name}.md"));
        if path.exists() {
            self.skills.error = Some(format!("A skill named {name} already exists."));
            self.skills.new_name = Some(input);
            return;
        }
        let title = input.value.trim();
        let title = if title.is_empty() {
            name.replace('-', " ")
        } else {
            title.to_owned()
        };
        let text = format!(
            "# {title}\n\nDescribe when Hearth should use this skill.\n\n## Steps\n\n1. Write the first step here.\n"
        );
        let editor = TextEditor::from_text(path, format!("skills/{name}.md"), text);
        self.editor = Some(OpenEditor {
            kind: EditorKind::Skill { name },
            text: editor,
        });
    }

    fn handle_prompt(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('r') => {
                self.prompt.raw = !self.prompt.raw;
                self.prompt.scroll = 0;
            }
            // Toggle full length for the selected section.
            KeyCode::Enter | KeyCode::Char(' ') => {
                if !self.prompt.expanded.remove(&self.prompt.selected) {
                    self.prompt.expanded.insert(self.prompt.selected);
                }
                self.prompt.scroll = 0;
            }
            KeyCode::Char('a') => {
                if self.prompt.expanded.is_empty() {
                    let count = self
                        .prompt
                        .cache
                        .borrow()
                        .as_ref()
                        .map(|cache| cache.sections.len())
                        .unwrap_or(0);
                    self.prompt.expanded.extend(0..count);
                } else {
                    self.prompt.expanded.clear();
                }
                self.prompt.scroll = 0;
            }
            KeyCode::Up => ui::prompt::select_previous(&mut self.prompt),
            KeyCode::Down => ui::prompt::select_next(&mut self.prompt),
            KeyCode::PageUp => self.prompt.scroll = self.prompt.scroll.saturating_sub(15),
            KeyCode::PageDown => {
                self.prompt.scroll = (self.prompt.scroll + 15).min(self.prompt.max_scroll.get());
            }
            KeyCode::Home => ui::prompt::select_first(&mut self.prompt),
            KeyCode::End => ui::prompt::select_last(&mut self.prompt),
            KeyCode::Char('o') => self.refresh_prompt(),
            _ => {}
        }
    }

    fn handle_settings(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.settings.focus = self.settings.focus.saturating_sub(1),
            KeyCode::Down => self.settings.focus = (self.settings.focus + 1).min(5),
            KeyCode::Left | KeyCode::Right => {
                if self.settings.focus == 0 {
                    cycle_start(&mut self.settings.daemon);
                }
            }
            KeyCode::Enter => self.activate_setting(),
            KeyCode::Char('e') => self.ensure_settings_form(),
            _ => {}
        }
    }

    fn activate_setting(&mut self) {
        match self.settings.focus {
            0 => {
                cycle_start(&mut self.settings.daemon);
                self.save_and_apply_mode();
            }
            1 => {
                self.settings.daemon.keep_running = !self.settings.daemon.keep_running;
                self.save_daemon();
                self.settings.message = Some(if self.settings.daemon.keep_running {
                    "Hearth will keep running after this window closes.".to_owned()
                } else {
                    "Hearth will stop when this window closes.".to_owned()
                });
            }
            2 => {
                self.ensure_settings_form();
            }
            3 => self.start_daemon(),
            4 => self.stop_daemon(),
            _ => self.restart_daemon(),
        }
    }

    fn start_daemon(&mut self) {
        if !matches!(self.daemon, DaemonState::Stopped | DaemonState::Error(_)) {
            self.settings.message = Some("Hearth is already running.".to_owned());
            return;
        }
        let start = self.settings.daemon.start.clone();
        self.settings.message = Some("Starting the daemon...".to_owned());
        self.daemon = DaemonState::Starting;
        self.sync_daemon_controls();
        self.workers.spawn_daemon_action(
            self.home.clone(),
            DaemonAction::Start,
            self.ownership.clone().unwrap_or(Ownership::Attached),
            start,
        );
    }

    fn stop_daemon(&mut self) {
        if matches!(self.daemon, DaemonState::Stopped) {
            self.settings.message = Some("Hearth is already stopped.".to_owned());
            return;
        }
        if matches!(self.daemon, DaemonState::Stopping) {
            self.settings.message = Some("Hearth is already stopping.".to_owned());
            return;
        }
        self.settings.message = Some("Stopping the daemon...".to_owned());
        self.daemon = DaemonState::Stopping;
        self.sync_daemon_controls();
        let ownership = self.ownership.clone().unwrap_or(Ownership::Attached);
        self.workers.spawn_daemon_action(
            self.home.clone(),
            DaemonAction::Stop,
            ownership,
            self.settings.daemon.start.clone(),
        );
    }

    fn restart_daemon(&mut self) {
        if matches!(self.daemon, DaemonState::Stopping) {
            self.settings.message = Some("Hearth is still stopping.".to_owned());
            return;
        }
        if matches!(self.daemon, DaemonState::Stopped) {
            self.start_daemon();
            return;
        }
        if matches!(self.daemon, DaemonState::Starting) {
            self.settings.message = Some("Hearth is still starting.".to_owned());
            return;
        }
        self.settings.message = Some("Restarting the daemon...".to_owned());
        self.daemon = DaemonState::Starting;
        self.sync_daemon_controls();
        let ownership = self.ownership.clone().unwrap_or(Ownership::Attached);
        self.workers.spawn_daemon_action(
            self.home.clone(),
            DaemonAction::Restart,
            ownership,
            self.settings.daemon.start.clone(),
        );
    }

    fn save_daemon(&mut self) {
        if let Err(error) = hearth_config::save_daemon(&self.home, &self.settings.daemon) {
            self.settings.message = Some(error.to_string());
        }
    }

    fn save_and_apply_mode(&mut self) {
        self.save_daemon();
        let start = self.settings.daemon.start.clone();
        let ownership = self.ownership.clone().unwrap_or(Ownership::Attached);
        self.settings.message = Some(format!("Applying {}...", start_label(&start)));
        self.daemon = DaemonState::Starting;
        self.sync_daemon_controls();
        self.workers
            .spawn_service(self.home.clone(), start, ownership);
    }

    fn refresh_prompt(&mut self) {
        if self.prompt.loading {
            return;
        }
        self.prompt.loading = true;
        self.workers.spawn_prompt(self.home.clone());
    }

    pub fn drain_events(&mut self) {
        while let Some(event) = self.workers.try_recv() {
            self.apply_event(event);
        }
    }

    fn apply_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Status(result) => match result {
                Ok(status) => {
                    self.status = Some(status);
                    if !matches!(self.daemon, DaemonState::Stopping) {
                        self.daemon = DaemonState::Running;
                    }
                }
                Err(_) => {
                    self.status = None;
                    if !matches!(
                        self.daemon,
                        DaemonState::Starting | DaemonState::Stopping | DaemonState::Error(_)
                    ) {
                        self.daemon = DaemonState::Stopped;
                    }
                }
            },
            AppEvent::Live(value) => {
                let settled = value["kind"].as_str() == Some("run_settled");
                self.events.push_back(value);
                while self.events.len() > MAX_EVENTS {
                    self.events.pop_front();
                }
                if !self.activity_paused {
                    self.activity_selected =
                        crate::ui::activity::row_count(&self.events).saturating_sub(1);
                }
                if settled && self.tab == Tab::Prompt {
                    self.refresh_prompt();
                }
            }
            AppEvent::Models(result) => self.onboarding.models_loaded(result),
            AppEvent::Doctor(result) => {
                self.doctor.running = false;
                match result {
                    Ok(output) => self.doctor.output = Some(output),
                    Err(error) => self.doctor.output = Some(error),
                }
            }
            AppEvent::Onboarded(result) => match result {
                OnboardResult::Ready(ownership, status) => {
                    self.ownership = Some(ownership);
                    self.status = Some(status);
                    self.daemon = DaemonState::Running;
                    self.finish_onboarding();
                }
                OnboardResult::CreatedButDaemonFailed(error) => {
                    self.daemon = DaemonState::Error(error.clone());
                    self.banner = Some(error);
                    self.finish_onboarding();
                }
                OnboardResult::InitFailed(error) => {
                    self.onboarding.step = crate::onboarding::Step::Failed;
                    self.onboarding.set_error(error);
                }
            },
            AppEvent::Service(result) => match result {
                Ok((ownership, status)) => {
                    self.ownership = Some(ownership);
                    self.status = Some(status);
                    self.daemon = DaemonState::Running;
                    self.settings.message = Some("Daemon settings applied.".to_owned());
                }
                Err(error) => {
                    self.daemon = DaemonState::Error(error.clone());
                    self.settings.message = Some(error);
                }
            },
            AppEvent::Daemon(result) => match result {
                Ok((ownership, status)) => {
                    self.ownership = Some(ownership);
                    self.status = Some(status);
                    self.daemon = DaemonState::Running;
                    self.settings.message = Some("Daemon is running.".to_owned());
                }
                Err(error) => {
                    self.daemon = DaemonState::Error(error.clone());
                    self.settings.message = Some(error.clone());
                    self.banner = Some(error);
                }
            },
            AppEvent::Stopped(result) => match result {
                Ok(()) => {
                    self.status = None;
                    self.daemon = DaemonState::Stopped;
                    self.ownership = None;
                    self.settings.message = Some("Daemon stopped.".to_owned());
                }
                Err(error) => {
                    self.daemon = DaemonState::Error(error.clone());
                    self.settings.message = Some(error);
                }
            },
            AppEvent::Prompt(result) => {
                self.prompt.loading = false;
                match result {
                    Ok(exchanges) => {
                        if exchanges.is_empty() && !self.prompt.exchanges.is_empty() {
                            self.banner = Some(
                                "No saved prompts were found; showing the last loaded set."
                                    .to_owned(),
                            );
                        } else {
                            self.prompt.exchanges = exchanges;
                            self.prompt.selected = 0;
                            self.prompt.scroll = 0;
                        }
                    }
                    Err(error) => {
                        self.banner = Some(error);
                    }
                }
            }
            AppEvent::ModelInfo { model, result } => {
                if self.screen == Screen::Onboarding && self.onboarding.model_limits_loading {
                    if self.onboarding.selected_model.as_deref() == Some(model.as_str()) {
                        self.onboarding.model_info_loaded(result);
                    }
                    return;
                }
                let changed = match (&mut self.settings_form, result) {
                    (Some(form), Ok(Some(limits)))
                        if form.document["models"]["model"].as_str() == Some(model.as_str()) =>
                    {
                        form.apply_model_info(limits)
                    }
                    (Some(form), Ok(Some(_))) => {
                        form.notice = Some(
                            "The model changed before the lookup finished; nothing changed."
                                .to_owned(),
                        );
                        false
                    }
                    (Some(form), Ok(None)) => {
                        form.notice = Some(
                            "models.dev has no entry for this base URL and model; nothing changed."
                                .to_owned(),
                        );
                        false
                    }
                    (Some(form), Err(error)) => {
                        form.error = Some(error);
                        false
                    }
                    (None, _) => false,
                };
                if changed {
                    self.save_settings_form();
                }
            }
        }
        self.sync_daemon_controls();
    }

    fn finish_onboarding(&mut self) {
        self.screen = Screen::Dashboard;
        self.tab = Tab::Overview;
        self.settings_data = hearth_config::load(&self.home).ok();
        if let Some(settings) = &self.settings_data {
            self.settings.daemon = settings.daemon.clone();
        }
        self.start_dashboard_workers();
        self.skills.reload();
        self.ensure_settings_form();
    }

    pub fn shutdown(&mut self) {
        self.workers.stop();
        let keep_running = self.settings.daemon.keep_running;
        if let Some(ownership) = &self.ownership {
            daemon_client::stop_on_exit(&self.home, ownership, keep_running);
        }
    }
}

impl SkillsView {
    pub(crate) fn reload(&mut self) {
        self.error = None;
        match self.library.load_all() {
            Ok(skills) => self.skills = skills,
            Err(error) => {
                self.skills.clear();
                self.error = Some(error.to_string());
            }
        }
        self.stats = self.library.stats().ok();
        self.selected = self.selected.min(self.skills.len().saturating_sub(1));
        self.load_selected();
    }

    pub(crate) fn load_selected(&mut self) {
        self.body = self
            .skills
            .get(self.selected)
            .and_then(|skill| self.library.load_body(&skill.name).ok().flatten());
        self.scroll = 0;
    }
}

fn slug(text: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in text.trim().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            separator = false;
        } else if matches!(character, ' ' | '-' | '_') && !slug.is_empty() && !separator {
            slug.push('-');
            separator = true;
        }
    }
    slug.trim_end_matches('-').to_owned()
}

fn compact_settings_text(text: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("settings.json is not valid: {error}"))?;
    let value = hearth_config::compact_settings(&value);
    let mut text = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("settings.json could not be encoded: {error}"))?;
    text.push('\n');
    Ok(text)
}

fn validate_settings_text(text: &str) -> Result<(), String> {
    let settings: Settings = serde_json::from_str(text)
        .map_err(|error| format!("settings.json is not valid: {error}"))?;
    settings.validate().map_err(|error| error.to_string())
}

fn load_dashboard(
    home: &std::path::Path,
) -> (Screen, Option<Settings>, SettingsView, Option<String>) {
    let path = home.join(SETTINGS_FILE);
    if !path.is_file() {
        return (
            Screen::Onboarding,
            None,
            SettingsView {
                daemon: Daemon::default(),
                focus: 0,
                message: None,
            },
            None,
        );
    }
    match hearth_config::load(home) {
        Ok(mut settings) => {
            let banner = settings
                .validate_for_run()
                .err()
                .map(|error| error.to_string());
            let daemon = migrate_daemon(home, &settings);
            // Keep the in-memory view in step with a migrated daemon section so
            // a later settings save does not think the start mode changed.
            settings.daemon = daemon.clone();
            (
                Screen::Dashboard,
                Some(settings),
                SettingsView {
                    daemon,
                    focus: 0,
                    message: None,
                },
                banner,
            )
        }
        Err(error) => (
            Screen::Dashboard,
            None,
            SettingsView {
                daemon: Daemon::default(),
                focus: 0,
                message: None,
            },
            Some(error.to_string()),
        ),
    }
}

fn migrate_daemon(home: &std::path::Path, settings: &Settings) -> Daemon {
    let has_section = std::fs::read_to_string(home.join(SETTINGS_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value.get("daemon").cloned())
        .is_some();
    if has_section {
        return settings.daemon.clone();
    }
    let installed = daemon_client::unit_installed(home);
    if !installed {
        return settings.daemon.clone();
    }
    let daemon = Daemon {
        start: "login".to_owned(),
        keep_running: true,
    };
    let _ = hearth_config::save_daemon(home, &daemon);
    // The unit is already installed. Never rewrite it from whatever binary
    // happens to open the TUI; a test or ad-hoc build must not capture the unit.
    daemon
}

fn cycle_start(daemon: &mut Daemon) {
    daemon.start = match daemon.start.as_str() {
        "tui" => "login".to_owned(),
        "login" => "startup".to_owned(),
        _ => "tui".to_owned(),
    };
}

pub fn start_label(start: &str) -> &'static str {
    match start {
        "login" => "start at login",
        "startup" => "start at startup",
        _ => "start when Hearth opens",
    }
}

pub fn run(home: PathBuf) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    execute!(std::io::stdout(), EnableBracketedPaste)?;
    let mut app = App::new(home);
    app.start();
    let result = run_loop(&mut terminal, &mut app);
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    app.shutdown();
    result
}

fn run_loop(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    while !app.quit {
        terminal.draw(|frame| ui::render(app, frame))?;
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.handle_key(key),
                Event::Paste(text) => app.handle_paste(text),
                _ => {}
            }
        }
        app.drain_events();
    }
    Ok(())
}
