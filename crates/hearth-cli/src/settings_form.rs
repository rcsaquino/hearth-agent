//! Friendly, all-settings form for settings.json.

use std::fs;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};

use crate::catalog_cache;
use crate::onboarding::TextInput;

const SEARCH_PROVIDERS: [(&str, &str); 5] = [
    ("exa", "Exa"),
    ("brave", "Brave"),
    ("deepseek", "DeepSeek"),
    ("firecrawl", "Firecrawl"),
    ("tinyfish", "TinyFish"),
];
const FETCH_PROVIDERS: [(&str, &str); 5] = [
    ("jina", "Jina Reader"),
    ("http", "Direct HTTP"),
    ("tinyfish", "TinyFish"),
    ("firecrawl", "Firecrawl"),
    ("deepseek", "DeepSeek"),
];
const FETCH_KEYED_PROVIDERS: [(&str, &str); 4] = [
    ("jina", "Jina Reader"),
    ("tinyfish", "TinyFish"),
    ("firecrawl", "Firecrawl"),
    ("deepseek", "DeepSeek"),
];

const SECTION_ORDER: [(&str, &str); 11] = [
    ("identity", "Agent"),
    ("daemon", "Daemon"),
    ("models", "Model provider"),
    ("telegram", "Telegram"),
    ("media", "Optional media services"),
    ("web", "Web search and fetch"),
    ("agent", "Agent loop"),
    ("tasks", "Tasks and helpers"),
    ("compaction", "Compaction"),
    ("memory", "Memory"),
    ("logging", "Logging"),
];

/// What a form row represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Header,
    Bool,
    Secret,
    Text,
    Integer,
    Float,
    Array,
    RawJson,
    AddImage,
    AddStt,
    AddTts,
    DaemonStart,
    Thinking,
    LogLevel,
    SearchOrder,
    FetchOrder,
    Timezone,
    LookupModel,
    EditSoul,
    EditGuidance,
    StartDaemon,
    StopDaemon,
    RestartDaemon,
}

/// What the form wants the app to do after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormAction {
    None,
    Close,
    RawJson,
    Changed,
    LookupModel,
    EditSoul,
    EditGuidance,
    StartDaemon,
    StopDaemon,
    RestartDaemon,
}

/// One row in the settings form.
#[derive(Debug, Clone)]
pub struct SettingsRow {
    pub label: String,
    pub path: Vec<String>,
    pub value: String,
    pub kind: RowKind,
    pub help: &'static str,
    pub tooltip: &'static str,
    pub modified: bool,
}

/// The active inline edit.
#[derive(Debug, Clone)]
pub struct SettingsEdit {
    pub row: usize,
    pub input: TextInput,
    pub hint: &'static str,
}

/// The active list picker.
#[derive(Debug, Clone)]
pub struct SettingsPicker {
    pub row: usize,
    pub title: String,
    pub options: Vec<String>,
    pub selected: usize,
}

/// The complete settings form.
#[derive(Debug, Clone)]
pub struct SettingsForm {
    pub document: Value,
    pub rows: Vec<SettingsRow>,
    pub selected: usize,
    pub edit: Option<SettingsEdit>,
    pub picker: Option<SettingsPicker>,
    pub draft: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    home: PathBuf,
    defaults: Value,
    catalog_key: Option<(String, String)>,
}

impl SettingsForm {
    /// Read and parse settings.json.
    pub fn open(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
        let document: Value = serde_json::from_str(&text)
            .map_err(|error| format!("settings.json is not valid JSON: {error}"))?;
        let home = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        Self::from_document_at(home, document)
    }

    /// Build a form from an already parsed document.
    pub fn from_document(document: Value) -> Result<Self, String> {
        Self::from_document_at(PathBuf::new(), document)
    }

    /// Build a form whose default workspace lives under `home`.
    pub fn from_document_at(home: PathBuf, document: Value) -> Result<Self, String> {
        if !document.is_object() {
            return Err("settings.json must contain a JSON object.".to_owned());
        }
        let mut document = with_defaults(document);
        normalize_fetch_aliases(&mut document);
        if let Some(tasks) = document.get_mut("tasks").and_then(Value::as_object_mut) {
            tasks.remove("eta_reminder_minutes");
        }
        if let Some(identity) = document.get_mut("identity").and_then(Value::as_object_mut) {
            identity.remove("soul_file");
        }
        if let Some(models) = document.get_mut("models").and_then(Value::as_object_mut) {
            models.remove("pricing");
        }
        let mut defaults = hearth_config::default_settings_value();
        let mut catalog_key = None;
        if let (Some(base_url), Some(model)) = (
            document["models"]["base_url"].as_str(),
            document["models"]["model"].as_str(),
        ) && let Some(limits) = catalog_cache::load(&home, base_url, model)
        {
            apply_catalog_defaults(&mut defaults, limits);
            catalog_key = Some((base_url.to_owned(), model.to_owned()));
        }
        let rows = build_rows(&document, &home, &defaults);
        let selected = rows
            .iter()
            .position(|row| row.kind != RowKind::Header)
            .unwrap_or(0);
        Ok(Self {
            document,
            rows,
            selected,
            edit: None,
            picker: None,
            draft: false,
            error: None,
            notice: Some("Changes save as you go. * marks a change; r resets a row.".to_owned()),
            home,
            defaults,
            catalog_key,
        })
    }

    /// The document as pretty JSON, with defaults omitted.
    pub fn text(&self) -> String {
        let document = hearth_config::compact_settings(&self.document);
        serde_json::to_string_pretty(&document)
            .map(|mut text| {
                text.push('\n');
                text
            })
            .unwrap_or_else(|_| document.to_string())
    }

    /// The selected row, when there is one.
    pub fn selected_row(&self) -> Option<&SettingsRow> {
        self.rows.get(self.selected)
    }

