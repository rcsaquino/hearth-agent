//! Validation, limits, and text helpers, ported from `src/text.ts`.
//!
//! Every bound here is the original's number. They are part of the contract:
//! a caller that relied on a 20,000-character memory limit or a 5,000-character
//! standing file keeps working after the port.

use hearth_core::secrets::contains_secret;

use crate::errors::{ErrorCode, ExpectedError, Result};

/// The whole `standing.md` budget, in characters of serialized text.
pub const STANDING_LIMIT: usize = 5_000;
/// One long-term memory's hard storage length, in characters.
pub const MEMORY_LIMIT: usize = 20_000;
/// The normal length for a new or edited memory.
pub const NEW_MEMORY_LIMIT: usize = 2_000;
/// The normal tag count for a new or edited memory.
pub const NEW_TAG_LIMIT: usize = 8;
/// The soft ceiling for one tool result, in bytes.
pub const RESULT_BYTES: usize = 32_000;

/// Trim and validate a required string.
pub fn nonempty(value: &str, name: &str, max: usize) -> Result<String> {
    let text = value.trim();
    if text.is_empty() || text.contains('\0') || text.chars().count() > max {
        return Err(ExpectedError::with_details(
            ErrorCode::InvalidInput,
            format!("{name} must contain 1-{max} characters and no NUL bytes."),
            serde_json::json!({ "field": name }),
        ));
    }
    Ok(text.to_owned())
}

/// Refuse credential-shaped content before it reaches either store.
pub fn no_credentials(content: &str) -> Result<()> {
    if contains_secret(content) {
        return Err(ExpectedError::new(
            ErrorCode::InvalidInput,
            "credential-like memory content was refused.",
        ));
    }
    Ok(())
}

/// Validate an integer within an inclusive range.
pub fn integer(value: i64, name: &str, min: i64, max: i64) -> Result<i64> {
    if value < min || value > max {
        return Err(ExpectedError::with_details(
            ErrorCode::InvalidInput,
            format!("{name} must be an integer from {min} to {max}."),
            serde_json::json!({ "field": name, "min": min, "max": max }),
        ));
    }
    Ok(value)
}

/// Normalize tags: trim each, reject empties, deduplicate, cap at 30.
pub fn tags(value: Option<&[String]>) -> Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.len() > 30 {
        return Err(ExpectedError::with_details(
            ErrorCode::InvalidInput,
            "Use at most 30 tags.",
            serde_json::json!({ "field": "tags", "max_items": 30 }),
        ));
    }
    let mut seen = Vec::with_capacity(value.len());
    for tag in value {
        let normalized = nonempty(tag, "tag", 80)?;
        if !seen.contains(&normalized) {
            seen.push(normalized);
        }
    }
    Ok(seen)
}

/// Hot content must be one literal line; internal spacing is preserved exactly.
///
/// This is the rule that keeps `standing.md` parseable. A newline would let one
/// bullet masquerade as two, which the parser would then read as a separate
/// standing instruction.
pub fn single_line(value: &str, name: &str, max: usize) -> Result<String> {
    if value.contains(['\r', '\n', '\u{2028}', '\u{2029}']) {
        return Err(ExpectedError::with_details(
            ErrorCode::InvalidInput,
            format!(
                "{name} must be a single line. Replace the line break with a space, or save multiline content in SQLite."
            ),
            serde_json::json!({ "field": name }),
        ));
    }
    nonempty(value, name, max)
}

/// A window of text plus where the next window starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    /// `None` when this window reaches the end.
    pub next_text_offset: Option<usize>,
    pub total_characters: usize,
}

/// Slice text without splitting a character.
///
/// The original guards the UTF-16 surrogate case because JavaScript strings are
/// UTF-16. Rust strings are UTF-8, so the equivalent hazard is a multi-byte
/// character straddling the boundary; slicing by characters covers both.
pub fn chunk(text: &str, offset: usize, limit: usize) -> Chunk {
    let total_characters = text.chars().count();
    if offset >= total_characters {
        return Chunk {
            text: String::new(),
            next_text_offset: None,
            total_characters,
        };
    }
    let slice: String = text.chars().skip(offset).take(limit).collect();
    let end = offset + slice.chars().count();
    Chunk {
        text: slice,
        next_text_offset: if end < total_characters {
            Some(end)
        } else {
            None
        },
        total_characters,
    }
}

/// A bounded window of text centered near the first match.
///
/// The original works in code units. This works in characters so the excerpt
/// length is stable across scripts, and keeps the same 120-character lead-in and
/// ellipsis markers.
pub fn excerpt(text: &str, needle: &str, limit: usize) -> String {
    let index = if needle.is_empty() {
        0
    } else {
        let haystack = text.to_lowercase();
        let needle_lower = needle.to_lowercase();
        match haystack.find(&needle_lower) {
            Some(byte_index) => haystack[..byte_index].chars().count(),
            None => 0,
        }
    };
    let mut start = index.saturating_sub(120);
    // With a window smaller than the lead-in, the match can fall past the end.
    // Pull the window back so the excerpt always shows what was matched.
    if index >= start + limit {
        start = index.saturating_sub(limit / 4);
    }
    let part = chunk(text, start, limit);
    format!(
        "{}{}{}",
        if start > 0 { "\u{2026}" } else { "" },
        part.text,
        if part.next_text_offset.is_some() {
            "\u{2026}"
        } else {
            ""
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_rejects_a_newline() {
        let error = single_line("two\nlines", "content", MEMORY_LIMIT).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn single_line_preserves_internal_spacing() {
        assert_eq!(
            single_line("  spaced   out  ", "content", MEMORY_LIMIT).unwrap(),
            "spaced   out"
        );
    }

    #[test]
    fn tags_deduplicate_and_cap() {
        let value = vec!["fruit".to_owned(), "fruit".to_owned()];
        assert_eq!(tags(Some(&value)).unwrap(), vec!["fruit".to_owned()]);

        let many: Vec<String> = (0..31).map(|index| format!("t{index}")).collect();
        assert_eq!(tags(Some(&many)).unwrap_err().code, ErrorCode::InvalidInput);
    }

    #[test]
    fn chunk_reports_the_next_offset_until_the_end() {
        let part = chunk("abcdef", 0, 4);
        assert_eq!(part.text, "abcd");
        assert_eq!(part.next_text_offset, Some(4));
        assert_eq!(chunk("abcdef", 4, 4).next_text_offset, None);
    }

    #[test]
    fn chunk_respects_multibyte_boundaries() {
        let part = chunk("日本語テキスト", 0, 2);
        assert_eq!(part.text, "日本");
        assert_eq!(part.next_text_offset, Some(2));
    }

    #[test]
    fn excerpt_marks_a_clipped_lead_in() {
        let text = format!("{}needle{}", "x".repeat(400), "y".repeat(400));
        let value = excerpt(&text, "needle", 100);
        assert!(value.starts_with('\u{2026}'));
        assert!(value.ends_with('\u{2026}'));
        assert!(value.contains("needle"));
    }
}
