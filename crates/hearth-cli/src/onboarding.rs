//! First-run onboarding state machine.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use hearth_config::Daemon;
use hearth_runtime::InitOptions;
use serde_json::json;

use crate::providers::{PROVIDERS, THINKING};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Welcome,
    Telegram,
    Provider,
    Credentials,
    Model,
    Thinking,
    Agent,
    Extras,
    Daemon,
    Review,
    Creating,
    Failed,
}

#[derive(Debug, Clone)]
pub struct TextInput {
    pub value: String,
    pub cursor: usize,
    pub secret: bool,
}

impl TextInput {
    pub fn new(secret: bool) -> Self {
        Self {
            value: String::new(),
            cursor: 0,
            secret,
        }
    }

    pub fn with_value(value: &str, secret: bool) -> Self {
        Self {
            value: value.to_owned(),
            cursor: value.chars().count(),
            secret,
        }
    }

    pub fn handle(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return false;
        }
        let mut chars: Vec<char> = self.value.chars().collect();
        match key.code {
            KeyCode::Char(character) => {
                chars.insert(self.cursor.min(chars.len()), character);
                self.cursor += 1;
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    chars.remove(self.cursor);
                }
            }
            KeyCode::Delete => {
                if self.cursor < chars.len() {
                    chars.remove(self.cursor);
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(chars.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = chars.len(),
            _ => return false,
        }
        self.value = chars.into_iter().collect();
        true
    }

    pub fn insert_text(&mut self, text: &str) -> bool {
        let text: String = text
            .chars()
            .filter(|character| !character.is_control())
            .collect();
        if text.is_empty() {
            return false;
        }
        let mut chars: Vec<char> = self.value.chars().collect();
        let at = self.cursor.min(chars.len());
        chars.splice(at..at, text.chars());
        self.cursor = at + text.chars().count();
        self.value = chars.into_iter().collect();
        true
    }

    pub fn masked(&self) -> String {
        if self.secret {
            "•".repeat(self.value.chars().count())
        } else {
            self.value.clone()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.value.trim().is_empty()
    }
}

#[derive(Debug, Clone)]
pub enum Action {
    None,
    FetchModels,
    FetchModelInfo,
    Finish(Box<InitOptions>),
    Quit,
}

pub struct Onboarding {
    pub step: Step,
    pub error: Option<String>,
    pub focus: usize,
    pub token: TextInput,
    pub user_id: TextInput,
    pub provider: usize,
    pub base_url: TextInput,
    pub api_key: TextInput,
    pub models: Vec<String>,
    pub models_loading: bool,
    pub models_error: Option<String>,
    pub model_filter: TextInput,
    pub model_selected: usize,
    pub model_manual: TextInput,
    pub selected_model: Option<String>,
    pub model_limits: Option<hearth_model::ModelLimits>,
    pub model_limits_loading: bool,
    pub model_limits_error: Option<String>,
    pub thinking: usize,
    pub agent_name: TextInput,
    pub extras_focus: usize,
    pub image: bool,
    pub image_key: TextInput,
    pub image_model: TextInput,
    pub stt: bool,
    pub stt_key: TextInput,
    pub stt_model: TextInput,
    pub tts: bool,
    pub tts_key: TextInput,
    pub tts_voice: TextInput,
    pub tts_model: TextInput,
    pub daemon_start: usize,
    pub keep_running: bool,
    pub pending: Option<InitOptions>,
}

impl Default for Onboarding {
    fn default() -> Self {
        Self {
            step: Step::Welcome,
            error: None,
            focus: 0,
            token: TextInput::new(true),
            user_id: TextInput::new(false),
            provider: 0,
            base_url: TextInput::new(false),
            api_key: TextInput::new(true),
            models: Vec::new(),
            models_loading: false,
            models_error: None,
            model_filter: TextInput::new(false),
            model_selected: 0,
            model_manual: TextInput::new(false),
            selected_model: None,
            model_limits: None,
            model_limits_loading: false,
            model_limits_error: None,
            thinking: 0,
            agent_name: TextInput::with_value("Hearth", false),
            extras_focus: 0,
            image: false,
            image_key: TextInput::new(true),
            image_model: TextInput::with_value("openai/gpt-image-2.6-sunburst", false),
            stt: false,
            stt_key: TextInput::new(true),
            stt_model: TextInput::with_value("whisper-large-v3-turbo", false),
            tts: false,
            tts_key: TextInput::new(true),
            tts_voice: TextInput::new(false),
            tts_model: TextInput::with_value("eleven_v4", false),
            daemon_start: 0,
            keep_running: true,
            pending: None,
        }
    }
}

impl Onboarding {
    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
    }

    pub fn models_loaded(&mut self, result: Result<Vec<String>, String>) {
        self.models_loading = false;
        self.model_selected = 0;
        match result {
            Ok(models) => {
                self.models = models;
                self.models_error = None;
            }
            Err(error) => {
                self.models.clear();
                self.models_error = Some(error);
            }
        }
    }

    pub fn model_info_loaded(&mut self, result: Result<Option<hearth_model::ModelLimits>, String>) {
        self.model_limits_loading = false;
        match result {
            Ok(Some(limits)) => {
                self.model_limits = Some(limits);
                self.model_limits_error = None;
            }
            Ok(None) => {
                self.model_limits = None;
                self.model_limits_error = Some(
                    "models.dev has no context window for this provider and model.".to_owned(),
                );
            }
            Err(error) => {
                self.model_limits = None;
                self.model_limits_error = Some(error);
            }
        }
    }

    pub fn filtered_models(&self) -> Vec<&String> {
        let filter = self.model_filter.value.to_ascii_lowercase();
        self.models
            .iter()
            .filter(|model| filter.is_empty() || model.to_ascii_lowercase().contains(&filter))
            .collect()
    }

    pub fn start_label(&self) -> &'static str {
        match self.daemon_start {
            1 => "At login",
            2 => "At startup",
            _ => "When Hearth opens",
        }
    }

