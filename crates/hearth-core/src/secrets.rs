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
    let mut words = content.split_whitespace().peekable();
    while let Some(word) = words.next() {
        if contains_word_secret(word) {
            return true;
        }
        if word.eq_ignore_ascii_case("bearer")
            && words
                .peek()
                .is_some_and(|next| credential_value(next).is_some())
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

        let redacted = secret_span(word, previous_bearer).map(|span| redact_span(word, span));

        match redacted {
            Some(value) => output.push_str(&value),
            None => output.push_str(word),
        }
        previous_bearer = word.eq_ignore_ascii_case("bearer");
    }
    output
}

/// Redact credential shapes and configured credential values in one string.
///
/// Configured values are matched longest first, so a key that contains a shorter
/// key is still replaced whole.
pub fn redact_with_values(content: &str, values: &[String]) -> String {
    let mut redacted = redact_secrets(content);
    let mut values: Vec<&str> = values
        .iter()
        .map(String::as_str)
        .filter(|value| is_redactable_value(value))
        .collect();
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    for value in values {
        redacted = redacted.replace(value, "[redacted]");
    }
    redacted
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
    credential_span(word).is_some() || bot_token(word) || key_value_secret(word).is_some()
}

/// The byte range to redact inside one whitespace-delimited word.
///
/// Punctuation outside the credential stays, so a serialized document still
/// parses after redaction.
fn secret_span(word: &str, previous_bearer: bool) -> Option<std::ops::Range<usize>> {
    if previous_bearer {
        return token_span(word);
    }
    if let Some((_, span)) = key_value_secret(word) {
        return Some(span);
    }
    if bot_token(word) {
        return Some(0..word.len());
    }
    credential_span(word)
}

/// The byte range of a credential inside one word.
///
/// A prefix only counts at the start of a token, so ordinary words such as
/// `task-id` and `risk-free` are not credentials.
fn credential_span(word: &str) -> Option<std::ops::Range<usize>> {
    let lower = word.to_ascii_lowercase();
    for prefix in ["sk-", "ghp_", "xoxb-", "akia", "-----begin"] {
        let mut search = 0;
        while let Some(found) = lower[search..].find(prefix) {
            let start = search + found;
            let at_start = word[..start]
                .chars()
                .next_back()
                .is_none_or(|before| !is_token_char(before));
            if at_start {
                let mut end = start + prefix.len();
                while let Some(character) = word[end..].chars().next() {
                    if !is_token_char(character) {
                        break;
                    }
                    end += character.len_utf8();
                }
                return Some(start..end);
            }
            search = start + prefix.len();
        }
    }
    None
}

fn is_token_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '-' || character == '_'
}

/// A Telegram bot token, `12345678:abcdefghijklmnopqrstuvwx`.
fn bot_token(word: &str) -> bool {
    word.split_once(':').is_some_and(|(left, right)| {
        left.len() >= 8
            && left.chars().all(|character| character.is_ascii_digit())
            && right.len() >= 20
    })
}

fn redact_span(word: &str, span: std::ops::Range<usize>) -> String {
    format!("{}[redacted]{}", &word[..span.start], &word[span.end..])
}

fn credential_value(word: &str) -> Option<&str> {
    Some(&word[token_span(word)?])
}

/// The range of a bare credential token, without its surrounding punctuation.
fn token_span(word: &str) -> Option<std::ops::Range<usize>> {
    let trimmed = word.trim_matches(|character: char| !is_token_char(character));
    let start = word.len()
        - word
            .trim_start_matches(|character: char| !is_token_char(character))
            .len();
    let end = start + trimmed.len();
    (trimmed.len() >= MIN_SECRET_LENGTH && !trimmed.starts_with('$')).then_some(start..end)
}

/// Return the key part and the value's byte range when a word is a `key=value`
/// or `key:value` credential pair with a credential-shaped value.
fn key_value_secret(word: &str) -> Option<(&str, std::ops::Range<usize>)> {
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
    let leading = right.len() - right.trim_start_matches(['"', '\'', ',', ';']).len();
    let value = &right[leading..];
    let length = value
        .find(|character: char| {
            character.is_whitespace() || matches!(character, '"' | '\'' | ',' | ';' | '}' | ']')
        })
        .unwrap_or(value.len());
    let value = &value[..length];
    if value.len() < MIN_SECRET_LENGTH || value.starts_with('$') {
        return None;
    }
    let start = left.len() + 1 + leading;
    Some((left, start..start + value.len()))
}