    /// Consume a key and return the next action.
    pub fn handle_key(&mut self, key: KeyEvent) -> FormAction {
        if self.edit.is_some() {
            return self.handle_edit_key(key);
        }
        if self.picker.is_some() {
            return self.handle_picker_key(key);
        }
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            match key.code {
                KeyCode::Up => return self.move_order(-1),
                KeyCode::Down => return self.move_order(1),
                _ => {}
            }
        }
        match key.code {
            KeyCode::Up => {
                self.step(-1);
                FormAction::None
            }
            KeyCode::Down => {
                self.step(1);
                FormAction::None
            }
            KeyCode::PageUp => {
                for _ in 0..10 {
                    self.step(-1);
                }
                FormAction::None
            }
            KeyCode::PageDown => {
                for _ in 0..10 {
                    self.step(1);
                }
                FormAction::None
            }
            KeyCode::Enter | KeyCode::Char('e') => self.activate(),
            KeyCode::Char('r') => self.reset_selected(),
            KeyCode::Esc => FormAction::None,
            _ => FormAction::None,
        }
    }

    /// Insert pasted text into the active edit.
    pub fn handle_paste(&mut self, text: &str) -> bool {
        if let Some(edit) = &mut self.edit
            && edit.input.insert_text(text)
        {
            self.error = None;
            return true;
        }
        false
    }

    fn handle_edit_key(&mut self, key: KeyEvent) -> FormAction {
        match key.code {
            KeyCode::Enter => self.commit_edit(),
            KeyCode::Esc => {
                self.edit = None;
                self.error = None;
                FormAction::None
            }
            _ => {
                if let Some(edit) = &mut self.edit
                    && edit.input.handle(key)
                {
                    self.error = None;
                }
                FormAction::None
            }
        }
    }

    fn handle_picker_key(&mut self, key: KeyEvent) -> FormAction {
        match key.code {
            KeyCode::Up => {
                if let Some(picker) = &mut self.picker {
                    picker.selected = picker.selected.saturating_sub(1);
                }
                FormAction::None
            }
            KeyCode::Down => {
                if let Some(picker) = &mut self.picker {
                    picker.selected =
                        (picker.selected + 1).min(picker.options.len().saturating_sub(1));
                }
                FormAction::None
            }
            KeyCode::PageUp => {
                if let Some(picker) = &mut self.picker {
                    picker.selected = picker.selected.saturating_sub(10);
                }
                FormAction::None
            }
            KeyCode::PageDown => {
                if let Some(picker) = &mut self.picker {
                    picker.selected =
                        (picker.selected + 10).min(picker.options.len().saturating_sub(1));
                }
                FormAction::None
            }
            KeyCode::Enter => {
                let Some(picker) = self.picker.take() else {
                    return FormAction::None;
                };
                let Some(value) = picker.options.get(picker.selected).cloned() else {
                    return FormAction::None;
                };
                let Some(row) = self.rows.get(picker.row).cloned() else {
                    return FormAction::None;
                };
                match self.apply_value(&row.path, Value::String(value)) {
                    Ok(true) => {
                        self.notice = Some(format!(
                            "Saved {}. Only changed values are kept.",
                            row.label
                        ));
                        FormAction::Changed
                    }
                    Ok(false) => FormAction::None,
                    Err(error) => {
                        self.error = Some(error);
                        FormAction::None
                    }
                }
            }
            KeyCode::Esc => {
                self.picker = None;
                FormAction::None
            }
            _ => FormAction::None,
        }
    }

    fn open_timezone_picker(&mut self, row: &SettingsRow) -> FormAction {
        let current = value_at(&self.document, &row.path)
            .and_then(Value::as_str)
            .unwrap_or("UTC");
        let options = timezone_options(current);
        let selected = options.iter().position(|zone| zone == current).unwrap_or(0);
        self.picker = Some(SettingsPicker {
            row: self.selected,
            title: row.label.clone(),
            options,
            selected,
        });
        self.error = None;
        self.notice = None;
        FormAction::None
    }

    fn activate(&mut self) -> FormAction {
        let Some(row) = self.rows.get(self.selected).cloned() else {
            return FormAction::None;
        };
        if row.kind == RowKind::Header {
            self.step(1);
            return FormAction::None;
        }
        match row.kind {
            RowKind::RawJson => FormAction::RawJson,
            RowKind::Timezone => self.open_timezone_picker(&row),
            RowKind::LookupModel => FormAction::LookupModel,
            RowKind::EditSoul => FormAction::EditSoul,
            RowKind::EditGuidance => FormAction::EditGuidance,
            RowKind::StartDaemon => FormAction::StartDaemon,
            RowKind::StopDaemon => FormAction::StopDaemon,
            RowKind::RestartDaemon => FormAction::RestartDaemon,
            RowKind::AddImage => self.add_service("image"),
            RowKind::AddStt => self.add_service("stt"),
            RowKind::AddTts => self.add_service("tts"),
            RowKind::SearchOrder | RowKind::FetchOrder => self.toggle_order(&row),
            RowKind::Bool => match self.toggle_bool(&row) {
                Ok(true) => FormAction::Changed,
                Ok(false) => FormAction::None,
                Err(error) => {
                    self.error = Some(error);
                    FormAction::None
                }
            },
            _ if cycle_value(row.kind, &self.document, &row.path).is_some() => {
                let value = cycle_value(row.kind, &self.document, &row.path).expect("cycle value");
                match self.apply_value(&row.path, value) {
                    Ok(true) => FormAction::Changed,
                    Ok(false) => FormAction::None,
                    Err(error) => {
                        self.error = Some(error);
                        FormAction::None
                    }
                }
            }
            _ => {
                self.start_edit(&row);
                FormAction::None
            }
        }
    }

    fn start_edit(&mut self, row: &SettingsRow) {
        let current = value_at(&self.document, &row.path)
            .cloned()
            .unwrap_or(Value::Null);
        let hint = if row.tooltip.is_empty() {
            match row.kind {
                RowKind::Secret => "Type a new value, or press Enter to keep the current one.",
                RowKind::Array => "Separate items with commas. Press Enter to save.",
                _ => "Type a new value, then press Enter. Esc cancels.",
            }
        } else {
            row.tooltip
        };
        let input = match row.kind {
            RowKind::Secret => TextInput::new(true),
            RowKind::Array => TextInput::with_value(&array_text(&current), false),
            _ if row.path == ["agent", "workspace"] && current.is_null() => {
                TextInput::with_value(&self.default_workspace().display().to_string(), false)
            }
            _ => TextInput::with_value(&plain_text(&current), false),
        };
        self.edit = Some(SettingsEdit {
            row: self.selected,
            input,
            hint,
        });
        self.error = None;
        self.notice = None;
    }

    /// The default workspace folder under this home.
    pub fn default_workspace(&self) -> PathBuf {
        self.home.join("workspace")
    }

    /// Apply limits from a catalog lookup.
    pub fn apply_model_info(&mut self, limits: hearth_model::ModelLimits) -> bool {
        let Some(model) = self.document["models"]["model"].as_str().map(str::to_owned) else {
            self.error = Some("Set a model id before looking it up.".to_owned());
            return false;
        };
        let Some(base_url) = self.document["models"]["base_url"]
            .as_str()
            .map(str::to_owned)
        else {
            self.error = Some("Set a base URL before looking it up.".to_owned());
            return false;
        };
        let previous = self.document.clone();
        let previous_defaults = self.defaults.clone();
        let previous_catalog = self.catalog_key.clone();
        if limits.context_window > 0 {
            self.document["models"]["context_window"] = json!(limits.context_window);
        }
        apply_catalog_defaults(&mut self.defaults, limits);
        match self.finish_change(previous) {
            Ok(true) => {
                self.catalog_key = Some((base_url.clone(), model.clone()));
                if let Err(error) = catalog_cache::save(&self.home, &base_url, &model, limits) {
                    self.error = Some(error);
                }
                self.notice = Some(format!(
                    "Filled the context window for {model} from models.dev."
                ));
                true
            }
            Ok(false) => {
                self.defaults = previous_defaults;
                self.catalog_key = previous_catalog;
                false
            }
            Err(error) => {
                self.defaults = previous_defaults;
                self.catalog_key = previous_catalog;
                self.error = Some(error);
                false
            }
        }
    }

    fn toggle_bool(&mut self, row: &SettingsRow) -> Result<bool, String> {
        let current = value_at(&self.document, &row.path)
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.apply_value(&row.path, Value::Bool(!current))
    }

    fn toggle_order(&mut self, row: &SettingsRow) -> FormAction {
        let Some((path, provider)) = order_parts(row) else {
            return FormAction::None;
        };
        let mut names = order_names(&self.document, path);
        if let Some(index) = names.iter().position(|name| name == provider) {
            if names.len() == 1 {
                self.error = Some("Keep at least one provider enabled.".to_owned());
                return FormAction::None;
            }
            names.remove(index);
        } else {
            names.push(provider.to_owned());
        }
        match self.apply_value(path, names_value(names)) {
            Ok(true) => FormAction::Changed,
            Ok(false) => FormAction::None,
            Err(error) => {
                self.error = Some(error);
                FormAction::None
            }
        }
    }

    fn move_order(&mut self, delta: isize) -> FormAction {
        let Some(row) = self.rows.get(self.selected).cloned() else {
            return FormAction::None;
        };
        let Some((path, provider)) = order_parts(&row) else {
            return FormAction::None;
        };
        let mut names = order_names(&self.document, path);
        let Some(index) = names.iter().position(|name| name == provider) else {
            self.error = Some("Turn this provider on before moving it.".to_owned());
            return FormAction::None;
        };
        let target = index as isize + delta;
        if target < 0 || target >= names.len() as isize {
            return FormAction::None;
        }
        names.swap(index, target as usize);
        match self.apply_value(path, names_value(names)) {
            Ok(true) => FormAction::Changed,
            Ok(false) => FormAction::None,
            Err(error) => {
                self.error = Some(error);
                FormAction::None
            }
        }
    }

    fn current_catalog_key(&self) -> Option<(String, String)> {
        let base_url = self.document["models"]["base_url"].as_str()?;
        let model = self.document["models"]["model"].as_str()?;
        Some((base_url.to_owned(), model.to_owned()))
    }

    fn is_catalog_path(&self, path: &[String]) -> bool {
        path == ["models", "context_window"]
    }

    fn reset_selected(&mut self) -> FormAction {
        let Some(row) = self.rows.get(self.selected).cloned() else {
            return FormAction::None;
        };
        if matches!(
            row.kind,
            RowKind::Header
                | RowKind::RawJson
                | RowKind::LookupModel
                | RowKind::EditSoul
                | RowKind::EditGuidance
                | RowKind::StartDaemon
                | RowKind::StopDaemon
                | RowKind::RestartDaemon
                | RowKind::AddImage
                | RowKind::AddStt
                | RowKind::AddTts
        ) {
            return FormAction::None;
        }
        if self.catalog_key != self.current_catalog_key() && self.is_catalog_path(&row.path) {
            self.notice = Some("Looking up this model at models.dev...".to_owned());
            return FormAction::LookupModel;
        }
        if let Some((path, _)) = order_parts(&row) {
            let Some(default) = default_order(path) else {
                return FormAction::None;
            };
            return match self.apply_value(path, default) {
                Ok(true) => {
                    self.notice = Some(format!("Reset {} to the default order.", row.label));
                    FormAction::Changed
                }
                Ok(false) => FormAction::None,
                Err(error) => {
                    self.error = Some(error);
                    FormAction::None
                }
            };
        }
        let previous = self.document.clone();
        if let Some(default) =
            default_value_with(&self.defaults, &row.path).filter(|value| !value.is_null())
        {
            let Some(slot) = value_at_mut(&mut self.document, &row.path) else {
                return FormAction::None;
            };
            *slot = default;
        } else if row.path.first().is_some_and(|key| key == "media") {
            let Some(slot) = value_at_mut(&mut self.document, &row.path) else {
                return FormAction::None;
            };
            *slot = Value::Null;
        } else {
            remove_value(&mut self.document, &row.path);
        }
        match self.finish_change(previous) {
            Ok(true) => {
                self.notice = Some(format!("Reset {} to its default.", row.label));
                FormAction::Changed
            }
            Ok(false) => FormAction::None,
            Err(error) => {
                self.error = Some(error);
                FormAction::None
            }
        }
    }

    fn commit_edit(&mut self) -> FormAction {
        let Some(edit) = self.edit.clone() else {
            return FormAction::None;
        };
        let Some(row) = self.rows.get(edit.row).cloned() else {
            self.edit = None;
            return FormAction::None;
        };
        let text = edit.input.value.trim().to_owned();
        if row.kind == RowKind::Secret && text.is_empty() {
            self.edit = None;
            return FormAction::None;
        }
        let current = value_at(&self.document, &row.path)
            .cloned()
            .unwrap_or(Value::Null);
        let value = match parse_value(row.kind, &text, &current) {
            Ok(value) => value,
            Err(error) => {
                self.error = Some(error);
                return FormAction::None;
            }
        };
        match self.apply_value(&row.path, value) {
            Ok(save) => {
                self.edit = None;
                if save {
                    FormAction::Changed
                } else {
                    FormAction::None
                }
            }
            Err(error) => {
                self.error = Some(error);
                FormAction::None
            }
        }
    }

    fn apply_value(&mut self, path: &[String], value: Value) -> Result<bool, String> {
        let previous = self.document.clone();
        let Some(slot) = value_at_mut(&mut self.document, path) else {
            return Err(
                "That setting is no longer present. Press Esc and reopen the form.".to_owned(),
            );
        };
        *slot = value;
        self.finish_change(previous)
    }

    fn finish_change(&mut self, previous: Value) -> Result<bool, String> {
        prune_empty_media(&mut self.document);
        match validate(&self.document) {
            Ok(()) => {
                self.draft = false;
                self.error = None;
                self.rebuild_rows();
                self.notice = Some("Saved. Only changed values are kept.".to_owned());
                Ok(true)
            }
            Err(error) if self.draft => {
                self.rebuild_rows();
                self.notice =
                    Some("Not saved yet. Fill in the missing values for this service.".to_owned());
                self.error = None;
                let _ = error;
                Ok(false)
            }
            Err(error) => {
                self.document = previous;
                Err(error)
            }
        }
    }

    fn add_service(&mut self, service: &str) -> FormAction {
        let value = match service {
            "image" => json!({
                "kind": "openrouter-images",
                "api_key": "",
                "model": "openai/gpt-image-2.6-sunburst"
            }),
            "stt" => json!({
                "kind": "groq",
                "api_key": "",
                "model": "whisper-large-v3-turbo"
            }),
            _ => json!({
                "kind": "elevenlabs",
                "api_key": "",
                "model": "eleven_v4",
                "voice_id": "",
                "output_format": "opus_48000_64"
            }),
        };
        let Some(object) = self.document.as_object_mut() else {
            return FormAction::None;
        };
        let media = object
            .entry("media".to_owned())
            .or_insert_with(|| json!({}));
        if !media.is_object() {
            *media = json!({});
        }
        if let Some(media) = media.as_object_mut() {
            media.insert(service.to_owned(), value);
        }
        self.draft = true;
        self.rebuild_rows();
        self.error = None;
        self.notice = Some(
            "Fill in the missing values. Nothing is saved until this service is complete."
                .to_owned(),
        );
        if let Some(index) = self.rows.iter().position(|row| {
            row.path.first().is_some_and(|key| key == "media")
                && row.path.get(1).is_some_and(|key| key == service)
        }) {
            self.selected = index;
        }
        FormAction::None
    }

    fn rebuild_rows(&mut self) {
        let selected_path = self.rows.get(self.selected).map(|row| row.path.clone());
        self.rows = build_rows(&self.document, &self.home, &self.defaults);
        if let Some(path) = selected_path
            && let Some(index) = self.rows.iter().position(|row| row.path == path)
        {
            self.selected = index;
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }

    fn step(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let mut index = self.selected as isize;
        for _ in 0..self.rows.len() {
            index += delta;
            if index < 0 {
                index = self.rows.len() as isize - 1;
            } else if index >= self.rows.len() as isize {
                index = 0;
            }
            if self.rows[index as usize].kind != RowKind::Header {
                self.selected = index as usize;
                return;
            }
        }
    }
}

