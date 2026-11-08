use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use hearth_config::{ConfigError, Media, Settings};
use hearth_core::{CoreError, Harness, Session, Submission};
use hearth_model::{
    MediaError, OpenAiCompatibleProvider, Pricing, Provider, ProviderError, Speaker, SpeechClient,
    Transcriber, TranscriptionClient,
};
use hearth_telegram::{
    InboundMedia, SendOutcome, SystemSleeper, TelegramClient, TelegramError, TelegramPoller,
    TelegramSender,
};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use thiserror::Error;

use crate::agent::{Agent, AgentConfig, AgentError, RunOutcome};
use crate::effect_worker::EffectWorker;
use crate::tools::{MessageTarget, build_tools};

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Media(#[from] MediaError),
    #[error(transparent)]
    Telegram(#[from] TelegramError),
    #[error("another hearth daemon is already running (pid {0})")]
    AlreadyRunning(u32),
    #[error("another hearth daemon is already running")]
    AlreadyRunningUnknown,
    #[error("daemon file operation failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Acquire the single-writer lock, recover interrupted work, and wait.
pub fn daemon(home: &Path) -> Result<(), DaemonError> {
    let settings = Arc::new(hearth_config::load(home)?);
    settings.validate_for_run()?;

    let lock_path = home.join("hearth.lock");
    let mut lock = open_lock(&lock_path)?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            return Err(match read_pid(&lock_path) {
                Some(pid) => DaemonError::AlreadyRunning(pid),
                None => DaemonError::AlreadyRunningUnknown,
            });
        }
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    lock.set_len(0)?;
    writeln!(lock, "{}", std::process::id())?;
    lock.flush()?;

    let harness = Harness::open(home)?;
    let session = harness.session().clone();
    let recovery = session.recover_tasks()?;
    tracing::info!(
        requeued = recovery.requeued,
        poisoned = recovery.poisoned,
        woken = recovery.woken,
        "task recovery complete"
    );

    let workspace = match settings.agent.workspace.clone() {
        Some(path) if path.is_absolute() => path,
        Some(path) => home.join(path),
        None => home.join("workspace"),
    };
    let spill = home.join("spill");

    let effect_worker = EffectWorker::new(session.clone(), workspace.clone());
    let recovered = effect_worker.recover_started()?;
    tracing::info!(recovered, "effect recovery complete");

    let shutdown = Arc::new(AtomicBool::new(false));
    let effect_shutdown = Arc::clone(&shutdown);
    let effect_handle = thread::spawn(move || {
        while !effect_shutdown.load(Ordering::Relaxed) {
            if let Err(error) = effect_worker.run_once() {
                tracing::error!(%error, "effect worker iteration failed");
            }
            thread::sleep(Duration::from_millis(100));
        }
    });
    tracing::info!("effect worker started");

    let mut telegram_handles = Vec::new();
    if settings
        .telegram
        .as_ref()
        .and_then(|telegram| telegram.bot_token.as_ref())
        .is_some()
    {
        let runtime = TelegramRuntime::build(
            home,
            Arc::clone(&settings),
            session.clone(),
            workspace,
            spill,
        )?;
        let poller_shutdown = Arc::clone(&shutdown);
        let poller = runtime.poller;
        telegram_handles.push(thread::spawn(move || run_poller(poller, poller_shutdown)));
        let worker_shutdown = Arc::clone(&shutdown);
        let worker = runtime.worker;
        telegram_handles.push(thread::spawn(move || worker.run(worker_shutdown)));
        tracing::info!("telegram poller and worker started");
    }

    tracing::info!(
        pid = std::process::id(),
        home = %home.display(),
        "hearth daemon started"
    );
    let mut signals = Signals::new([SIGTERM, SIGINT])?;
    if let Some(signal) = signals.forever().next() {
        tracing::info!(signal, "hearth daemon stopping");
    }
    shutdown.store(true, Ordering::Relaxed);
    for handle in telegram_handles {
        if handle.join().is_err() {
            tracing::error!("telegram thread panicked");
        }
    }
    if effect_handle.join().is_err() {
        tracing::error!("effect worker panicked");
    }
    drop(harness);
    Ok(())
}

struct TelegramRuntime {
    poller: TelegramPoller,
    worker: TelegramWorker,
}

impl TelegramRuntime {
    fn build(
        home: &Path,
        settings: Arc<Settings>,
        session: Session,
        workspace: PathBuf,
        spill: PathBuf,
    ) -> Result<Self, DaemonError> {
        let telegram = settings
            .telegram
            .as_ref()
            .ok_or_else(|| invalid("telegram is not configured"))?;
        let token = telegram
            .bot_token
            .clone()
            .ok_or_else(|| invalid("telegram.bot_token is required"))?;
        let base_url = telegram
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.telegram.org".to_owned());
        let client = TelegramClient::with_base_url(token, base_url)?;

        let models = settings
            .models
            .as_ref()
            .ok_or_else(|| invalid("models is required for telegram"))?;
        let provider = OpenAiCompatibleProvider::new(
            models.base_url.clone(),
            models.api_key.clone(),
            models
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            models.session_header.clone(),
        )?;
        let provider: Arc<dyn Provider> = Arc::new(provider);

        let soul_path = match &settings.identity.soul_file {
            Some(path) if path.is_absolute() => path.clone(),
            Some(path) => home.join(path),
            None => home.join("SOUL.md"),
        };
        let soul = std::fs::read_to_string(&soul_path).unwrap_or_else(|_| {
            hearth_core::default_soul(&settings.identity.agent_name, &settings.identity.user_name)
        });

        let media = settings.media.clone().unwrap_or_default();
        let transcriber = build_transcriber(&media)?;
        let speaker = build_speaker(&media)?;
        let poller = TelegramPoller::new(
            client.clone(),
            session.clone(),
            telegram.allowed_user_ids.clone(),
            telegram.allowed_chat_ids.clone(),
            30,
            InboundMedia::new(home.join("media/inbound")),
            transcriber,
        );
        let sender = TelegramSender::new(client.clone(), Arc::new(SystemSleeper));
        let worker = TelegramWorker {
            settings: Arc::clone(&settings),
            session,
            provider,
            client,
            sender,
            speaker,
            soul,
            workspace,
            spill,
            outbound: home.join("media/outbound"),
            model: models.model.clone(),
            provider_name: models.kind.clone(),
            max_turns: settings.agent.max_turns,
            pid: std::process::id(),
        };
        Ok(Self { poller, worker })
    }
}

struct TelegramWorker {
    settings: Arc<Settings>,
    session: Session,
    provider: Arc<dyn Provider>,
    client: TelegramClient,
    sender: TelegramSender,
    speaker: Option<Arc<dyn Speaker>>,
    soul: String,
    workspace: PathBuf,
    spill: PathBuf,
    outbound: PathBuf,
    model: String,
    provider_name: String,
    max_turns: u32,
    pid: u32,
}

impl TelegramWorker {
    fn run(&self, shutdown: Arc<AtomicBool>) {
        while !shutdown.load(Ordering::Relaxed) {
            match self.session.next_queued_telegram_submission() {
                Ok(Some(submission)) => self.run_one(submission),
                Ok(None) => thread::sleep(Duration::from_millis(100)),
                Err(error) => {
                    tracing::error!(%error, "queued submission lookup failed");
                    thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }

    fn run_one(&self, submission: Submission) {
        let conversation = match self.session.conversation(&submission.conversation_id) {
            Ok(Some(conversation)) => conversation,
            Ok(None) => {
                tracing::warn!(
                    submission = %submission.id,
                    "telegram conversation is missing"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "telegram conversation lookup failed");
                return;
            }
        };
        let Some((chat_id, thread_id)) = parse_target(&conversation.transport_key) else {
            tracing::warn!(
                key = %conversation.transport_key,
                "telegram transport key is invalid"
            );
            return;
        };

        let target = MessageTarget {
            client: self.client.clone(),
            chat_id,
            thread_id,
            outbound: self.outbound.clone(),
            speaker: self.speaker.clone(),
        };
        let tools = match build_tools(&self.settings, &self.workspace, &self.spill, Some(target)) {
            Ok(tools) => tools,
            Err(error) => {
                tracing::warn!(%error, "telegram tools could not be built");
                return;
            }
        };
        let agent = Agent::new(
            self.session.clone(),
            Arc::clone(&self.provider),
            Pricing::default(),
            AgentConfig {
                soul: self.soul.clone(),
                model: self.model.clone(),
                provider_name: self.provider_name.clone(),
                owner: format!("daemon:{}:{}", self.pid, submission.id),
                max_turns: self.max_turns,
                tools,
                spill_dir: self.spill.clone(),
            },
        );

        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let typing_sender = self.sender.clone();
        let typing = thread::spawn(move || {
            loop {
                if typing_sender.send_typing(chat_id, thread_id).is_err() {
                    break;
                }
                match stop_rx.recv_timeout(Duration::from_secs(4)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        });
        let outcome = agent.run_submission(&submission.id);
        let _ = stop_tx.send(());
        let _ = typing.join();

        match outcome {
            Ok(outcome) => self.deliver(chat_id, thread_id, &outcome),
            Err(AgentError::Busy) => {
                tracing::debug!(submission = %submission.id, "run is busy");
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                tracing::warn!(
                    submission = %submission.id,
                    %error,
                    "telegram run failed"
                );
                thread::sleep(Duration::from_secs(1));
            }
        }
    }

    fn deliver(&self, chat_id: i64, thread_id: Option<i64>, outcome: &RunOutcome) {
        let text = outcome.answer_text.as_deref().or(outcome.notice.as_deref());
        let Some(text) = text else {
            return;
        };
        match self.sender.send_reply(chat_id, thread_id, text) {
            Ok(SendOutcome::Rich) => tracing::info!(chat_id, "telegram reply sent"),
            Ok(SendOutcome::Fallback { parts }) => {
                tracing::info!(chat_id, parts, "telegram fallback reply sent");
            }
            Ok(SendOutcome::Uncertain { message }) => {
                tracing::warn!(chat_id, %message, "telegram delivery is uncertain");
            }
            Err(error) => {
                tracing::warn!(chat_id, %error, "telegram reply failed");
            }
        }
    }
}

fn run_poller(poller: TelegramPoller, shutdown: Arc<AtomicBool>) {
    let mut backoff = Duration::from_secs(1);
    while !shutdown.load(Ordering::Relaxed) {
        match poller.poll_once() {
            Ok(report) => {
                backoff = Duration::from_secs(1);
                if report.admitted > 0 || report.duplicates > 0 || report.ignored > 0 {
                    tracing::info!(
                        admitted = report.admitted,
                        duplicates = report.duplicates,
                        ignored = report.ignored,
                        "telegram poll complete"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(%error, "telegram poll failed");
                thread::sleep(backoff);
                backoff = std::cmp::min(backoff * 2, Duration::from_secs(30));
            }
        }
    }
}

fn build_transcriber(media: &Media) -> Result<Option<Arc<dyn Transcriber>>, DaemonError> {
    let Some(stt) = &media.stt else {
        return Ok(None);
    };
    let api_key = stt
        .api_key
        .clone()
        .ok_or_else(|| invalid("media.stt.api_key is required"))?;
    let model = stt
        .model
        .clone()
        .ok_or_else(|| invalid("media.stt.model is required"))?;
    let base_url = stt
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.groq.com".to_owned());
    Ok(Some(Arc::new(TranscriptionClient::new(
        base_url, api_key, model,
    )?)))
}

fn build_speaker(media: &Media) -> Result<Option<Arc<dyn Speaker>>, DaemonError> {
    let Some(tts) = &media.tts else {
        return Ok(None);
    };
    let api_key = tts
        .api_key
        .clone()
        .ok_or_else(|| invalid("media.tts.api_key is required"))?;
    let voice_id = tts
        .voice_id
        .clone()
        .ok_or_else(|| invalid("media.tts.voice_id is required"))?;
    let model = tts.model.clone().unwrap_or_else(|| "eleven_v4".to_owned());
    let output_format = tts
        .output_format
        .clone()
        .unwrap_or_else(|| "opus_48000_64".to_owned());
    let base_url = tts
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.elevenlabs.io".to_owned());
    Ok(Some(Arc::new(SpeechClient::new(
        base_url,
        api_key,
        voice_id,
        model,
        output_format,
    )?)))
}

fn parse_target(key: &str) -> Option<(i64, Option<i64>)> {
    let (chat_id, thread_id) = key.split_once(':')?;
    let chat_id = chat_id.parse::<i64>().ok()?;
    let thread_id = thread_id.parse::<i64>().ok()?;
    Some((chat_id, (thread_id != 0).then_some(thread_id)))
}

fn invalid(message: &str) -> DaemonError {
    DaemonError::Config(ConfigError::Invalid(message.to_owned()))
}

fn open_lock(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn read_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}
