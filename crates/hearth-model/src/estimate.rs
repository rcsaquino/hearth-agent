//! Token estimates for budgeting.

/// Estimate tokens as characters divided by four plus a ten percent margin.
pub fn estimate_tokens(text: &str) -> i64 {
    if text.is_empty() {
        return 0;
    }
    ((text.chars().count() as f64 / 4.0) * 1.10).ceil() as i64
}