    pub fn handle_paste(&mut self, text: &str) -> bool {
        let changed = self
            .active_input_mut()
            .is_some_and(|input| input.insert_text(text));
        if changed {
            self.error = None;
        }
        changed
    }

    fn active_input_mut(&mut self) -> Option<&mut TextInput> {
        match self.step {
            Step::Telegram => Some(if self.focus == 0 {
                &mut self.token
            } else {
                &mut self.user_id
            }),
            Step::Credentials if PROVIDERS[self.provider].base_url.is_none() && self.focus == 0 => {
                Some(&mut self.base_url)
            }
            Step::Credentials => Some(&mut self.api_key),
            Step::Model if self.models_loading => None,
            Step::Model if self.models.is_empty() => Some(&mut self.model_manual),
            Step::Model => Some(&mut self.model_filter),
            Step::Agent => Some(&mut self.agent_name),
            Step::Extras => self.extras_input_mut(),
            _ => None,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        match self.step {
            Step::Welcome => self.welcome(key),
            Step::Telegram => self.telegram(key),
            Step::Provider => self.provider(key),
            Step::Credentials => self.credentials(key),
            Step::Model => self.model(key),
            Step::Thinking => self.thinking(key),
            Step::Agent => self.agent(key),
            Step::Extras => self.extras(key),
            Step::Daemon => self.daemon(key),
            Step::Review => self.review(key),
            Step::Creating => Action::None,
            Step::Failed => self.failed(key),
        }
    }

    fn welcome(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Enter => {
                self.step = Step::Telegram;
                self.focus = 0;
                self.error = None;
            }
            KeyCode::Esc => return Action::Quit,
            _ => {}
        }
        Action::None
    }