fn normalize_fetch_aliases(document: &mut Value) {
    let Some(order) = document
        .get_mut("web")
        .and_then(|web| web.get_mut("fetch"))
        .and_then(|fetch| fetch.get_mut("order"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    let mut seen: Vec<String> = Vec::new();
    order.retain_mut(|value| {
        let Some(name) = value.as_str().map(str::to_owned) else {
            return true;
        };
        let name = if name == "https" {
            "http".to_owned()
        } else {
            name
        };
        let duplicate = seen.iter().any(|seen| seen == &name);
        if !duplicate {
            seen.push(name.clone());
            *value = Value::String(name);
        }
        !duplicate
    });
}

fn with_defaults(mut document: Value) -> Value {
    let mut defaults = hearth_config::default_settings_value();
    for key in ["models", "telegram"] {
        if document.get(key).is_some()
            && let Some(section_defaults) = defaults.get(key).cloned()
            && let Some(section) = document.get_mut(key)
        {
            merge_defaults(section, &section_defaults);
        }
    }
    if let Some(object) = defaults.as_object_mut() {
        object.remove("models");
        object.remove("telegram");
        object.remove("media");
    }
    merge_defaults(&mut document, &defaults);
    document
}

fn default_web() -> Value {
    hearth_config::default_settings_value()["web"].clone()
}

fn merge_defaults(document: &mut Value, defaults: &Value) {
    let (Some(document), Some(defaults)) = (document.as_object_mut(), defaults.as_object()) else {
        return;
    };
    for (key, value) in defaults {
        match document.get_mut(key) {
            Some(existing) if existing.is_object() && value.is_object() => {
                merge_defaults(existing, value);
            }
            Some(_) => {}
            None => {
                document.insert(key.clone(), value.clone());
            }
        }
    }
}

fn build_rows(document: &Value, home: &Path, defaults: &Value) -> Vec<SettingsRow> {
    let mut rows = Vec::new();
    let object = match document.as_object() {
        Some(object) => object,
        None => return rows,
    };
    let mut seen = Vec::new();
    for (key, title) in SECTION_ORDER {
        if key == "media" {
            build_media_rows(&mut rows, object.get(key), defaults);
            seen.push(key.to_owned());
            continue;
        }
        if key == "web" {
            build_web_rows(&mut rows, object.get(key), defaults);
            seen.push(key.to_owned());
            continue;
        }
        let Some(value) = object.get(key) else {
            continue;
        };
        rows.push(header(title));
        flatten(&mut rows, vec![key.to_owned()], value, defaults);
        if key == "models" {
            rows.push(action_row(
                "Refresh model info (models.dev)",
                "Refresh",
                RowKind::LookupModel,
                "Fill the context window from models.dev.",
                "Fetch the context window for the current base URL and model from models.dev. The local output reserve is left unchanged.",
            ));
        }
        if key == "identity" {
            rows.push(header("Identity files"));
            rows.push(action_row(
                "Edit SOUL.md",
                "Open",
                RowKind::EditSoul,
                "Open the identity file in the editor.",
                "Edit the agent's identity, voice, and standing character.",
            ));
            rows.push(action_row(
                "Edit HEARTH.md",
                "Open",
                RowKind::EditGuidance,
                "Open the home guidance file in the editor.",
                "Edit the standing instructions for every conversation.",
            ));
        }
        if key == "daemon" {
            rows.push(header("Daemon controls"));
            rows.push(action_row(
                "Start daemon now",
                "Start",
                RowKind::StartDaemon,
                "Start Hearth without closing this window.",
                "Start the daemon now. It uses the start mode saved above.",
            ));
            rows.push(action_row(
                "Stop daemon now",
                "Stop",
                RowKind::StopDaemon,
                "Stop the daemon this window started.",
                "Stop a daemon this window owns. A service-managed daemon is left alone.",
            ));
            rows.push(action_row(
                "Restart daemon now",
                "Restart",
                RowKind::RestartDaemon,
                "Restart the daemon this window owns.",
                "Stop and start a daemon this window owns. A service-managed daemon is left alone.",
            ));
        }
        seen.push(key.to_owned());
    }
    for (key, value) in object {
        if seen.iter().any(|seen| seen == key) {
            continue;
        }
        rows.push(header(&humanize(key)));
        flatten(&mut rows, vec![key.clone()], value, defaults);
    }
    if let Some(row) = rows
        .iter_mut()
        .find(|row| row.path == ["agent", "workspace"] && row.value == "not set")
        && !home.as_os_str().is_empty()
    {
        row.value = home.join("workspace").display().to_string();
    }
    rows.push(header("Advanced"));
    rows.push(SettingsRow {
        label: "Edit settings.json".to_owned(),
        path: Vec::new(),
        value: "All values, including new provider keys".to_owned(),
        kind: RowKind::RawJson,
        help: "Open the complete settings file in the text editor.",
        tooltip: "Use this only for values the form does not show. The file is validated before it is saved.",
        modified: false,
    });
    rows
}

fn action_row(
    label: &str,
    value: &str,
    kind: RowKind,
    help: &'static str,
    tooltip: &'static str,
) -> SettingsRow {
    SettingsRow {
        label: label.to_owned(),
        path: vec![format!("@{label}")],
        value: value.to_owned(),
        kind,
        help,
        tooltip,
        modified: false,
    }
}

fn build_web_rows(rows: &mut Vec<SettingsRow>, web: Option<&Value>, defaults: &Value) {
    let fallback = default_web();
    let web = web.unwrap_or(&fallback);
    let search = web.get("search").and_then(Value::as_object);
    let fetch = web.get("fetch").and_then(Value::as_object);

    rows.push(header("Search order"));
    let search_order = search
        .and_then(|search| search.get("order"))
        .and_then(Value::as_array)
        .map(|values| array_names(values))
        .unwrap_or_else(|| {
            SEARCH_PROVIDERS
                .iter()
                .map(|(name, _)| (*name).to_owned())
                .collect::<Vec<_>>()
        });
    for (name, label) in provider_names(&SEARCH_PROVIDERS, &search_order) {
        rows.push(order_row(
            "search",
            &name,
            &label,
            &search_order,
            RowKind::SearchOrder,
        ));
    }

    rows.push(header("Search API keys"));
    let search_providers = search
        .and_then(|search| search.get("providers"))
        .and_then(Value::as_object);
    for (name, label) in provider_names(&SEARCH_PROVIDERS, &[]) {
        rows.push(key_row(
            "search",
            &name,
            &label,
            search_providers.and_then(|providers| providers.get(&name)),
            name == "exa",
        ));
    }

    rows.push(header("Search limits"));
    if let Some(search) = search {
        for (key, value) in search {
            if key == "order" || key == "providers" {
                continue;
            }
            flatten(
                rows,
                vec!["web".to_owned(), "search".to_owned(), key.clone()],
                value,
                defaults,
            );
        }
    }

    rows.push(header("Fetch order"));
    let fetch_order = fetch
        .and_then(|fetch| fetch.get("order"))
        .and_then(Value::as_array)
        .map(|values| array_names(values))
        .unwrap_or_else(|| vec!["jina".to_owned(), "http".to_owned()]);
    for (name, label) in provider_names(&FETCH_PROVIDERS, &fetch_order) {
        rows.push(order_row(
            "fetch",
            &name,
            &label,
            &fetch_order,
            RowKind::FetchOrder,
        ));
    }

    rows.push(header("Fetch API keys"));
    let fetch_providers = fetch
        .and_then(|fetch| fetch.get("providers"))
        .and_then(Value::as_object);
    for (name, label) in FETCH_KEYED_PROVIDERS {
        rows.push(key_row(
            "fetch",
            name,
            label,
            fetch_providers.and_then(|providers| providers.get(name)),
            name == "jina",
        ));
    }

    if let Some(fetch) = fetch {
        let extra: Vec<(&String, &Value)> = fetch
            .iter()
            .filter(|(key, _)| *key != "order" && *key != "providers")
            .collect();
        if !extra.is_empty() {
            rows.push(header("Fetch limits"));
            for (key, value) in extra {
                flatten(
                    rows,
                    vec!["web".to_owned(), "fetch".to_owned(), key.clone()],
                    value,
                    defaults,
                );
            }
        }
    }
}

fn array_names(values: &[Value]) -> Vec<String> {
    values
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn provider_names(known: &[(&str, &str)], current: &[String]) -> Vec<(String, String)> {
    let mut names: Vec<(String, String)> = known
        .iter()
        .map(|(name, label)| ((*name).to_owned(), (*label).to_owned()))
        .collect();
    for name in current {
        if !names.iter().any(|(known, _)| known == name) {
            names.push((name.clone(), humanize(name)));
        }
    }
    names
}

fn order_row(
    service: &str,
    name: &str,
    label: &str,
    order: &[String],
    kind: RowKind,
) -> SettingsRow {
    let position = order.iter().position(|entry| entry == name);
    let value = match position {
        Some(index) => format!("{} of {}", index + 1, order.len()),
        None => "Off".to_owned(),
    };
    let path = vec![
        "web".to_owned(),
        service.to_owned(),
        "order".to_owned(),
        name.to_owned(),
    ];
    let modified = default_order(path.as_slice())
        .is_some_and(|default| default != names_value(order.to_vec()));
    SettingsRow {
        label: label.to_owned(),
        path,
        value,
        kind,
        help: "Enter turns a provider on or off; Shift+Up or Shift+Down moves it.",
        tooltip: "The first enabled provider is tried first, then the next. Enter turns this provider on or off. Shift+Up and Shift+Down change its position. At least one provider must stay on.",
        modified,
    }
}

fn key_row(
    service: &str,
    name: &str,
    label: &str,
    value: Option<&Value>,
    optional: bool,
) -> SettingsRow {
    let path = vec![
        "web".to_owned(),
        service.to_owned(),
        "providers".to_owned(),
        name.to_owned(),
        "api_key".to_owned(),
    ];
    let modified = value.is_some_and(|value| is_modified_path(&path, value));
    SettingsRow {
        label: format!("{label} API key"),
        path,
        value: display_value(value.unwrap_or(&Value::Null), RowKind::Secret),
        kind: RowKind::Secret,
        help: if optional {
            "Optional. This provider also works without a key."
        } else {
            "Used only when this provider is enabled."
        },
        tooltip: if optional {
            "Optional. Type a key to use the provider's authenticated endpoint, or press Enter to keep the current setting. This provider also works without a key."
        } else {
            "Type a key to use this provider. Leave it blank to keep the current key. You can also use a ${ENV_VAR} reference in settings.json."
        },
        modified,
    }
}

fn build_media_rows(rows: &mut Vec<SettingsRow>, media: Option<&Value>, defaults: &Value) {
    rows.push(header("Optional media services"));
    let object = media.and_then(Value::as_object);
    if let Some(object) = object {
        for (key, value) in object {
            if value.is_null() {
                continue;
            }
            flatten(rows, vec!["media".to_owned(), key.clone()], value, defaults);
        }
    }
    for (service, label, kind) in [
        ("image", "Add image generation…", RowKind::AddImage),
        ("stt", "Add transcription…", RowKind::AddStt),
        ("tts", "Add speech…", RowKind::AddTts),
    ] {
        let configured = object
            .and_then(|object| object.get(service))
            .and_then(Value::as_object)
            .is_some_and(|value| !value.is_empty());
        if configured {
            continue;
        }
        rows.push(SettingsRow {
            label: label.to_owned(),
            path: vec!["media".to_owned(), service.to_owned()],
            value: "Not configured".to_owned(),
            kind,
            help: "Enter to add this service. You can fill in its key next.",
            tooltip: "Enter to add this optional service. It stays an unsaved draft until every required value is filled in.",
            modified: false,
        });
    }
}

fn flatten(rows: &mut Vec<SettingsRow>, path: Vec<String>, value: &Value, defaults: &Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let mut next = path.clone();
                next.push(key.clone());
                flatten(rows, next, child, defaults);
            }
        }
        _ => {
            let key = path.last().map(String::as_str).unwrap_or_default();
            let kind = row_kind(key, value);
            let help = help_for(key, kind);
            let tooltip = tooltip_for(key, kind);
            let modified = is_modified_with(defaults, &path, value);
            rows.push(SettingsRow {
                label: label_for(&path),
                path,
                value: display_value(value, kind),
                kind,
                help,
                tooltip,
                modified,
            });
        }
    }
}

