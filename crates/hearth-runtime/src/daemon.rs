use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use hearth_config::{ConfigError, Media, Settings};
use hearth_core::{
    ContextBudget, CoreError, EffectIntent, EffectStatus, Harness, Save, Session, Submission,
    plan_compaction,
};
use hearth_model::{
    MediaError, OpenAiCompatibleProvider, Provider, ProviderError, Speaker, SpeechClient,
    Transcriber, TranscriptionClient,
};
use hearth_telegram::{
    InboundMedia, SendOutcome, StopHandler, SystemSleeper, TelegramClient, TelegramError,
    TelegramPoller, TelegramSender,
};
use serde_json::{Value, json};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use thiserror::Error;

use crate::agent::{Agent, AgentConfig, AgentError, RunOutcome};
use crate::compaction::Compactor;
use crate::effect_worker::EffectWorker;
use crate::events::EventBus;
use crate::helpers::{HelperConfig, HelperControls, HelperPool};
use crate::ipc::{self, StatusContext};
use crate::maintenance::{BACKUP_KIND, BackupHandler, SKILL_REVIEW_KIND, ensure_maintenance_tasks};
use crate::scheduler::{ReminderHandler, Scheduler, TaskRegistry};
use crate::skill_review::{SkillReviewHandler, SkillReviewer};
use crate::skills::SkillLibrary;
use crate::tools::{MessageTarget, TaskTarget, ToolMediaRoots, build_tools};
use hearth_memory::MemoryService;

const TELEGRAM_POLL_TIMEOUT_SECS: u32 = 2;

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
    #[error(transparent)]
    Ipc(#[from] ipc::IpcError),
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
    let started = Instant::now();
    let recovery = session.recover_tasks()?;
    if let Ok(pruned) = session.prune_helpers(
        settings.tasks.retention_days,
        settings.tasks.retention_runs,
        session.now_micros(),
    ) {
        tracing::info!(pruned, "helper retention complete");
    }
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
    // Memory is not optional: a daemon that cannot reach its store would answer
    // with silently missing context, so failing to open it stops startup.
    let memory = Arc::new(
        MemoryService::new(session.clone())
            .map_err(|error| std::io::Error::other(format!("memory could not be opened: {error}")))?
            .with_recall_budget(
                settings.memory.recall_max_items,
                settings.memory.recall_max_tokens,
            )
            .with_embeddings(settings.memory.embeddings),
    );
    // Repair an over-budget standing file at startup, as the store intends:
    // bullets trimmed there are archived to long-term memory first.
    match memory.standing_store().load() {
        Ok(snapshot) if !snapshot.archived.is_empty() => tracing::warn!(
            archived = snapshot.archived.len(),
            characters = snapshot.characters,
            "standing memory repaired"
        ),
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "standing memory could not be loaded"),
    }

    let effect_worker = EffectWorker::new(session.clone(), workspace.clone());
    let recovered = effect_worker.recover_started()?;
    tracing::info!(recovered, "effect recovery complete");

    let shutdown = Arc::new(AtomicBool::new(false));
    let ipc_shutdown = Arc::new(AtomicBool::new(false));
    let events = Arc::new(EventBus::new(collect_secrets(&settings)));
    let model = settings
        .models
        .as_ref()
        .map(|models| models.model.clone())
        .unwrap_or_default();
    let socket_handle = ipc::serve(
        home,
        StatusContext {
            session: session.clone(),
            model,
            started,
            memory: Arc::clone(&memory),
        },
        Arc::clone(&events),
        Arc::clone(&ipc_shutdown),
    )?;
    tracing::info!(socket = %ipc::socket_path(home).display(), "daemon socket ready");

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
            Arc::clone(&events),
            Arc::clone(&memory),
        )?;
        let poller_shutdown = Arc::clone(&shutdown);
        let poller = runtime.poller;
        telegram_handles.push(thread::spawn(move || run_poller(poller, poller_shutdown)));
        let worker_shutdown = Arc::clone(&shutdown);
        let worker = runtime.worker;
        telegram_handles.push(thread::spawn(move || worker.run(worker_shutdown)));
        let scheduler_shutdown = Arc::clone(&shutdown);
        let scheduler = runtime.scheduler;
        telegram_handles.push(thread::spawn(move || scheduler.run(scheduler_shutdown)));
        telegram_handles.extend(runtime.helpers.spawn_workers(Arc::clone(&shutdown)));
        tracing::info!("telegram poller, worker, and scheduler started");
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
    ipc_shutdown.store(true, Ordering::Relaxed);
    if socket_handle.join().is_err() {
        tracing::error!("daemon socket panicked");
    }
    drop(harness);
    Ok(())
}

