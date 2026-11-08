//! Executes recorded effects in the background.

use std::path::PathBuf;

use hearth_core::{CoreError, Effect, Session};
use hearth_tools::{FileWrite, ToolError};
use serde_json::{Value, json};

const BATCH: usize = 10;
const MAX_ATTEMPTS: i64 = 3;

/// Runs effect intents through their executors.
#[derive(Clone)]
pub struct EffectWorker {
    session: Session,
    files: FileWrite,
}

impl EffectWorker {
    /// Create a worker writing files under `workspace`.
    pub fn new(session: Session, workspace: PathBuf) -> Self {
        Self {
            session,
            files: FileWrite::new(workspace),
        }
    }

    /// Run one batch of pending effects.
    pub fn run_once(&self) -> Result<usize, CoreError> {
        let effects = self.session.pending_effects(BATCH)?;
        let mut processed = 0;
        for effect in effects {
            if effect.kind == "telegram.send" {
                continue;
            }
            let started = match self.session.begin_effect(&effect.id) {
                Ok(effect) => effect,
                Err(error) => {
                    tracing::warn!(effect = %effect.id, %error, "effect could not start");
                    continue;
                }
            };
            match self.execute(&started) {
                Ok(result) => {
                    hearth_storage::crash_if("during_effect");
                    self.session.finish_effect(&started.id, result)?;
                    tracing::info!(effect = %started.id, kind = %started.kind, "effect done");
                }
                Err(error) => {
                    let retryable = error.retryable() && started.attempts < MAX_ATTEMPTS;
                    let payload = json!({
                        "error": error.to_string(),
                        "retryable": retryable,
                    });
                    self.session.fail_effect(&started.id, payload, retryable)?;
                    tracing::warn!(
                        effect = %started.id,
                        kind = %started.kind,
                        retryable,
                        %error,
                        "effect failed"
                    );
                }
            }
            processed += 1;
        }
        Ok(processed)
    }

    /// Requeue effects that were interrupted after they started.
    pub fn recover_started(&self) -> Result<usize, CoreError> {
        let effects = self.session.started_effects()?;
        let mut changed = 0;
        for effect in effects {
            if effect.kind == "file_write" {
                self.session.requeue_effect(&effect.id)?;
                tracing::info!(effect = %effect.id, "effect requeued");
            } else if effect.kind == "telegram.send" {
                self.session
                    .mark_effect_unknown(&effect.id, json!({"error": "delivery interrupted"}))?;
                tracing::warn!(
                    effect = %effect.id,
                    kind = %effect.kind,
                    "telegram delivery interrupted"
                );
            } else {
                let payload = json!({
                    "error": "unsupported effect kind during recovery",
                    "retryable": false,
                });
                self.session.fail_effect(&effect.id, payload, false)?;
                tracing::warn!(
                    effect = %effect.id,
                    kind = %effect.kind,
                    "effect failed during recovery"
                );
            }
            changed += 1;
        }
        Ok(changed)
    }

    fn execute(&self, effect: &Effect) -> Result<Value, ToolError> {
        match effect.kind.as_str() {
            "file_write" => self.files.write(&effect.payload),
            other => Err(ToolError::invalid(
                "unsupported_effect",
                format!("unsupported effect kind: {other}"),
            )),
        }
    }
}