fn timezone_options(current: &str) -> Vec<String> {
    let roots = [
        "/usr/share/zoneinfo",
        "/etc/zoneinfo",
        "/var/db/timezone/zoneinfo",
    ];
    let mut zones = timezone_options_from(&roots, current);
    if zones.len() <= 1 {
        for zone in [
            "Asia/Manila",
            "Asia/Tokyo",
            "Asia/Singapore",
            "Australia/Sydney",
            "Europe/London",
            "Europe/Paris",
            "Europe/Berlin",
            "America/New_York",
            "America/Chicago",
            "America/Denver",
            "America/Los_Angeles",
            "America/Sao_Paulo",
            "Africa/Johannesburg",
            "Pacific/Auckland",
        ] {
            if !zones.iter().any(|known| known == zone) {
                zones.push(zone.to_owned());
            }
        }
        zones.sort();
    }
    if let Some(index) = zones.iter().position(|zone| zone == "UTC") {
        let utc = zones.remove(index);
        zones.insert(0, utc);
    }
    zones
}

fn timezone_options_from(roots: &[&str], current: &str) -> Vec<String> {
    let mut zones = Vec::new();
    for root in roots {
        collect_timezones(Path::new(root), Path::new(""), 0, &mut zones);
    }
    if !current.is_empty() && !zones.iter().any(|zone| zone == current) {
        zones.push(current.to_owned());
    }
    zones.sort();
    zones.dedup();
    if !zones.iter().any(|zone| zone == "UTC") {
        zones.push("UTC".to_owned());
        zones.sort();
    }
    zones
}

fn collect_timezones(root: &Path, relative: &Path, depth: usize, zones: &mut Vec<String>) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if matches!(name.as_ref(), "posix" | "right" | ".git") {
            continue;
        }
        let path = entry.path();
        let child = relative.join(name.as_ref());
        if path.is_dir() {
            collect_timezones(&path, &child, depth + 1, zones);
            continue;
        }
        if matches!(
            name.as_ref(),
            "localtime"
                | "posixrules"
                | "zone.tab"
                | "zone1970.tab"
                | "zonenow.tab"
                | "iso3166.tab"
                | "leapseconds"
                | "leap-seconds.list"
                | "tzdata.zi"
                | "SECURITY"
                | "README"
                | "Makefile"
                | "+VERSION"
        ) || name.contains('.')
        {
            continue;
        }
        zones.push(child.display().to_string());
    }
}

