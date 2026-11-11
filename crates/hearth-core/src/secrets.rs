//! Detection and redaction of credential-shaped text.
//!
//! This is intentionally a small hand-written matcher, not a full secret
//! scanner. It catches the common credential shapes the memory writer, the
//! transcript, the spill files, and the logs must not store.

use serde_json::{Map, Value};

/// The shortest configured value that is treated as a credential.
///
/// Shorter values are placeholders such as `-`, `none`, or `1`. Replacing them
/// across prompt or event text would corrupt ordinary output.
pub const MIN_SECRET_LENGTH: usize = 8;

/// Whether a configured value is long enough to redact as a credential.
pub fn is_redactable_value(value: &str) -> bool {
    value.chars().count() >= MIN_SECRET_LENGTH
}

/// Whether the text contains a credential-shaped token.
pub fn contains_secret(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    for prefix in ["sk-", "ghp_", "xoxb-", "akia", "-----begin"] {
        if lower.contains(prefix) {
            return true;
        }
    }

    let mut words = content.split_whitespace().peekable();
    while let Some(word) = words.next() {
        if word.eq_ignore_ascii_case("bearer")
            && words
                .peek()
                .is_some_and(|next| credential_value(next).is_some())
        {
            return true;
        }
        if let Some((left, right)) = word.split_once(':')
            && left.len() >= 8
            && left.chars().all(|character| character.is_ascii_digit())
            && right.len() >= 20
        {
            return true;
        }
        if key_value_secret(word).is_some() {
            return true;
        }
    }
    false
}

/// Redact credential-shaped tokens in plain text.
///
/// Whitespace is preserved so the result stays readable. A whole token is
/// replaced when it is a recognizable credential or a `key=value` pair whose
/// value looks like a credential. A `Bearer` token is redacted on the token
/// after it.
pub fn redact_secrets(content: &str) -> String {
    let mut output = String::with_capacity(content.len());
    let mut rest = content;
    let mut previous_bearer = false;
    while !rest.is_empty() {
        let whitespace_len = rest.len() - rest.trim_start().len();
        if whitespace_len > 0 {
            output.push_str(&rest[..whitespace_len]);
            rest = &rest[whitespace_len..];
            continue;
        }
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let word = &rest[..end];
        rest = &rest[end..];

        let redacted = if previous_bearer {
            credential_value(word).map(|_| "[redacted]".to_owned())
        } else if let Some((key, _)) = key_value_secret(word) {
            Some(format!("{key}=[redacted]"))
        } else if contains_word_secret(word) {
            Some("[redacted]".to_owned())
        } else {
            None
        };

        match redacted {
            Some(value) => output.push_str(&value),
            None => output.push_str(word),
        }
        previous_bearer = word.eq_ignore_ascii_case("bearer");
    }
    output
}

/// Recursively redact every string in a JSON value.
pub fn redact_value(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact_secrets(text)),
        Value::Array(items) => Value::Array(items.iter().map(redact_value).collect()),
        Value::Object(object) => {
            let mut redacted = Map::new();
            for (key, value) in object {
                redacted.insert(key.clone(), redact_value(value));
            }
            Value::Object(redacted)
        }
        other => other.clone(),
    }
}

fn contains_word_secret(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    for prefix in ["sk-", "ghp_", "xoxb-", "akia", "-----begin"] {
        if lower.contains(prefix) {
            return true;
        }
    }
    if let Some((left, right)) = word.split_once(':')
        && left.len() >= 8
        && left.chars().all(|character| character.is_ascii_digit())
        && right.len() >= 20
    {
        return true;
    }
    false
}

fn credential_value(word: &str) -> Option<&str> {
    let trimmed = word.trim_matches(|character: char| {
        !character.is_ascii_alphanumeric() && character != '-' && character != '_'
    });
    if trimmed.len() >= MIN_SECRET_LENGTH && !trimmed.starts_with('$') {
        Some(trimmed)
    } else {
        None
    }
}

/// Return the key part when a word is a `key=value` or `key:value` credential
/// pair with a credential-shaped value.
fn key_value_secret(word: &str) -> Option<(&str, &str)> {
    let (left, right) = word.split_once(['=', ':'])?;
    let key = left
        .trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .to_ascii_lowercase();
    let known = ["api_key", "apikey", "token", "password", "secret"]
        .iter()
        .any(|candidate| key == *candidate || key.ends_with(candidate));
    if !known {
        return None;
    }
    let value = right.trim_matches(|character: char| {
        character == '"' || character == '\'' || character == ',' || character == ';'
    });
    if value.len() < MIN_SECRET_LENGTH || value.starts_with('$') {
        return None;
    }
    Some((left, right))
}
