//! Clocks and identifiers, injected so behavior stays deterministic in tests.

use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

/// Reads the current time in UTC microseconds.
pub trait Clock: Send + Sync {
    fn now_micros(&self) -> i64;
}

/// Creates stable identifiers.
pub trait IdGen: Send + Sync {
    fn new_id(&self) -> String;
}

/// The production clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_micros(&self) -> i64 {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        duration.as_micros() as i64
    }
}

/// The production identifier source, time-ordered UUIDv7.
pub struct UuidV7;

impl IdGen for UuidV7 {
    fn new_id(&self) -> String {
        Uuid::now_v7().to_string()
    }
}