struct TelegramRuntime {
    poller: TelegramPoller,
    worker: TelegramWorker,
    scheduler: Scheduler,
    helpers: HelperPool,
}

impl TelegramRuntime {
    fn build(
        home: &Path,
        settings: Arc<Settings>,
        session: Session,
        workspace: PathBuf,
        spill: PathBuf,
        events: Arc<EventBus>,
        memory: Arc<MemoryService>,
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
        )?
        .with_reasoning_effort(models.thinking.clone());
        let provider: Arc<dyn Provider> = Arc::new(provider);

        let soul_path = home.join("SOUL.md");
        let soul = std::fs::read_to_string(&soul_path)
            .unwrap_or_else(|_| hearth_core::default_soul(&settings.identity.agent_name));

        let media = settings.media.clone().unwrap_or_default();
        let transcriber = build_transcriber(&media)?;
        let speaker = build_speaker(&media)?;
        let cancellations = Arc::new(CancellationRegistry::default());
        let poller = TelegramPoller::new(
            client.clone(),
            session.clone(),
            telegram.allowed_user_ids.clone(),
            telegram.allowed_chat_ids.clone(),
            TELEGRAM_POLL_TIMEOUT_SECS,
            InboundMedia::new(home.join("media/inbound")),
            transcriber,
            Some(Arc::new(RegistryStop(Arc::clone(&cancellations)))),
        );
        let sender = TelegramSender::new(client.clone(), Arc::new(SystemSleeper));
        let helper_controls = Arc::new(HelperControls::new());
        let budget = ContextBudget::new(
            models.context_window,
            models.max_output_tokens,
            settings.compaction.soft_compact,
            settings.compaction.hard_compact,
            settings.compaction.keep_recent_tokens,
        );
        let skills = Arc::new(SkillLibrary::new(home));
        let reviewer = Arc::new(SkillReviewer::new(
            session.clone(),
            Arc::clone(&provider),
            Arc::clone(&memory),
            Arc::clone(&skills),
            models.model.clone(),
            models.kind.clone(),
        ));
        let registry = Arc::new(
            TaskRegistry::new()
                .register(
                    "reminder",
                    Arc::new(ReminderHandler::new(session.clone(), sender.clone())),
                )
                .register(
                    SKILL_REVIEW_KIND,
                    Arc::new(SkillReviewHandler::new(session.clone(), reviewer)),
                )
                .register(
                    BACKUP_KIND,
                    Arc::new(BackupHandler::new(home.to_path_buf())),
                ),
        );
        let scheduler = Scheduler::new(
            session.clone(),
            registry,
            format!("scheduler:{}", std::process::id()),
        );
        ensure_maintenance_tasks(&session)?;
        let helper_config = Arc::new(HelperConfig {
            settings: Arc::clone(&settings),
            session: session.clone(),
            provider: Arc::clone(&provider),
            sender: sender.clone(),
            speaker: speaker.clone(),
            memory: Arc::clone(&memory),
            soul: soul.clone(),
            home: home.to_path_buf(),
            workspace: workspace.clone(),
            spill: spill.clone(),
            outbound: home.join("media/outbound"),
            media: ToolMediaRoots {
                inbound: home.join("media/inbound"),
                outbound: home.join("media/outbound"),
            },
            model: models.model.clone(),
            provider_name: models.kind.clone(),
            budget,
            events: Arc::clone(&events) as Arc<dyn crate::events::EventSink>,
            owner: format!("helper:{}", std::process::id()),
        });
        let helpers = HelperPool::new(helper_config, Arc::clone(&helper_controls));
        let worker = TelegramWorker {
            settings: Arc::clone(&settings),
            session,
            memory,
            skills: Arc::clone(&skills),
            provider,
            sender,
            speaker,
            soul,
            home: home.to_path_buf(),
            workspace,
            spill,
            outbound: home.join("media/outbound"),
            media: ToolMediaRoots {
                inbound: home.join("media/inbound"),
                outbound: home.join("media/outbound"),
            },
            model: models.model.clone(),
            provider_name: models.kind.clone(),
            budget,
            pid: std::process::id(),
            cancellations,
            helper_controls,
            events,
        };
        Ok(Self {
            poller,
            worker,
            scheduler,
            helpers,
        })
    }
}