fn header(title: &str) -> SettingsRow {
    SettingsRow {
        label: title.to_owned(),
        path: Vec::new(),
        value: String::new(),
        kind: RowKind::Header,
        help: "",
        tooltip: "",
        modified: false,
    }
}

fn row_kind(key: &str, value: &Value) -> RowKind {
    if is_secret(key) {
        RowKind::Secret
    } else {
        if key == "start" {
            RowKind::DaemonStart
        } else if key == "thinking" {
            RowKind::Thinking
        } else if key == "level" {
            RowKind::LogLevel
        } else if key == "timezone" {
            RowKind::Timezone
        } else {
            match value {
                Value::Bool(_) => RowKind::Bool,
                Value::Number(number) if number.is_f64() => RowKind::Float,
                Value::Number(_) => RowKind::Integer,
                Value::Array(_) => RowKind::Array,
                _ => RowKind::Text,
            }
        }
    }
}

fn is_secret(key: &str) -> bool {
    key.contains("api_key") || key == "bot_token"
}

fn display_value(value: &Value, kind: RowKind) -> String {
    match kind {
        RowKind::DaemonStart => {
            return match value.as_str() {
                Some("login") => "At login".to_owned(),
                Some("startup") => "At startup".to_owned(),
                _ => "When Hearth opens".to_owned(),
            };
        }
        RowKind::Thinking => {
            return match value.as_str() {
                Some("off") => "Off".to_owned(),
                Some("low") => "Low".to_owned(),
                Some("medium") => "Medium".to_owned(),
                Some("high") => "High".to_owned(),
                Some("xhigh") => "Extra high".to_owned(),
                Some("max") => "Maximum".to_owned(),
                _ => "Provider default".to_owned(),
            };
        }
        RowKind::LogLevel => {
            return capitalize(value.as_str().unwrap_or("info"));
        }
        _ => {}
    }
    if kind == RowKind::Secret {
        return match value.as_str() {
            Some(text) if !text.is_empty() => "••••••••".to_owned(),
            _ => "not set".to_owned(),
        };
    }
    match value {
        Value::Bool(value) => {
            if *value {
                "Yes".to_owned()
            } else {
                "No".to_owned()
            }
        }
        Value::String(value) if value.is_empty() => "not set".to_owned(),
        Value::String(value) => value.clone(),
        Value::Array(_) => array_text(value),
        Value::Null => "not set".to_owned(),
        other => other.to_string(),
    }
}

fn plain_text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn array_text(value: &Value) -> String {
    let Some(items) = value.as_array() else {
        return value.to_string();
    };
    if items.is_empty() {
        return "none".to_owned();
    }
    items
        .iter()
        .map(|item| match item {
            Value::String(value) => value.clone(),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn cycle_value(kind: RowKind, document: &Value, path: &[String]) -> Option<Value> {
    let current = value_at(document, path)
        .and_then(Value::as_str)
        .unwrap_or_default();
    let values: &[&str] = match kind {
        RowKind::DaemonStart => &["tui", "login", "startup"],
        RowKind::Thinking => &["default", "off", "low", "medium", "high", "xhigh", "max"],
        RowKind::LogLevel => &["trace", "debug", "info", "warn", "error"],
        _ => return None,
    };
    let index = values
        .iter()
        .position(|value| *value == current)
        .unwrap_or(0);
    let next = values[(index + 1) % values.len()];
    Some(Value::String(next.to_owned()))
}

fn parse_value(kind: RowKind, text: &str, current: &Value) -> Result<Value, String> {
    match kind {
        RowKind::Secret | RowKind::Text => Ok(Value::String(text.to_owned())),
        RowKind::Integer => {
            if current.is_u64() {
                text.parse::<u64>()
                    .map(Value::from)
                    .map_err(|_| "Enter a whole number greater than or equal to zero.".to_owned())
            } else {
                text.parse::<i64>()
                    .map(Value::from)
                    .map_err(|_| "Enter a whole number.".to_owned())
            }
        }
        RowKind::Float => {
            let value = text
                .parse::<f64>()
                .map_err(|_| "Enter a number, for example 0.65.".to_owned())?;
            if !value.is_finite() {
                return Err("Enter a finite number.".to_owned());
            }
            serde_json::Number::from_f64(value)
                .map(Value::Number)
                .ok_or_else(|| "Enter a finite number.".to_owned())
        }
        RowKind::Array => parse_array(text, current),
        _ => Ok(Value::String(text.to_owned())),
    }
}

fn parse_array(text: &str, current: &Value) -> Result<Value, String> {
    if text.is_empty() {
        return Ok(Value::Array(Vec::new()));
    }
    if text.starts_with('[') {
        let value: Value = serde_json::from_str(text).map_err(|error| {
            format!("Enter a JSON list, or separate items with commas: {error}")
        })?;
        if !value.is_array() {
            return Err("This setting must be a list.".to_owned());
        }
        return Ok(value);
    }
    let first = current
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .unwrap_or(Value::String(String::new()));
    let mut items = Vec::new();
    for part in text
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let item = match &first {
            Value::Number(number) if number.is_u64() => part
                .parse::<u64>()
                .map(Value::from)
                .map_err(|_| format!("`{part}` is not a whole number."))?,
            Value::Number(_) => part
                .parse::<i64>()
                .map(Value::from)
                .map_err(|_| format!("`{part}` is not a number."))?,
            Value::Bool(_) => part
                .parse::<bool>()
                .map(Value::from)
                .map_err(|_| format!("`{part}` is not true or false."))?,
            _ => Value::String(part.to_owned()),
        };
        items.push(item);
    }
    Ok(Value::Array(items))
}

fn value_at<'a>(document: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(document, |value, key| value.get(key))
}

fn value_at_mut<'a>(document: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    let (last, parents) = path.split_last()?;
    let mut value = document;
    for key in parents {
        if value.get(key).is_none() {
            value
                .as_object_mut()?
                .insert(key.clone(), Value::Object(serde_json::Map::new()));
        }
        value = value.get_mut(key)?;
    }
    value
        .as_object_mut()?
        .entry(last.clone())
        .or_insert(Value::Null);
    value.get_mut(last)
}

fn remove_value(document: &mut Value, path: &[String]) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut value = document;
    for key in parents {
        let Some(next) = value.get_mut(key) else {
            return;
        };
        value = next;
    }
    if let Some(object) = value.as_object_mut() {
        object.remove(last);
    }
}

fn prune_empty_media(document: &mut Value) {
    let Some(media) = document.get_mut("media").and_then(Value::as_object_mut) else {
        return;
    };
    media.retain(|_, value| value.as_object().is_none_or(|object| !object.is_empty()));
}

fn apply_catalog_defaults(defaults: &mut Value, limits: hearth_model::ModelLimits) {
    if limits.context_window > 0 {
        defaults["models"]["context_window"] = json!(limits.context_window);
    }
}

fn default_value_at(path: &[String]) -> Option<Value> {
    value_at(&hearth_config::default_settings_value(), path).cloned()
}

fn default_value_with(defaults: &Value, path: &[String]) -> Option<Value> {
    value_at(defaults, path).cloned()
}

fn default_order(path: &[String]) -> Option<Value> {
    let order_path = if path.last().is_some_and(|key| key == "order") {
        path
    } else {
        path.split_last()?.1
    };
    if order_path.last().is_none_or(|key| key != "order") {
        return None;
    }
    default_value_at(order_path)
}

fn is_modified_path(path: &[String], value: &Value) -> bool {
    if let Some(default) = default_value_at(path) {
        return value != &default;
    }
    !value.is_null() && !value.as_str().is_some_and(str::is_empty)
}

fn is_modified_with(defaults: &Value, path: &[String], value: &Value) -> bool {
    if let Some(default) = default_value_with(defaults, path) {
        return value != &default;
    }
    !value.is_null() && !value.as_str().is_some_and(str::is_empty)
}

fn order_parts(row: &SettingsRow) -> Option<(&[String], &str)> {
    if !matches!(row.kind, RowKind::SearchOrder | RowKind::FetchOrder) {
        return None;
    }
    let (provider, path) = row.path.split_last()?;
    path.last()
        .is_some_and(|key| key == "order")
        .then_some((path, provider.as_str()))
}

