//! Retrieval and standing-memory constants.
//!
//! These are the provisional Q6 values. The M4 benchmark may tune them; the
//! spec records the settled numbers at M4.

/// Reciprocal rank fusion constant.
pub const RRF_K: f64 = 60.0;
/// Recency half-life in days.
pub const RECENCY_HALF_LIFE_DAYS: f64 = 180.0;
/// Token Jaccard similarity at which two memories count as near duplicates.
pub const NEAR_DUPLICATE: f64 = 0.85;
/// Weight of recency in the recall score.
pub const RECENCY_WEIGHT: f64 = 0.20;
/// Weight of importance in the recall score.
pub const IMPORTANCE_WEIGHT: f64 = 0.20;
/// Weight of staleness in the recall score.
pub const STALENESS_WEIGHT: f64 = 0.15;
/// Default recall token budget.
pub const RECALL_MAX_TOKENS: usize = 900;
/// Default recall item cap.
pub const RECALL_MAX_ITEMS: usize = 6;
/// Default standing-memory token budget.
pub const STANDING_MAX_TOKENS: usize = 1200;
/// Importance at which a rule, preference, or person becomes standing memory.
pub const PROMOTE_IMPORTANCE: i64 = 85;
/// Inputs that never trigger recall.
pub const TRIVIAL: &[&str] = &[
    "hi",
    "hello",
    "hey",
    "thanks",
    "thank you",
    "ok",
    "okay",
    "sure",
    "yes",
    "no",
    "good morning",
    "good afternoon",
    "good evening",
    "lol",
];

/// Retrieval and standing-memory budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryPolicy {
    pub standing_max_tokens: usize,
    pub recall_max_tokens: usize,
    pub recall_max_items: usize,
}

impl Default for MemoryPolicy {
    fn default() -> Self {
        Self {
            standing_max_tokens: STANDING_MAX_TOKENS,
            recall_max_tokens: RECALL_MAX_TOKENS,
            recall_max_items: RECALL_MAX_ITEMS,
        }
    }
}

/// Days without use for one 5 percent importance decay step.
pub const DECAY_DAYS: f64 = 90.0;
/// Importance multiplier per decay step.
pub const DECAY_FACTOR: f64 = 0.95;
/// Floor for the decayed importance of an unpinned item.
pub const DECAY_FLOOR: i64 = 15;
/// Days without use before a low-importance memory is archived.
pub const ARCHIVE_UNUSED_DAYS: i64 = 3 * 365;
/// Stored importance below which an unused memory can be archived.
pub const ARCHIVE_BELOW_IMPORTANCE: i64 = 20;

/// Common query words that add no retrieval signal.
pub const STOPWORDS: &[&str] = &[
    "a", "about", "an", "and", "are", "as", "at", "be", "been", "being", "but", "by", "can",
    "could", "did", "do", "does", "for", "from", "had", "has", "have", "he", "her", "here", "him",
    "his", "i", "if", "in", "into", "is", "it", "its", "may", "me", "might", "must", "my", "no",
    "not", "of", "on", "or", "our", "she", "should", "so", "some", "than", "that", "the", "their",
    "them", "then", "there", "these", "they", "this", "those", "to", "was", "we", "were", "will",
    "with", "would", "you", "your",
];
