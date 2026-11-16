//! Context window budgets and prompt segment accounting.

use serde_json::Value;

/// The identity layer budget.
pub const SEGMENT_IDENTITY: u64 = 800;
/// The tool schema budget.
pub const SEGMENT_TOOLS: u64 = 1_200;
/// The rules budget.
pub const SEGMENT_RULES: u64 = 800;

/// The skills index budget.
pub const SEGMENT_SKILLS: u64 = 300;
/// The recall digest budget.
pub const SEGMENT_RECALL: u64 = 300;
/// The runtime note budget.
pub const SEGMENT_RUNTIME: u64 = 150;
/// The fixed prompt overhead target from section 14.
pub const FIXED_OVERHEAD_TARGET: u64 = 4_500;

/// The budget each prompt segment is measured against, in the order the model
/// reads them.
pub const SEGMENT_BUDGETS: [(&str, u64); 6] = [
    ("identity", SEGMENT_IDENTITY),
    ("tools", SEGMENT_TOOLS),
    ("rules", SEGMENT_RULES),
    ("skills", SEGMENT_SKILLS),
    ("recall", SEGMENT_RECALL),
    ("runtime", SEGMENT_RUNTIME),
];

/// One prompt segment's count against its budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentCost {
    pub name: &'static str,
    pub tokens: u64,
    pub budget: u64,
}

impl SegmentCost {
    /// Whether the segment costs more than its budget.
    pub fn over(&self) -> bool {
        self.tokens > self.budget
    }
}

const RESERVE_FLOOR: u64 = 4_096;
const RESERVE_EXTRA: u64 = 1_024;
const KEEP_FRACTION: f64 = 0.35;

/// The room a model window leaves after reserving the reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudget {
    pub context_window: u64,
    pub max_output_tokens: u64,
    pub reserve: u64,
    pub usable: u64,
    pub soft: u64,
    pub hard: u64,
    pub keep_recent: u64,
}

impl ContextBudget {
    /// Compute the budget from the configured window and compaction thresholds.
    pub fn new(
        context_window: u64,
        max_output_tokens: u64,
        soft_compact: f64,
        hard_compact: f64,
        keep_recent_tokens: u64,
    ) -> Self {
        let reserve = max_output_tokens
            .max(RESERVE_FLOOR)
            .saturating_add(RESERVE_EXTRA);
        let usable = context_window.saturating_sub(reserve);
        Self {
            context_window,
            max_output_tokens,
            reserve,
            usable,
            soft: fraction(usable, soft_compact),
            hard: fraction(usable, hard_compact),
            keep_recent: keep_recent_tokens.min(fraction(usable, KEEP_FRACTION)),
        }
    }
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self::new(128_000, 32_000, 0.65, 0.85, 20_000)
    }
}

fn fraction(total: u64, fraction: f64) -> u64 {
    if total == 0 {
        return 0;
    }
    (total as f64 * fraction.clamp(0.0, 1.0)).floor() as u64
}

/// The estimated prompt token counts for each layer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenCounts {
    pub identity: u64,
    pub tools: u64,
    pub rules: u64,
    pub skills: u64,
    pub guidance: u64,
    pub transcript: u64,
    pub recall: u64,
    pub runtime: u64,
    pub total: u64,
    pub fixed: u64,
}

impl TokenCounts {
    /// Build counts and compute the total and fixed overhead.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        identity: u64,
        tools: u64,
        rules: u64,
        skills: u64,
        guidance: u64,
        transcript: u64,
        recall: u64,
        runtime: u64,
    ) -> Self {
        let total = identity + tools + rules + skills + guidance + transcript + recall + runtime;
        let fixed = identity + tools + rules + skills + guidance + runtime;
        Self {
            identity,
            tools,
            rules,
            skills,
            guidance,
            transcript,
            recall,
            runtime,
            total,
            fixed,
        }
    }

    /// Read the counts a saved prompt document recorded.
    pub fn from_document(document: &Value) -> Option<Self> {
        let tokens = document.get("tokens")?;
        let count = |name: &str| tokens.get(name).and_then(Value::as_u64);
        Some(Self {
            identity: count("identity")?,
            tools: count("tools")?,
            rules: count("rules")?,
            skills: count("skills")?,
            guidance: count("guidance")?,
            transcript: count("transcript")?,
            recall: count("recall")?,
            runtime: count("runtime")?,
            total: count("total")?,
            fixed: count("fixed")?,
        })
    }

    /// One segment's count, or `None` for a name no segment carries.
    pub fn segment(&self, name: &str) -> Option<u64> {
        match name {
            "identity" => Some(self.identity),
            "tools" => Some(self.tools),
            "rules" => Some(self.rules),
            "skills" => Some(self.skills),
            "guidance" => Some(self.guidance),
            "transcript" => Some(self.transcript),
            "recall" => Some(self.recall),
            "runtime" => Some(self.runtime),
            "total" => Some(self.total),
            "fixed" => Some(self.fixed),
            _ => None,
        }
    }

    /// Every budgeted segment with its count, in prompt order.
    pub fn segments(&self) -> impl Iterator<Item = SegmentCost> + '_ {
        SEGMENT_BUDGETS
            .into_iter()
            .filter_map(move |(name, budget)| {
                self.segment(name).map(|tokens| SegmentCost {
                    name,
                    tokens,
                    budget,
                })
            })
    }

    /// The names of segments over their spec budgets.
    pub fn over_budget(&self) -> Vec<&'static str> {
        self.segments()
            .filter(SegmentCost::over)
            .map(|segment| segment.name)
            .collect()
    }
}
