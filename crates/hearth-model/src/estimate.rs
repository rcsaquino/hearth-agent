//! Token estimates for budgeting.

/// Estimate tokens as characters divided by four plus a ten percent margin.
pub fn estimate_tokens(text: &str) -> i64 {
    estimate_tokens_chars(text.chars().count() as u64)
}

/// The same estimate from a character count, for a block built from measured parts.
pub fn estimate_tokens_chars(chars: u64) -> i64 {
    if chars == 0 {
        return 0;
    }
    ((chars as f64 / 4.0) * 1.10).ceil() as i64
}