struct TelegramWorker {
    settings: Arc<Settings>,
    session: Session,
    memory: Arc<MemoryService>,
    skills: Arc<SkillLibrary>,
    provider: Arc<dyn Provider>,
    sender: TelegramSender,
    speaker: Option<Arc<dyn Speaker>>,
    soul: String,
    home: PathBuf,
    workspace: PathBuf,
    spill: PathBuf,
    outbound: PathBuf,
    media: ToolMediaRoots,
    model: String,
    provider_name: String,
    budget: ContextBudget,
    pid: u32,
    cancellations: Arc<CancellationRegistry>,
    helper_controls: Arc<HelperControls>,
    events: Arc<EventBus>,
}

#[derive(Default)]
struct CancellationRegistry {
    tokens: Mutex<HashMap<i64, Arc<AtomicBool>>>,
}

impl CancellationRegistry {
    fn register(self: &Arc<Self>, chat_id: i64) -> RunCancellation {
        let token = Arc::new(AtomicBool::new(false));
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.insert(chat_id, Arc::clone(&token));
        }
        RunCancellation {
            registry: Arc::clone(self),
            chat_id,
            token,
        }
    }

    fn remove(&self, chat_id: i64) {
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.remove(&chat_id);
        }
    }

    fn set_cancelled(&self, chat_id: i64) -> bool {
        let Ok(tokens) = self.tokens.lock() else {
            return false;
        };
        let Some(token) = tokens.get(&chat_id) else {
            return false;
        };
        token.store(true, Ordering::Relaxed);
        true
    }
}

struct RunCancellation {
    registry: Arc<CancellationRegistry>,
    chat_id: i64,
    token: Arc<AtomicBool>,
}

impl RunCancellation {
    fn token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.token)
    }
}

impl Drop for RunCancellation {
    fn drop(&mut self) {
        self.registry.remove(self.chat_id);
    }
}

struct RegistryStop(Arc<CancellationRegistry>);

impl StopHandler for RegistryStop {
    fn stop(&self, chat_id: i64) -> bool {
        self.0.set_cancelled(chat_id)
    }
}

