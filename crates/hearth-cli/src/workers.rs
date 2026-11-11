//! Background jobs and the events they send back to the app.

use std::io::BufRead;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use hearth_runtime::InitOptions;
use serde_json::Value;

use crate::daemon_client::{self, Ownership};

#[derive(Debug)]
pub enum OnboardResult {
    Ready(Ownership, Value),
    CreatedButDaemonFailed(String),
    InitFailed(String),
}

#[derive(Debug, Clone)]
pub enum DaemonAction {
    Start,
    Stop,
    Restart,
}

#[derive(Debug)]
pub enum AppEvent {
    Status(Result<Value, String>),
    Live(Value),
    Models(Result<Vec<String>, String>),
    Doctor(Result<String, String>),
    Onboarded(OnboardResult),
    Service(Result<(Ownership, Value), String>),
    Daemon(Result<(Ownership, Value), String>),
    Stopped(Result<(), String>),
    Prompt(Result<Option<Value>, String>),
    ModelInfo {
        model: String,
        result: Result<Option<hearth_model::ModelLimits>, String>,
    },
}

pub struct Workers {
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
    stop: Arc<AtomicBool>,
}

impl Default for Workers {
    fn default() -> Self {
        Self::new()
    }
}

impl Workers {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn sender(&self) -> Sender<AppEvent> {
        self.tx.clone()
    }

    pub fn try_recv(&self) -> Option<AppEvent> {
        self.rx.try_recv().ok()
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn spawn_status(&self, home: PathBuf) {
        let tx = self.sender();
        let stop = Arc::clone(&self.stop);
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let status = daemon_client::probe(&home).map_err(|error| error.to_string());
                if tx.send(AppEvent::Status(status)).is_err() {
                    return;
                }
                for _ in 0..20 {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        });
    }

    pub fn spawn_watch(&self, home: PathBuf) {
        let tx = self.sender();
        let stop = Arc::clone(&self.stop);
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let reader = match hearth_runtime::open_watch(&home, None) {
                    Ok(reader) => reader,
                    Err(_) => {
                        thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                };
                for line in reader.lines() {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let Ok(line) = line else {
                        break;
                    };
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok(value) = serde_json::from_str::<Value>(&line)
                        && tx.send(AppEvent::Live(value)).is_err()
                    {
                        return;
                    }
                }
                thread::sleep(Duration::from_secs(1));
            }
        });
    }

    pub fn spawn_models(&self, base_url: String, api_key: String) {
        let tx = self.sender();
        thread::spawn(move || {
            let result =
                hearth_model::list_models(&base_url, &api_key).map_err(|error| error.to_string());
            let _ = tx.send(AppEvent::Models(result));
        });
    }

    pub fn spawn_model_info(&self, base_url: String, model: String) {
        let tx = self.sender();
        thread::spawn(move || {
            let result = hearth_model::lookup_model_info(&base_url, &model)
                .map_err(|error| error.to_string());
            let _ = tx.send(AppEvent::ModelInfo { model, result });
        });
    }

    pub fn spawn_doctor(&self, home: PathBuf, live: bool) {
        let tx = self.sender();
        thread::spawn(move || {
            let mut output = Vec::new();
            let result = hearth_runtime::doctor_with(&home, live, &mut output)
                .map(|()| String::from_utf8_lossy(&output).into_owned())
                .map_err(|error| error.to_string());
            let _ = tx.send(AppEvent::Doctor(result));
        });
    }

    pub fn spawn_onboard(&self, home: PathBuf, options: InitOptions) {
        let tx = self.sender();
        thread::spawn(move || {
            let daemon = options.daemon.clone();
            let catalog = options.context_window.map(|context_window| {
                (
                    options.base_url.clone(),
                    options.model.clone(),
                    hearth_model::ModelLimits {
                        context_window,
                        max_output_tokens: 0,
                    },
                )
            });
            match hearth_runtime::init(&home, options) {
                Ok(_) => {}
                Err(error) => {
                    let _ = tx.send(AppEvent::Onboarded(OnboardResult::InitFailed(
                        error.to_string(),
                    )));
                    return;
                }
            }
            if let Some((base_url, model, limits)) = catalog {
                let _ = crate::catalog_cache::save(&home, &base_url, &model, limits);
            }
            if let Err(error) = daemon_client::apply_start_mode(&home, &daemon.start) {
                let _ = tx.send(AppEvent::Onboarded(OnboardResult::CreatedButDaemonFailed(
                    error,
                )));
                return;
            }
            match daemon_client::ensure(&home, &daemon.start) {
                Ok((ownership, status)) => {
                    let _ = tx.send(AppEvent::Onboarded(OnboardResult::Ready(ownership, status)));
                }
                Err(error) => {
                    let _ = tx.send(AppEvent::Onboarded(OnboardResult::CreatedButDaemonFailed(
                        error,
                    )));
                }
            }
        });
    }

    pub fn spawn_service(&self, home: PathBuf, start: String, ownership: Ownership) {
        let tx = self.sender();
        thread::spawn(move || {
            if !matches!(ownership, Ownership::Attached) {
                let _ = daemon_client::stop(&home, &ownership);
            }
            if let Err(error) = daemon_client::apply_start_mode(&home, &start) {
                let _ = tx.send(AppEvent::Service(Err(error)));
                return;
            }
            let result = daemon_client::ensure(&home, &start);
            let _ = tx.send(AppEvent::Service(result));
        });
    }

    pub fn spawn_daemon_action(
        &self,
        home: PathBuf,
        action: DaemonAction,
        ownership: Ownership,
        start: String,
    ) {
        let tx = self.sender();
        thread::spawn(move || match action {
            DaemonAction::Start => {
                let result = daemon_client::ensure(&home, &start);
                let _ = tx.send(AppEvent::Daemon(result));
            }
            DaemonAction::Stop => {
                let result = daemon_client::stop(&home, &ownership);
                let _ = tx.send(AppEvent::Stopped(result));
            }
            DaemonAction::Restart => {
                let result = daemon_client::restart(&home, &ownership, &start);
                let _ = tx.send(AppEvent::Daemon(result));
            }
        });
    }

    pub fn spawn_prompt(&self, home: PathBuf) {
        let tx = self.sender();
        thread::spawn(move || {
            let result =
                hearth_runtime::inspect::latest_prompt(&home).map_err(|error| error.to_string());
            let _ = tx.send(AppEvent::Prompt(result));
        });
    }
}
