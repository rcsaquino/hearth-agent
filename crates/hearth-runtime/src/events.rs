//! Live run events for local watchers.

use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// One event emitted by a run.
#[derive(Debug, Clone)]
pub struct RunEvent {
    pub task_id: String,
    pub conversation_id: String,
    pub kind: &'static str,
    pub data: Value,
}

/// A sink for live run events.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: RunEvent);
}

/// A sink that discards events.
pub struct NoopSink;

impl EventSink for NoopSink {
    fn emit(&self, _event: RunEvent) {}
}

/// Fan-out of live events to local watchers.
pub struct EventBus {
    subscribers: Mutex<Vec<Sender<String>>>,
    secrets: Vec<String>,
}

impl EventBus {
    /// Build a bus that redacts the supplied secret values.
    pub fn new(secrets: Vec<String>) -> Self {
        Self {
            subscribers: Mutex::new(Vec::new()),
            secrets: secrets
                .into_iter()
                .filter(|secret| !secret.is_empty())
                .collect(),
        }
    }

    /// Subscribe to events from this point on. Earlier events are not replayed.
    pub fn subscribe(&self) -> Receiver<String> {
        let (sender, receiver) = mpsc::channel();
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(sender);
        }
        receiver
    }

    /// Serialize and fan out one event.
    pub fn publish(&self, event: RunEvent) {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_micros() as i64)
            .unwrap_or(0);
        let mut text = json!({
            "v": 1,
            "ts": ts,
            "kind": event.kind,
            "task_id": event.task_id,
            "conversation_id": event.conversation_id,
            "data": event.data,
        })
        .to_string();
        for secret in &self.secrets {
            if text.contains(secret) {
                text = text.replace(secret, "[redacted]");
            }
        }
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|subscriber| subscriber.send(text.clone()).is_ok());
        }
    }

    /// The current number of attached watchers, for tests.
    pub fn subscriber_count(&self) -> usize {
        self.subscribers
            .lock()
            .map(|subscribers| subscribers.len())
            .unwrap_or(0)
    }
}

impl EventSink for EventBus {
    fn emit(&self, event: RunEvent) {
        self.publish(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_serialized_and_redacted() {
        let bus = EventBus::new(vec!["super-secret".to_owned()]);
        let receiver = bus.subscribe();
        bus.publish(RunEvent {
            task_id: "task-1".to_owned(),
            conversation_id: "conv-1".to_owned(),
            kind: "text_delta",
            data: json!({"text": "the key is super-secret"}),
        });
        let line = receiver.recv().expect("event");
        assert!(line.contains("[redacted]"));
        assert!(!line.contains("super-secret"));
        let value: Value = serde_json::from_str(&line).expect("json");
        assert_eq!(value["v"], 1);
        assert_eq!(value["kind"], "text_delta");
        assert_eq!(value["task_id"], "task-1");
    }

    #[test]
    fn detached_subscribers_are_dropped() {
        let bus = EventBus::new(Vec::new());
        let receiver = bus.subscribe();
        drop(receiver);
        bus.publish(RunEvent {
            task_id: "task-1".to_owned(),
            conversation_id: "conv-1".to_owned(),
            kind: "run_started",
            data: json!({}),
        });
        assert_eq!(bus.subscriber_count(), 0);
    }
}