impl TelegramWorker {
    fn run(&self, shutdown: Arc<AtomicBool>) {
        let mut last_idle_check = Instant::now();
        let mut last_idle_compaction = 0i64;
        let mut idle_attempted: HashMap<String, i64> = HashMap::new();
        while !shutdown.load(Ordering::Relaxed) {
            match self.session.next_queued_telegram_submission() {
                Ok(Some(submission)) => self.run_one(submission),
                Ok(None) => {
                    thread::sleep(Duration::from_millis(100));
                    if last_idle_check.elapsed() >= Duration::from_secs(60) {
                        last_idle_check = Instant::now();
                        let now = self.session.now_micros();
                        if now.saturating_sub(last_idle_compaction) >= 3_600_000_000 {
                            let min_tokens = 100_000u64.min(self.budget.soft);
                            match self.session.idle_compaction_candidate(
                                now,
                                3_600_000_000,
                                min_tokens,
                            ) {
                                Ok(Some(conversation)) => {
                                    let newest = self
                                        .session
                                        .history(&conversation.id)
                                        .ok()
                                        .and_then(|history| history.last().map(|entry| entry.seq))
                                        .unwrap_or(0);
                                    if idle_attempted.get(&conversation.id) == Some(&newest) {
                                        continue;
                                    }
                                    idle_attempted.insert(conversation.id.clone(), newest);
                                    if self.idle_compact(&conversation.id) {
                                        last_idle_compaction = now;
                                    }
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    tracing::warn!(%error, "idle compaction lookup failed")
                                }
                            }
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "queued submission lookup failed");
                    thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }

    fn idle_compact(&self, conversation_id: &str) -> bool {
        let history = match self.session.history_for_prompt(conversation_id) {
            Ok(history) => history,
            Err(error) => {
                tracing::warn!(%error, "idle compaction history failed");
                return false;
            }
        };
        let Some(plan) = plan_compaction(&history, self.budget.keep_recent) else {
            return false;
        };
        let compactor = Compactor::new(
            self.session.clone(),
            Arc::clone(&self.provider),
            self.model.clone(),
            self.provider_name.clone(),
            format!("daemon:idle:{}", self.pid),
        );
        match compactor.compact(conversation_id, &plan) {
            Ok(Some(_)) => {
                tracing::info!(conversation = conversation_id, "idle compaction complete");
                true
            }
            Ok(None) => false,
            Err(error) => {
                tracing::warn!(%error, "idle compaction failed");
                false
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

        self.recover_delivery(chat_id, thread_id);
        let run_cancel = self.cancellations.register(chat_id);

        let target = MessageTarget {
            session: self.session.clone(),
            conversation_id: conversation.id.clone(),
            sender: self.sender.clone(),
            chat_id,
            thread_id,
            outbound: self.outbound.clone(),
            speaker: self.speaker.clone(),
        };
        let built = match build_tools(
            &self.settings,
            &self.workspace,
            &self.spill,
            Some(target),
            Some(Arc::clone(&self.memory)),
            Some(Arc::clone(&self.skills)),
            Some(TaskTarget {
                depth: 0,
                controls: Arc::clone(&self.helper_controls),
                max_depth: self.settings.tasks.max_depth,
                max_concurrent: self.settings.tasks.max_concurrent,
            }),
            &self.media,
        ) {
            Ok(tools) => tools,
            Err(error) => {
                tracing::warn!(%error, "telegram tools could not be built");
                return;
            }
        };
        let agent = Agent::new(
            self.session.clone(),
            Arc::clone(&self.provider),
            AgentConfig {
                soul: self.soul.clone(),
                model: self.model.clone(),
                provider_name: self.provider_name.clone(),
                owner: format!("daemon:{}:{}", self.pid, submission.id),
                tools: built.core,
                spill_dir: self.spill.clone(),
                guidance: hearth_core::load_guidance(&self.home, &self.workspace).text,
            },
        )
        .with_memory(Arc::clone(&self.memory))
        .with_skills(Arc::clone(&self.skills))
        .with_groups(built.groups, built.selection)
        .with_budget(self.budget);

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
        let events: Arc<dyn crate::events::EventSink> = self.events.clone();
        let outcome = agent.run_submission_with_events(&submission.id, run_cancel.token(), events);
        let _ = stop_tx.send(());
        let _ = typing.join();

        match outcome {
            Ok(outcome) => {
                self.deliver(chat_id, thread_id, &outcome);
                if let Err(error) = agent.compact_soft(&conversation.id) {
                    tracing::warn!(%error, "soft compaction failed");
                }
            }
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
        let key = format!("tg:{}", outcome.submission.id);
        let payload = json!({
            "chat_id": chat_id,
            "thread_id": thread_id,
            "markdown": text,
            "submission_id": outcome.submission.id,
        });
        let intent = Save::new(&outcome.submission.conversation_id).effect(EffectIntent::new(
            "telegram.send",
            key,
            payload,
        ));
        let effect_id = match self.session.save(intent) {
            Ok(commit) => match commit.effect_ids.first().cloned() {
                Some(id) => id,
                None => {
                    tracing::warn!(chat_id, "delivery intent has no effect id");
                    return;
                }
            },
            Err(error) => {
                tracing::warn!(chat_id, %error, "delivery intent could not be saved");
                return;
            }
        };
        if let Err(error) = self.session.begin_effect(&effect_id) {
            tracing::warn!(chat_id, %error, "delivery effect could not start");
            return;
        }
        match self.sender.send_reply(chat_id, thread_id, text) {
            Ok(SendOutcome::Rich) => {
                if let Err(error) = self
                    .session
                    .finish_effect(&effect_id, json!({"delivered": true}))
                {
                    tracing::warn!(chat_id, %error, "delivery result could not be saved");
                }
                tracing::info!(chat_id, "telegram reply sent");
            }
            Ok(SendOutcome::Fallback { parts }) => {
                if let Err(error) = self
                    .session
                    .finish_effect(&effect_id, json!({"delivered": true, "parts": parts}))
                {
                    tracing::warn!(chat_id, %error, "delivery result could not be saved");
                }
                tracing::info!(chat_id, parts, "telegram fallback reply sent");
            }
            Ok(SendOutcome::Uncertain { message }) => {
                if let Err(error) = self
                    .session
                    .mark_effect_unknown(&effect_id, json!({"message": message}))
                {
                    tracing::warn!(chat_id, %error, "uncertain delivery could not be saved");
                }
                tracing::warn!(chat_id, %message, "telegram delivery is uncertain");
            }
            Err(error) => {
                if let Err(save_error) = self
                    .session
                    .mark_effect_failed(&effect_id, json!({"error": error.to_string()}))
                {
                    tracing::warn!(chat_id, %save_error, "failed delivery could not be saved");
                }
                tracing::warn!(chat_id, %error, "telegram reply failed");
            }
        }
    }

    fn recover_delivery(&self, chat_id: i64, thread_id: Option<i64>) {
        let effects = match self.session.effects_awaiting_delivery("telegram.send") {
            Ok(effects) => effects,
            Err(error) => {
                tracing::warn!(%error, "pending telegram deliveries could not be read");
                return;
            }
        };
        let Some(effect) = effects.into_iter().find(|effect| {
            effect.payload.get("reminder").and_then(Value::as_bool) != Some(true)
                && effect
                    .payload
                    .get("chat_id")
                    .and_then(|value| value.as_i64())
                    == Some(chat_id)
        }) else {
            return;
        };
        let markdown = effect
            .payload
            .get("markdown")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if markdown.is_empty() {
            let _ = self
                .session
                .mark_effect_failed(&effect.id, json!({"error": "empty recovered reply"}));
            return;
        }
        let recovered_thread = effect
            .payload
            .get("thread_id")
            .and_then(|value| value.as_i64())
            .or(thread_id);
        if effect.status == EffectStatus::Intent
            && let Err(error) = self.session.begin_effect(&effect.id)
        {
            tracing::warn!(%error, "recovered delivery could not start");
            return;
        }
        match self.sender.send_reply(
            chat_id,
            recovered_thread,
            &format!("Recovered reply:\n\n{markdown}"),
        ) {
            Ok(SendOutcome::Rich) | Ok(SendOutcome::Fallback { .. }) => {
                if let Err(error) = self
                    .session
                    .mark_effect_done(&effect.id, json!({"recovered": true}))
                {
                    tracing::warn!(%error, "recovered delivery could not be saved");
                } else {
                    tracing::info!(chat_id, effect = %effect.id, "recovered telegram reply sent");
                }
            }
            Ok(SendOutcome::Uncertain { message }) => {
                if let Err(error) = self
                    .session
                    .mark_effect_unknown(&effect.id, json!({"message": message}))
                {
                    tracing::warn!(%error, "uncertain recovered delivery could not be saved");
                }
                tracing::warn!(chat_id, %message, "recovered delivery is uncertain");
            }
            Err(error) => {
                if let Err(save_error) = self
                    .session
                    .mark_effect_failed(&effect.id, json!({"error": error.to_string()}))
                {
                    tracing::warn!(%save_error, "failed recovered delivery could not be saved");
                }
                tracing::warn!(chat_id, %error, "recovered delivery failed");
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

fn collect_secrets(settings: &Settings) -> Vec<String> {
    let mut secrets = Vec::new();
    if let Some(telegram) = &settings.telegram
        && let Some(token) = &telegram.bot_token
    {
        secrets.push(token.clone());
    }
    if let Some(models) = &settings.models {
        secrets.push(models.api_key.clone());
        for value in models.headers.values() {
            secrets.push(value.clone());
        }
    }
    if let Some(media) = &settings.media {
        if let Some(image) = &media.image
            && let Some(key) = &image.api_key
        {
            secrets.push(key.clone());
        }
        if let Some(stt) = &media.stt
            && let Some(key) = &stt.api_key
        {
            secrets.push(key.clone());
        }
        if let Some(tts) = &media.tts
            && let Some(key) = &tts.api_key
        {
            secrets.push(key.clone());
        }
    }
    if let Some(web) = &settings.web {
        for provider in web.search.providers.values() {
            if let Some(key) = &provider.api_key {
                secrets.push(key.clone());
            }
        }
        for provider in web.fetch.providers.values() {
            if let Some(key) = &provider.api_key {
                secrets.push(key.clone());
            }
        }
    }
    secrets.retain(|secret| hearth_core::secrets::is_redactable_value(secret));
    secrets
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

/// Where the embedding model lives: an absolute setting wins, a relative one
/// resolves against the Hearth home.
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