fn order_names(document: &Value, path: &[String]) -> Vec<String> {
    value_at(document, path)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn names_value(names: Vec<String>) -> Value {
    Value::Array(names.into_iter().map(Value::String).collect())
}

fn validate(document: &Value) -> Result<(), String> {
    let settings: hearth_config::Settings = serde_json::from_value(document.clone())
        .map_err(|error| format!("Check this setting: {error}"))?;
    settings.validate().map_err(|error| error.to_string())
}

fn label_for(path: &[String]) -> String {
    let parts = if path.len() > 1 { &path[1..] } else { path };
    parts
        .iter()
        .map(|part| humanize(part))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn humanize(key: &str) -> String {
    let label = match key {
        "agent_name" => "Name",
        "timezone" => "Timezone",
        "start" => "Start",
        "keep_running" => "Keep running after close",
        "kind" => "Kind",
        "base_url" => "Base URL",
        "api_key" => "API key",
        "model" => "Model",
        "context_window" => "Context window (local)",
        "max_output_tokens" => "Output reserve (local)",
        "session_header" => "Session header",
        "thinking" => "Thinking",
        "headers" => "Headers",
        "bot_token" => "Bot token",
        "allowed_user_ids" => "Allowed user IDs",
        "allowed_chat_ids" => "Allowed chat IDs",
        "image" => "Image generation",
        "stt" => "Transcription",
        "tts" => "Speech",
        "output_format" => "Output format",
        "voice_id" => "Voice",
        "order" => "Order",
        "parallel" => "Parallel searches",
        "results" => "Results per search",
        "max_results" => "Maximum results",
        "providers" => "Providers",
        "workspace" => "Workspace",
        "max_turns" => "Max turns",
        "max_concurrent" => "Max concurrent tasks",
        "max_depth" => "Max helper depth",
        "retention_days" => "Helper retention days",
        "retention_runs" => "Helper retention runs",
        "eta_reminder_seconds" => "ETA reminder delay (seconds)",
        "idle_minutes" => "Idle minutes",
        "idle_min_tokens" => "Idle token threshold",
        "soft_compact" => "Soft compaction",
        "hard_compact" => "Hard compaction",
        "keep_recent_tokens" => "Keep recent tokens",
        "standing_max_tokens" => "Standing memory budget",
        "recall_max_tokens" => "Recall budget",
        "level" => "Log level",
        _ => {
            return key
                .replace('_', " ")
                .split_whitespace()
                .map(capitalize)
                .collect::<Vec<_>>()
                .join(" ");
        }
    };
    label.to_owned()
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn help_for(key: &str, kind: RowKind) -> &'static str {
    match key {
        "agent_name" => "The name Hearth uses for itself.",
        "timezone" => "Enter opens the timezone picker.",
        "api_key" => "Type a new key, or leave it blank to keep the current one.",
        "bot_token" => "Type a new bot token, or leave it blank to keep the current one.",
        "start" => "When Hearth opens, at login, or at startup. Enter to change.",
        "keep_running" => "Whether a daemon started by this window stays up after it closes.",
        "thinking" => "Provider default, off, low, medium, high, xhigh, or max.",
        "context_window" => "The model's full context window in tokens, stored locally.",
        "max_output_tokens" => {
            "Local space reserved for answers when planning context. Not sent to the provider."
        }
        "max_turns" => "Maximum model requests in one run. Default 120.",
        "workspace" => "The folder tools may read and write. Empty uses the default workspace.",
        "soft_compact" => "A fraction of the usable window, for example 0.65.",
        "hard_compact" => "A fraction of the usable window, for example 0.85.",
        "allowed_user_ids" => "Telegram numeric IDs, separated by commas.",
        "allowed_chat_ids" => "Telegram numeric chat IDs, separated by commas.",
        "parallel" => "How many searches run at once, 1 to 3.",
        "results" => "Results requested from each provider, 1 to 20.",
        "max_results" => "Results returned after merging, 1 to 50.",
        "order" => "Provider names in the order Hearth tries them.",
        "level" => "trace, debug, info, warn, or error.",
        "output_format" => "For example opus_48000_64.",
        _ if kind == RowKind::Bool => "Enter to turn this on or off.",
        _ if kind == RowKind::Array => "Items are separated by commas.",
        _ if kind == RowKind::Secret => "Stored locally with mode 0600.",
        _ => "Enter to change this value.",
    }
}

fn tooltip_for(key: &str, kind: RowKind) -> &'static str {
    match key {
        "agent_name" => {
            "The name Hearth uses for itself in prompts and replies. Change it if you prefer another name."
        }
        "timezone" => {
            "An IANA timezone name such as Asia/Manila or Europe/London. Enter opens the picker; schedules, reminders, and local timestamps use this value."
        }
        "start" => {
            "When Hearth starts: when the TUI opens, at login, or at startup. Enter cycles through the choices."
        }
        "keep_running" => {
            "When on, a daemon started by this window keeps running after the window closes. When off, it stops with the window."
        }
        "kind" => {
            "The provider protocol. Hearth currently supports openai-compatible model endpoints."
        }
        "base_url" => {
            "The provider's OpenAI-compatible base URL, including any version path such as /v1."
        }
        "api_key" => {
            "Type a new key, or press Enter to keep the current one. You can also use a ${ENV_VAR} reference in settings.json."
        }
        "model" => "The model id sent to the provider, for example deepseek-v4.1-flash.",
        "context_window" => {
            "The model's full context window in tokens. Hearth uses it locally to plan compaction and reply space; it is not sent to the provider. Refresh it from models.dev above."
        }
        "max_output_tokens" => {
            "Local only. Tokens reserved for the model's reply when Hearth calculates the context budget. It is not sent as max_tokens; the provider's own output limit applies."
        }
        "session_header" => {
            "Optional. Send the conversation id in this HTTP header for gateways that route requests by session."
        }
        "thinking" => {
            "Provider default leaves thinking unchanged. Off disables it; low through max set reasoning effort."
        }
        "headers" => {
            "Extra HTTP headers sent to the model provider. Use the raw settings editor for this map."
        }
        "bot_token" => {
            "The Telegram bot token from BotFather. Press Enter to keep the current token."
        }
        "allowed_user_ids" => {
            "Telegram numeric user ids allowed to talk to Hearth, separated by commas. At least one is required when Telegram is on."
        }
        "allowed_chat_ids" => {
            "Optional Telegram chat ids. When set, both the user and the chat must be allowed."
        }
        "workspace" => {
            "The folder file tools may read and write. A relative path lives inside the Hearth home; empty uses the default workspace folder."
        }
        "max_turns" => {
            "Maximum model requests in one run. Each tool call and follow-up counts as another request. The default is 120."
        }
        "max_concurrent" => {
            "How many helper tasks may run at the same time. Enter a whole number from 1 to 8."
        }
        "max_depth" => {
            "How deep helpers may spawn more helpers: 1 means helpers cannot spawn helpers; 2 allows one level."
        }
        "retention_days" => {
            "Delete helper run records older than this many days. Runs are also capped by retention_runs."
        }
        "retention_runs" => {
            "Keep at most this many recent helper runs, even when they are newer than retention_days."
        }
        "eta_reminder_seconds" => {
            "Seconds after a task's estimated finish before Hearth sends the reminder. The default is 1 second."
        }
        "idle_minutes" => "Minutes without activity before Hearth compacts a large conversation.",
        "idle_min_tokens" => {
            "Idle compaction runs only when the conversation is at least this many tokens."
        }
        "soft_compact" => {
            "Compact at this fraction of the usable context. It must be above 0 and below hard_compact; 0.65 is the default."
        }
        "hard_compact" => {
            "Force compaction at this fraction of the usable context. It must be below 1; 0.85 is the default."
        }
        "keep_recent_tokens" => {
            "Recent conversation tokens kept verbatim when older turns are compacted."
        }
        "standing_max_tokens" => "Token budget for standing memory included in every prompt.",
        "recall_max_tokens" => "Token budget for memories recalled for each user message.",
        "level" => {
            "Diagnostic verbosity: trace, debug, info, warn, or error. Enter cycles through the choices."
        }
        "output_format" => "Speech audio format for ElevenLabs, for example opus_48000_64.",
        "voice_id" => "The ElevenLabs voice id used for speech replies.",
        _ if kind == RowKind::Secret => {
            "Stored locally with mode 0600. Press Enter to keep the current value."
        }
        _ if kind == RowKind::Bool => {
            "Enter turns this setting on or off. The change is validated and saved immediately."
        }
        _ if kind == RowKind::Array => {
            "Enter opens a comma-separated list. The change is validated and saved immediately."
        }
        _ if kind == RowKind::Integer => {
            "Enter a whole number. The change is validated before it is saved."
        }
        _ if kind == RowKind::Float => {
            "Enter a decimal number, for example 0.65. The change is validated before it is saved."
        }
        _ => "Enter to change this value, then press Enter to save. Esc cancels without saving.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use tempfile::TempDir;

    fn form() -> SettingsForm {
        SettingsForm::from_document(json!({
            "identity": {"agent_name": "Hearth", "timezone": "UTC"},
            "daemon": {"start": "tui", "keep_running": true},
            "models": {
                "kind": "openai-compatible",
                "base_url": "https://example.com/v1",
                "api_key": "fake-key",
                "model": "fake-model",
                "context_window": 128000,
                "max_output_tokens": 32000
            },
            "telegram": {"bot_token": "123:fake", "allowed_user_ids": [42]},
        }))
        .expect("form")
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn row_index(form: &SettingsForm, path: &[&str]) -> usize {
        form.rows
            .iter()
            .position(|row| {
                row.path.len() == path.len()
                    && row.path.iter().zip(path).all(|(left, right)| left == right)
            })
            .unwrap_or_else(|| panic!("missing row {path:?}"))
    }

    fn order(form: &SettingsForm, kind: &str, field: &str) -> Vec<String> {
        form.document["web"][kind][field]
            .as_array()
            .expect("order array")
            .iter()
            .map(|value| value.as_str().expect("provider name").to_owned())
            .collect()
    }

    #[test]
    fn identity_has_no_soul_file_row_and_120_turns_by_default() {
        let form = form();
        assert!(
            !form
                .rows
                .iter()
                .any(|row| row.path == ["identity", "soul_file"])
        );
        let turns = &form.rows[row_index(&form, &["agent", "max_turns"])];
        assert_eq!(turns.value, "120");
    }

    #[test]
    fn web_provider_order_is_toggled_instead_of_typed() {
        let mut form = form();
        let brave = row_index(&form, &["web", "search", "order", "brave"]);
        form.selected = brave;
        assert_eq!(form.handle_key(key(KeyCode::Enter)), FormAction::Changed);
        assert!(!order(&form, "search", "order").contains(&"brave".to_owned()));

        let brave = row_index(&form, &["web", "search", "order", "brave"]);
        form.selected = brave;
        assert_eq!(form.handle_key(key(KeyCode::Enter)), FormAction::Changed);
        assert_eq!(
            order(&form, "search", "order"),
            vec!["exa", "deepseek", "firecrawl", "tinyfish", "brave"]
        );
    }

    #[test]
    fn web_provider_order_moves_with_shift_arrows() {
        let mut form = form();
        let brave = row_index(&form, &["web", "search", "order", "brave"]);
        form.selected = brave;
        let shift_up = KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT);
        assert_eq!(form.handle_key(shift_up), FormAction::Changed);
        assert_eq!(
            order(&form, "search", "order"),
            vec!["brave", "exa", "deepseek", "firecrawl", "tinyfish"]
        );
    }

    #[test]
    fn the_last_web_provider_cannot_be_disabled() {
        let mut form = SettingsForm::from_document(json!({
            "identity": {"agent_name": "Hearth", "timezone": "UTC"},
            "web": {"search": {"order": ["exa"]}}
        }))
        .expect("form");
        let exa = row_index(&form, &["web", "search", "order", "exa"]);
        form.selected = exa;
        assert_eq!(form.handle_key(key(KeyCode::Enter)), FormAction::None);
        assert_eq!(order(&form, "search", "order"), vec!["exa"]);
        assert!(
            form.error
                .as_deref()
                .is_some_and(|error| error.contains("at least one")),
            "{:?}",
            form.error
        );
    }

    #[test]
    fn web_api_keys_can_be_set_without_raw_json() {
        let mut form = form();
        for path in [
            ["web", "search", "providers", "brave", "api_key"],
            ["web", "fetch", "providers", "firecrawl", "api_key"],
        ] {
            let index = row_index(&form, &path);
            form.selected = index;
            form.handle_key(key(KeyCode::Enter));
            let edit = form.edit.as_mut().expect("key edit");
            edit.input.value = "new-key".to_owned();
            edit.input.cursor = edit.input.value.chars().count();
            assert_eq!(form.handle_key(key(KeyCode::Enter)), FormAction::Changed);
        }
        assert_eq!(
            form.document["web"]["search"]["providers"]["brave"]["api_key"],
            "new-key"
        );
        assert_eq!(
            form.document["web"]["fetch"]["providers"]["firecrawl"]["api_key"],
            "new-key"
        );
    }

    #[test]
    fn editing_a_vague_setting_shows_a_tooltip() {
        let mut form = form();
        let index = row_index(&form, &["agent", "max_turns"]);
        form.selected = index;
        form.handle_key(key(KeyCode::Enter));
        let hint = form.edit.as_ref().expect("edit").hint;
        assert!(hint.contains("model requests"), "{hint}");
    }

    #[test]
    fn rows_use_friendly_labels_and_mask_secrets() {
        let form = form();
        let labels: Vec<&str> = form.rows.iter().map(|row| row.label.as_str()).collect();
        assert!(labels.contains(&"Name"));
        assert!(labels.contains(&"API key"));
        assert!(labels.contains(&"Keep running after close"));
        let key_row = form
            .rows
            .iter()
            .find(|row| row.label == "API key")
            .expect("key row");
        assert_eq!(key_row.value, "••••••••");
    }

    #[test]
    fn defaults_and_friendly_choices_are_visible() {
        let form = form();
        let labels: Vec<&str> = form.rows.iter().map(|row| row.label.as_str()).collect();
        assert!(labels.contains(&"Tasks and helpers"));
        assert!(labels.contains(&"Logging"));
        assert!(labels.contains(&"Search order"));
        let start = form
            .rows
            .iter()
            .find(|row| row.label == "Start")
            .expect("start");
        assert_eq!(start.value, "When Hearth opens");
        assert_eq!(start.kind, RowKind::DaemonStart);
    }

    #[test]
    fn editing_a_value_updates_the_document() {
        let mut form = form();
        let index = form
            .rows
            .iter()
            .position(|row| row.label == "Name")
            .expect("name row");
        form.selected = index;
        form.handle_key(key(KeyCode::Enter));
        form.edit.as_mut().expect("edit").input.value = "Ember".to_owned();
        assert_eq!(form.handle_key(key(KeyCode::Enter)), FormAction::Changed);
        assert_eq!(form.document["identity"]["agent_name"], "Ember");
    }

    #[test]
    fn an_optional_service_can_be_added_and_completed() {
        let mut form = form();
        let add = form
            .rows
            .iter()
            .position(|row| row.label == "Add image generation…")
            .expect("add image row");
        form.selected = add;
        assert_eq!(form.handle_key(key(KeyCode::Enter)), FormAction::None);
        assert!(form.draft);
        let key_row = form
            .rows
            .iter()
            .position(|row| row.path == ["media", "image", "api_key"])
            .expect("new key row");
        form.selected = key_row;
        form.handle_key(key(KeyCode::Enter));
        form.edit.as_mut().expect("edit").input.value = "sk-new".to_owned();
        assert_eq!(form.handle_key(key(KeyCode::Enter)), FormAction::Changed);
        assert!(!form.draft);
        assert_eq!(form.document["media"]["image"]["api_key"], "sk-new");
    }

    #[test]
    fn a_bad_value_is_not_applied() {
        let mut form = form();
        let index = form
            .rows
            .iter()
            .position(|row| row.label.starts_with("Context window"))
            .expect("a model number");
        form.selected = index;
        form.handle_key(key(KeyCode::Enter));
        form.edit.as_mut().expect("edit").input.value = "zero".to_owned();
        assert_eq!(form.handle_key(key(KeyCode::Enter)), FormAction::None);
        assert_eq!(form.document["models"]["context_window"], 128000);
        assert!(form.error.is_some());
    }

    #[test]
    fn only_changed_values_are_written_to_settings_json() {
        let form = form();
        let text = form.text();
        let value: Value = serde_json::from_str(&text).expect("compact settings");
        assert!(value.get("identity").is_none(), "{text}");
        assert!(value.get("daemon").is_none(), "{text}");
        assert!(value.get("agent").is_none(), "{text}");
        assert!(value["models"].get("context_window").is_none(), "{text}");
        assert!(value["models"].get("max_output_tokens").is_none(), "{text}");
        assert_eq!(value["models"]["kind"], "openai-compatible");
        assert_eq!(value["models"]["model"], "fake-model");
        assert_eq!(value["telegram"]["bot_token"], "123:fake");
        assert_eq!(value["telegram"]["allowed_user_ids"], json!([42]));
    }

    #[test]
    fn a_changed_value_is_marked_and_r_resets_it() {
        let mut form = SettingsForm::from_document(json!({
            "models": {
                "kind": "openai-compatible",
                "base_url": "https://example.com/v1",
                "api_key": "fake-key",
                "model": "fake-model"
            },
            "agent": {"max_turns": 60}
        }))
        .expect("form");
        let index = row_index(&form, &["agent", "max_turns"]);
        assert!(form.rows[index].modified);
        form.selected = index;
        assert_eq!(
            form.handle_key(key(KeyCode::Char('r'))),
            FormAction::Changed
        );
        assert_eq!(form.document["agent"]["max_turns"], 120);
        assert!(!form.rows[row_index(&form, &["agent", "max_turns"])].modified);
        assert!(!form.text().contains("max_turns"));
    }

    #[test]
    fn resetting_a_provider_restores_the_default_order() {
        let mut form = SettingsForm::from_document(json!({
            "models": {
                "kind": "openai-compatible",
                "base_url": "https://example.com/v1",
                "api_key": "fake-key",
                "model": "fake-model"
            },
            "web": {"search": {"order": ["exa"]}}
        }))
        .expect("form");
        let index = row_index(&form, &["web", "search", "order", "exa"]);
        assert!(form.rows[index].modified);
        form.selected = index;
        assert_eq!(
            form.handle_key(key(KeyCode::Char('r'))),
            FormAction::Changed
        );
        assert_eq!(
            order(&form, "search", "order"),
            vec!["exa", "brave", "deepseek", "firecrawl", "tinyfish"]
        );
        assert!(!form.rows[row_index(&form, &["web", "search", "order", "exa"])].modified);
    }

    #[test]
    fn model_defaults_stay_visible_when_the_file_omits_them() {
        let form = SettingsForm::from_document(json!({
            "models": {
                "kind": "openai-compatible",
                "base_url": "https://example.com/v1",
                "api_key": "fake-key",
                "model": "fake-model"
            }
        }))
        .expect("form");
        assert_eq!(
            form.rows[row_index(&form, &["models", "context_window"])].value,
            "128000"
        );
        assert_eq!(
            form.rows[row_index(&form, &["models", "thinking"])].value,
            "Provider default"
        );
    }

    #[test]
    fn settings_rows_include_the_file_and_daemon_actions() {
        let form = form();
        for kind in [
            RowKind::EditSoul,
            RowKind::EditGuidance,
            RowKind::StartDaemon,
            RowKind::StopDaemon,
            RowKind::RestartDaemon,
        ] {
            assert!(
                form.rows.iter().any(|row| row.kind == kind),
                "missing action {kind:?}"
            );
        }
        for (kind, value) in [
            (RowKind::StartDaemon, "Start"),
            (RowKind::StopDaemon, "Stop"),
            (RowKind::RestartDaemon, "Restart"),
        ] {
            let row = form
                .rows
                .iter()
                .find(|row| row.kind == kind)
                .expect("daemon action");
            assert_eq!(row.value, value);
        }
    }
    #[test]
    fn the_timezone_row_opens_a_picker() {
        let mut form = SettingsForm::from_document(json!({
            "identity": {"agent_name": "Hearth", "timezone": "Asia/Manila"},
            "models": {
                "kind": "openai-compatible",
                "base_url": "https://example.com/v1",
                "api_key": "fake-key",
                "model": "fake-model"
            }
        }))
        .expect("form");
        let index = row_index(&form, &["identity", "timezone"]);
        form.selected = index;
        assert_eq!(form.handle_key(key(KeyCode::Enter)), FormAction::None);
        let picker = form.picker.as_mut().expect("picker");
        assert_eq!(picker.options.first().map(String::as_str), Some("UTC"));
        picker.selected = 0;
        assert_eq!(form.handle_key(key(KeyCode::Enter)), FormAction::Changed);
        assert_eq!(form.document["identity"]["timezone"], "UTC");
        assert!(form.picker.is_none());
    }

    #[test]
    fn workspace_shows_the_default_folder() {
        let home = TempDir::new().expect("home");
        let form = SettingsForm::from_document_at(
            home.path().to_path_buf(),
            json!({
                "models": {
                    "kind": "openai-compatible",
                    "base_url": "https://example.com/v1",
                    "api_key": "fake-key",
                    "model": "fake-model"
                }
            }),
        )
        .expect("form");
        assert_eq!(
            form.rows[row_index(&form, &["agent", "workspace"])].value,
            home.path().join("workspace").display().to_string()
        );
    }

    #[test]
    fn context_window_is_labeled_local() {
        let form = form();
        assert!(
            form.rows
                .iter()
                .any(|row| row.label == "Context window (local)")
        );
    }

    #[test]
    fn a_legacy_https_fetch_alias_is_normalized() {
        let form = SettingsForm::from_document(json!({
            "models": {
                "kind": "openai-compatible",
                "base_url": "https://example.com/v1",
                "api_key": "fake-key",
                "model": "fake-model"
            },
            "web": {"fetch": {"order": ["jina", "https", "http"]}}
        }))
        .expect("form");
        assert_eq!(
            form.document["web"]["fetch"]["order"],
            json!(["jina", "http"])
        );
        assert!(!form.rows.iter().any(|row| row.label == "Direct HTTPS"));
    }

    #[test]
    fn only_one_direct_fetch_row_is_offered() {
        let form = form();
        let labels: Vec<&str> = form.rows.iter().map(|row| row.label.as_str()).collect();
        assert!(labels.contains(&"Direct HTTP"));
        assert!(!labels.contains(&"Direct HTTPS"));
    }

    #[test]
    fn an_empty_media_service_still_offers_to_add_it() {
        let form = SettingsForm::from_document(json!({
            "models": {
                "kind": "openai-compatible",
                "base_url": "https://example.com/v1",
                "api_key": "fake-key",
                "model": "fake-model"
            },
            "media": {"image": {}}
        }))
        .expect("form");
        assert!(
            form.rows
                .iter()
                .any(|row| row.label == "Add image generation…")
        );
    }

    #[test]
    fn a_legacy_pricing_block_is_not_shown() {
        let form = SettingsForm::from_document(json!({
            "models": {
                "kind": "openai-compatible",
                "base_url": "https://example.com/v1",
                "api_key": "fake-key",
                "model": "fake-model",
                "pricing": {"fake-model": {"input": 0.15}}
            }
        }))
        .expect("form");
        assert!(!form.rows.iter().any(|row| {
            row.path
                .starts_with(&["models".to_owned(), "pricing".to_owned()])
        }));
        assert!(!form.text().contains("pricing"));
    }

    #[test]
    fn a_missing_catalog_lookup_makes_reset_fetch_models_dev() {
        let mut form = form();
        let index = row_index(&form, &["models", "context_window"]);
        form.selected = index;
        assert_eq!(
            form.handle_key(key(KeyCode::Char('r'))),
            FormAction::LookupModel
        );
    }

    #[test]
    fn catalog_defaults_survive_reopening_the_form() {
        let home = TempDir::new().expect("home");
        let mut form = SettingsForm::from_document_at(
            home.path().to_path_buf(),
            json!({
                "models": {
                    "kind": "openai-compatible",
                    "base_url": "https://example.com/v1",
                    "api_key": "fake-key",
                    "model": "fake-model"
                }
            }),
        )
        .expect("form");
        assert!(form.apply_model_info(hearth_model::ModelLimits {
            context_window: 1_000_000,
            max_output_tokens: 384_000,
        }));
        let text: Value = serde_json::from_str(&form.text()).expect("json");
        let mut reopened =
            SettingsForm::from_document_at(home.path().to_path_buf(), text).expect("reopen");
        let context = row_index(&reopened, &["models", "context_window"]);
        reopened.selected = context;
        assert_eq!(
            reopened.handle_key(key(KeyCode::Char('r'))),
            FormAction::Changed
        );
        assert_eq!(reopened.document["models"]["context_window"], 1_000_000);
    }

    #[test]
    fn resetting_a_draft_service_keeps_its_rows() {
        let mut form = form();
        let add = form
            .rows
            .iter()
            .position(|row| row.label == "Add image generation…")
            .expect("add image row");
        form.selected = add;
        form.handle_key(key(KeyCode::Enter));
        for path in [
            ["media", "image", "kind"],
            ["media", "image", "api_key"],
            ["media", "image", "model"],
        ] {
            let index = row_index(&form, &path);
            form.selected = index;
            form.handle_key(key(KeyCode::Char('r')));
            assert!(row_index(&form, &path) > 0);
        }
        assert!(form.document["media"]["image"].is_object());
        assert!(
            form.rows
                .iter()
                .any(|row| row.path == ["media", "image", "kind"] && row.value == "not set")
        );
    }

    #[test]
    fn resetting_a_required_media_field_keeps_the_service() {
        let mut form = SettingsForm::from_document(json!({
            "models": {
                "kind": "openai-compatible",
                "base_url": "https://example.com/v1",
                "api_key": "fake-key",
                "model": "fake-model"
            },
            "media": {
                "image": {
                    "kind": "openrouter-images",
                    "api_key": "image-key",
                    "model": "openai/gpt-image-2.6-sunburst"
                }
            }
        }))
        .expect("form");
        let index = row_index(&form, &["media", "image", "kind"]);
        form.selected = index;
        form.handle_key(key(KeyCode::Char('r')));
        assert!(form.document["media"]["image"].is_object());
        assert_eq!(form.document["media"]["image"]["kind"], "openrouter-images");
    }

    #[test]
    fn the_old_eta_minutes_key_is_not_shown() {
        let form = SettingsForm::from_document(json!({
            "models": {
                "kind": "openai-compatible",
                "base_url": "https://example.com/v1",
                "api_key": "fake-key",
                "model": "fake-model"
            },
            "tasks": {"eta_reminder_minutes": 10}
        }))
        .expect("form");
        assert!(
            !form
                .rows
                .iter()
                .any(|row| row.label.contains("ETA reminder minutes"))
        );
        assert!(
            form.rows
                .iter()
                .any(|row| row.label.contains("ETA reminder"))
        );
    }

    #[test]
    fn the_model_info_action_fills_the_context_window() {
        let mut form = form();
        let index = form
            .rows
            .iter()
            .position(|row| row.kind == RowKind::LookupModel)
            .expect("lookup action");
        form.selected = index;
        assert_eq!(
            form.handle_key(key(KeyCode::Enter)),
            FormAction::LookupModel
        );
        assert!(form.apply_model_info(hearth_model::ModelLimits {
            context_window: 1_000_000,
            max_output_tokens: 384_000,
        }));
        assert_eq!(form.document["models"]["context_window"], 1_000_000);
        assert_eq!(form.document["models"]["max_output_tokens"], 32000);
        assert!(
            form.notice
                .as_deref()
                .is_some_and(|notice| notice.contains("models.dev"))
        );
        let context = row_index(&form, &["models", "context_window"]);
        assert!(!form.rows[context].modified);
        form.selected = context;
        assert_eq!(
            form.handle_key(key(KeyCode::Char('r'))),
            FormAction::Changed
        );
        assert_eq!(form.document["models"]["context_window"], 1_000_000);
    }
}