    fn telegram(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Tab | KeyCode::Down | KeyCode::Up | KeyCode::BackTab => {
                self.focus = (self.focus + 1) % 2;
            }
            KeyCode::Enter => {
                let id = self.user_id.value.trim();
                if self.token.is_empty() {
                    self.set_error("The bot token is required.");
                } else if id.parse::<i64>().is_err() {
                    self.set_error("The Telegram user id must be a number.");
                } else {
                    self.error = None;
                    self.step = Step::Provider;
                    self.focus = 0;
                }
            }
            KeyCode::Esc => {
                self.step = Step::Welcome;
            }
            _ => {
                let input = if self.focus == 0 {
                    &mut self.token
                } else {
                    &mut self.user_id
                };
                if input.handle(key) {
                    self.error = None;
                }
            }
        }
        Action::None
    }

    fn provider(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Up => self.provider = self.provider.saturating_sub(1),
            KeyCode::Down => self.provider = (self.provider + 1).min(PROVIDERS.len() - 1),
            KeyCode::Enter => {
                let preset = &PROVIDERS[self.provider];
                self.base_url = preset
                    .base_url
                    .map(|url| TextInput::with_value(url, false))
                    .unwrap_or_else(|| TextInput::with_value("", false));
                self.error = None;
                self.step = Step::Credentials;
                self.focus = if preset.base_url.is_some() { 1 } else { 0 };
            }
            KeyCode::Esc => {
                self.step = Step::Telegram;
                self.focus = 0;
            }
            _ => {}
        }
        Action::None
    }

    fn credentials(&mut self, key: KeyEvent) -> Action {
        let other = PROVIDERS[self.provider].base_url.is_none();
        match key.code {
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Down | KeyCode::Up => {
                if other {
                    self.focus = (self.focus + 1) % 2;
                } else {
                    self.focus = 1;
                }
            }
            KeyCode::Enter => {
                let url = self.base_url.value.trim();
                if !(url.starts_with("https://") || url.starts_with("http://")) {
                    self.set_error("The provider URL must start with http:// or https://.");
                } else if self.api_key.is_empty() {
                    self.set_error("The provider API key is required.");
                } else {
                    self.error = None;
                    self.step = Step::Model;
                    self.models_loading = true;
                    self.models.clear();
                    self.models_error = None;
                    self.model_filter = TextInput::new(false);
                    self.model_selected = 0;
                    return Action::FetchModels;
                }
            }
            KeyCode::Esc => {
                self.step = Step::Provider;
                self.focus = 0;
            }
            _ => {
                let input = if other && self.focus == 0 {
                    &mut self.base_url
                } else {
                    &mut self.api_key
                };
                if input.handle(key) {
                    self.error = None;
                }
            }
        }
        Action::None
    }

    fn model(&mut self, key: KeyEvent) -> Action {
        if self.models_loading {
            if key.code == KeyCode::Esc {
                self.models_loading = false;
                self.step = Step::Credentials;
            }
            return Action::None;
        }
        if !self.models.is_empty() {
            match key.code {
                KeyCode::Up => self.model_selected = self.model_selected.saturating_sub(1),
                KeyCode::Down => {
                    let len = self.filtered_models().len();
                    if len > 0 {
                        self.model_selected = (self.model_selected + 1).min(len - 1);
                    }
                }
                KeyCode::Enter => {
                    let filtered = self.filtered_models();
                    if let Some(model) = filtered.get(self.model_selected) {
                        self.selected_model = Some((*model).clone());
                        self.error = None;
                        self.step = Step::Thinking;
                        self.focus = 0;
                        self.model_limits = None;
                        self.model_limits_error = None;
                        self.model_limits_loading = true;
                        return Action::FetchModelInfo;
                    }
                }
                KeyCode::Esc => {
                    self.step = Step::Credentials;
                    self.focus = 1;
                }
                _ => {
                    if self.model_filter.handle(key) {
                        self.model_selected = 0;
                    }
                }
            }
        } else {
            match key.code {
                KeyCode::Enter => {
                    if self.model_manual.is_empty() {
                        self.set_error("Enter a model name.");
                    } else {
                        self.selected_model = Some(self.model_manual.value.trim().to_owned());
                        self.error = None;
                        self.step = Step::Thinking;
                        self.focus = 0;
                        self.model_limits = None;
                        self.model_limits_error = None;
                        self.model_limits_loading = true;
                        return Action::FetchModelInfo;
                    }
                }
                KeyCode::Esc => {
                    self.step = Step::Credentials;
                    self.focus = 1;
                }
                _ => {
                    if self.model_manual.handle(key) {
                        self.error = None;
                    }
                }
            }
        }
        Action::None
    }

    fn thinking(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Up => self.thinking = self.thinking.saturating_sub(1),
            KeyCode::Down => self.thinking = (self.thinking + 1).min(THINKING.len() - 1),
            KeyCode::Enter => {
                self.step = Step::Agent;
                self.focus = 0;
                self.error = None;
            }
            KeyCode::Esc => {
                self.step = Step::Model;
            }
            _ => {}
        }
        Action::None
    }

    fn agent(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Enter => {
                if self.agent_name.is_empty() {
                    self.set_error("Give Hearth a name.");
                } else {
                    self.error = None;
                    self.step = Step::Extras;
                    self.extras_focus = 0;
                }
            }
            KeyCode::Esc => {
                self.step = Step::Thinking;
            }
            _ => {
                if self.agent_name.handle(key) {
                    self.error = None;
                }
            }
        }
        Action::None
    }

    fn extras(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Up => self.step_extras(-1),
            KeyCode::Down | KeyCode::Tab => self.step_extras(1),
            KeyCode::Enter => match self.extras_focus {
                0 => self.image = !self.image,
                3 => self.stt = !self.stt,
                6 => self.tts = !self.tts,
                10 => {
                    if let Err(message) = self.validate_extras() {
                        self.set_error(message);
                    } else {
                        self.error = None;
                        self.step = Step::Daemon;
                        self.focus = 0;
                    }
                }
                _ => self.step_extras(1),
            },
            KeyCode::Esc => {
                self.step = Step::Agent;
            }
            _ => {
                if let Some(input) = self.extras_input_mut()
                    && input.handle(key)
                {
                    self.error = None;
                }
            }
        }
        Action::None
    }

    fn step_extras(&mut self, delta: isize) {
        const LAST: usize = 10;
        let mut index = self.extras_focus as isize;
        for _ in 0..=LAST {
            index += delta;
            if !(0..=LAST as isize).contains(&index) {
                return;
            }
            if self.extras_focus_enabled(index as usize) {
                self.extras_focus = index as usize;
                return;
            }
        }
    }

    fn extras_focus_enabled(&self, focus: usize) -> bool {
        match focus {
            1..=2 => self.image,
            4..=5 => self.stt,
            7..=9 => self.tts,
            _ => true,
        }
    }

    fn extras_input_mut(&mut self) -> Option<&mut TextInput> {
        match self.extras_focus {
            1 if self.image => Some(&mut self.image_key),
            2 if self.image => Some(&mut self.image_model),
            4 if self.stt => Some(&mut self.stt_key),
            5 if self.stt => Some(&mut self.stt_model),
            7 if self.tts => Some(&mut self.tts_key),
            8 if self.tts => Some(&mut self.tts_voice),
            9 if self.tts => Some(&mut self.tts_model),
            _ => None,
        }
    }

    pub fn extras_input(&self) -> Option<&TextInput> {
        match self.extras_focus {
            1 if self.image => Some(&self.image_key),
            2 if self.image => Some(&self.image_model),
            4 if self.stt => Some(&self.stt_key),
            5 if self.stt => Some(&self.stt_model),
            7 if self.tts => Some(&self.tts_key),
            8 if self.tts => Some(&self.tts_voice),
            9 if self.tts => Some(&self.tts_model),
            _ => None,
        }
    }

    fn validate_extras(&self) -> Result<(), &'static str> {
        if self.image {
            if self.image_key.is_empty() {
                return Err("The OpenRouter key is required for images.");
            }
            if self.image_model.is_empty() {
                return Err("The image model is required.");
            }
        }
        if self.stt {
            if self.stt_key.is_empty() {
                return Err("The Groq key is required for transcription.");
            }
            if self.stt_model.is_empty() {
                return Err("The transcription model is required.");
            }
        }
        if self.tts {
            if self.tts_key.is_empty() {
                return Err("The ElevenLabs key is required for speech.");
            }
            if self.tts_voice.is_empty() {
                return Err("The ElevenLabs voice id is required.");
            }
            if self.tts_model.is_empty() {
                return Err("The speech model is required.");
            }
        }
        Ok(())
    }

    fn daemon(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Up => self.focus = self.focus.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => self.focus = (self.focus + 1).min(2),
            KeyCode::Enter => match self.focus {
                0 => self.daemon_start = (self.daemon_start + 1) % 3,
                1 => self.keep_running = !self.keep_running,
                _ => {
                    self.step = Step::Review;
                    self.error = None;
                }
            },
            KeyCode::Esc => {
                self.step = Step::Extras;
                self.extras_focus = 10;
            }
            _ => {}
        }
        Action::None
    }

    fn review(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Enter => match self.build_options() {
                Ok(options) => {
                    self.pending = Some(options.clone());
                    self.step = Step::Creating;
                    self.error = None;
                    Action::Finish(Box::new(options))
                }
                Err(message) => {
                    self.set_error(message);
                    Action::None
                }
            },
            KeyCode::Esc => {
                self.step = Step::Daemon;
                self.focus = 2;
                Action::None
            }
            _ => Action::None,
        }
    }

    fn failed(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Enter => {
                if let Some(options) = self.pending.clone() {
                    self.step = Step::Creating;
                    self.error = None;
                    Action::Finish(Box::new(options))
                } else {
                    Action::None
                }
            }
            KeyCode::Esc => {
                self.step = Step::Review;
                Action::None
            }
            _ => Action::None,
        }
    }

    pub fn build_options(&self) -> Result<InitOptions, String> {
        let user_id = self
            .user_id
            .value
            .trim()
            .parse::<i64>()
            .map_err(|_| "The Telegram user id must be a number.".to_owned())?;
        let model = self
            .selected_model
            .clone()
            .or_else(|| {
                let filtered = self.filtered_models();
                filtered
                    .get(self.model_selected)
                    .map(|model| (*model).clone())
            })
            .filter(|model| !model.is_empty())
            .ok_or_else(|| "Choose a model.".to_owned())?;
        let media = self.media_json();
        let context_window = self
            .model_limits
            .map(|limits| limits.context_window)
            .filter(|window| *window > 0);
        Ok(InitOptions {
            telegram_token: self.token.value.trim().to_owned(),
            allowed_user_ids: vec![user_id],
            base_url: self.base_url.value.trim().to_owned(),
            api_key: self.api_key.value.trim().to_owned(),
            model,
            thinking: THINKING[self.thinking].0.to_owned(),
            agent_name: self.agent_name.value.trim().to_owned(),
            timezone: None,
            media,
            context_window,
            daemon: Daemon {
                start: ["tui", "login", "startup"][self.daemon_start].to_owned(),
                keep_running: self.keep_running,
            },
        })
    }

    fn media_json(&self) -> Option<serde_json::Value> {
        let mut media = serde_json::Map::new();
        if self.image {
            media.insert(
                "image".to_owned(),
                json!({
                    "kind": "openrouter-images",
                    "api_key": self.image_key.value.trim(),
                    "model": self.image_model.value.trim(),
                }),
            );
        }
        if self.stt {
            media.insert(
                "stt".to_owned(),
                json!({
                    "kind": "groq",
                    "api_key": self.stt_key.value.trim(),
                    "model": self.stt_model.value.trim(),
                }),
            );
        }
        if self.tts {
            media.insert(
                "tts".to_owned(),
                json!({
                    "kind": "elevenlabs",
                    "api_key": self.tts_key.value.trim(),
                    "voice_id": self.tts_voice.value.trim(),
                    "model": self.tts_model.value.trim(),
                    "output_format": "opus_48000_64",
                }),
            );
        }
        if media.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(media))
        }
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn build_options_collects_the_wizard() {
        let mut onboarding = Onboarding::default();
        onboarding.token.value = "123:fake".to_owned();
        onboarding.user_id.value = "42".to_owned();
        onboarding.base_url.value = "https://example.com/v1".to_owned();
        onboarding.api_key.value = "fake-key".to_owned();
        onboarding.selected_model = Some("fake-model".to_owned());
        onboarding.thinking = 3;
        onboarding.agent_name.value = "Hearth".to_owned();
        onboarding.image = true;
        onboarding.image_key.value = "image-key".to_owned();
        onboarding.stt = true;
        onboarding.stt_key.value = "stt-key".to_owned();
        onboarding.tts = true;
        onboarding.tts_key.value = "tts-key".to_owned();
        onboarding.tts_voice.value = "voice-1".to_owned();
        onboarding.daemon_start = 1;
        onboarding.keep_running = false;

        let options = onboarding.build_options().expect("options");
        assert_eq!(options.telegram_token, "123:fake");
        assert_eq!(options.allowed_user_ids, vec![42]);
        assert_eq!(options.base_url, "https://example.com/v1");
        assert_eq!(options.api_key, "fake-key");
        assert_eq!(options.model, "fake-model");
        assert_eq!(options.thinking, "medium");
        assert_eq!(options.daemon.start, "login");
        assert!(!options.daemon.keep_running);
        let media = options.media.expect("media");
        assert_eq!(media["image"]["kind"], "openrouter-images");
        assert_eq!(media["stt"]["kind"], "groq");
        assert_eq!(media["tts"]["kind"], "elevenlabs");
        assert_eq!(media["tts"]["voice_id"], "voice-1");
    }

    #[test]
    fn model_limits_fill_the_init_options() {
        let mut onboarding = Onboarding::default();
        onboarding.token.value = "123:fake".to_owned();
        onboarding.user_id.value = "42".to_owned();
        onboarding.base_url.value = "https://example.com/v1".to_owned();
        onboarding.api_key.value = "fake-key".to_owned();
        onboarding.selected_model = Some("fake-model".to_owned());
        onboarding.model_limits = Some(hearth_model::ModelLimits {
            context_window: 1_000_000,
            max_output_tokens: 384_000,
        });
        let options = onboarding.build_options().expect("options");
        assert_eq!(options.context_window, Some(1_000_000));
    }

    #[test]
    fn telegram_rejects_a_non_numeric_id() {
        let mut onboarding = Onboarding::default();
        onboarding.step = Step::Telegram;
        onboarding.token.value = "123:fake".to_owned();
        onboarding.user_id.value = "not-a-number".to_owned();
        let action = onboarding.handle_key(key(KeyCode::Enter));
        assert!(matches!(action, Action::None));
        assert_eq!(onboarding.step, Step::Telegram);
        assert!(
            onboarding
                .error
                .as_deref()
                .is_some_and(|error| error.contains("number"))
        );
    }

    #[test]
    fn model_filter_narrows_the_list() {
        let mut onboarding = Onboarding::default();
        onboarding.models = vec!["alpha".to_owned(), "beta".to_owned()];
        onboarding.model_filter.value = "be".to_owned();
        let filtered = onboarding.filtered_models();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0], "beta");
    }

    #[test]
    fn extras_require_a_key_when_enabled() {
        let mut onboarding = Onboarding::default();
        onboarding.step = Step::Extras;
        onboarding.extras_focus = 10;
        onboarding.image = true;
        let action = onboarding.handle_key(key(KeyCode::Enter));
        assert!(matches!(action, Action::None));
        assert_eq!(onboarding.step, Step::Extras);
        assert!(
            onboarding
                .error
                .as_deref()
                .is_some_and(|error| error.contains("OpenRouter"))
        );

        onboarding.image_key.value = "image-key".to_owned();
        let _ = onboarding.handle_key(key(KeyCode::Enter));
        assert_eq!(onboarding.step, Step::Daemon);
    }

    #[test]
    fn extras_navigation_skips_disabled_services() {
        let mut onboarding = Onboarding::default();
        onboarding.step = Step::Extras;
        onboarding.extras_focus = 0;
        for expected in [3, 6, 10] {
            let _ = onboarding.handle_key(key(KeyCode::Down));
            assert_eq!(onboarding.extras_focus, expected);
        }
        let _ = onboarding.handle_key(key(KeyCode::Up));
        assert_eq!(onboarding.extras_focus, 6);

        onboarding.image = true;
        onboarding.extras_focus = 0;
        let _ = onboarding.handle_key(key(KeyCode::Down));
        assert_eq!(onboarding.extras_focus, 1);

        onboarding.stt = true;
        onboarding.extras_focus = 3;
        let _ = onboarding.handle_key(key(KeyCode::Down));
        assert_eq!(onboarding.extras_focus, 4);

        onboarding.tts = true;
        onboarding.extras_focus = 6;
        let _ = onboarding.handle_key(key(KeyCode::Down));
        assert_eq!(onboarding.extras_focus, 7);
    }

    #[test]
    fn paste_inserts_at_the_cursor_and_strips_newlines() {
        let mut onboarding = Onboarding::default();
        onboarding.step = Step::Telegram;
        onboarding.token.value = "12:ab".to_owned();
        onboarding.token.cursor = 3;
        assert!(onboarding.handle_paste("cd\nef"));
        assert_eq!(onboarding.token.value, "12:cdefab");
        assert_eq!(onboarding.token.cursor, 7);
    }

    #[test]
    fn welcome_quits_on_escape() {
        let mut onboarding = Onboarding::default();
        let action = onboarding.handle_key(key(KeyCode::Esc));
        assert!(matches!(action, Action::Quit));
    }
}
