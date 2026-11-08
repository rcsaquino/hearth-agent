//! Per-model token prices.

use std::collections::BTreeMap;

use crate::types::Usage;

/// Prices in micros per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Price {
    pub input: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub output: i64,
    pub reasoning: i64,
}

impl Price {
    /// Compute a cost in micros, rounding each category half up.
    pub fn cost_micros(&self, usage: &Usage) -> i64 {
        cost(usage.input_tokens, self.input)
            + cost(usage.cache_read_tokens, self.cache_read)
            + cost(usage.cache_write_tokens, self.cache_write)
            + cost(usage.output_tokens, self.output)
            + cost(usage.reasoning_tokens, self.reasoning)
    }
}

fn cost(tokens: i64, micros_per_million: i64) -> i64 {
    if tokens == 0 || micros_per_million == 0 {
        return 0;
    }
    (tokens.saturating_mul(micros_per_million) + 500_000) / 1_000_000
}

/// A registry of prices by model name.
#[derive(Debug, Clone, Default)]
pub struct Pricing {
    prices: BTreeMap<String, Price>,
}

impl Pricing {
    /// An empty registry, where every model is unpriced.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add or replace a model price.
    pub fn insert(&mut self, model: impl Into<String>, price: Price) {
        self.prices.insert(model.into(), price);
    }

    /// The price for a model, when known.
    pub fn price(&self, model: &str) -> Option<Price> {
        self.prices.get(model).copied()
    }
}
