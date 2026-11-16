//! The free safety net for statements the user asked Hearth to keep.
//!
//! The agent's own `memory_save` call is the primary write path. This exists
//! because "remember that ..." is a promise, and a model that forgets to call a
//! tool has broken it. The extractor is therefore narrow on purpose: four
//! phrasings where the user's intent is not in doubt.
//!
//! It never calls a model, so it costs no tokens and no latency. It runs after
//! the reply has been delivered, and a wrong candidate is cheap to repair: it
//! appears in the next recall digest, the agent sees it, and one `memory_forget`
//! removes it. Precision is still worth more than recall here, because a noisy
//! store spends the user's tokens on every turn from then on.

use crate::model::{Kind, NewMemory, Source};
use crate::query::normalize;

/// One statement the extractor is confident enough to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub content: String,
    pub kind: Kind,
    pub subject: String,
}

impl Candidate {
    /// The write this candidate becomes.
    pub fn into_new(self, entry: Option<String>) -> NewMemory {
        NewMemory::new(self.content)
            .kind(self.kind)
            .subject(self.subject)
            .source(Source::Extract)
            .from_entry(entry)
    }
}

/// The shortest statement worth keeping.
const MIN_CHARS: usize = 8;
/// The longest statement worth keeping.
const MAX_CHARS: usize = 200;
/// The longest message that may contain a statement worth keeping.
///
/// A long message is a request, not a durable statement, but a short promise can
/// still sit inside one. This guard is separate from the candidate limit so a
/// long message is scanned rather than skipped.
const MESSAGE_CHARS: usize = 1_600;
/// The most candidates one message may produce.
const MAX_CANDIDATES: usize = 3;

/// Phrasings that introduce a statement the user wants kept.
const REMEMBER: &[&str] = &[
    "remember that ",
    "remember: ",
    "remember ",
    "please remember ",
    "do remember ",
    "note that ",
    "keep in mind that ",
    "don't forget that ",
    "do not forget that ",
    "don't forget ",
    "for future reference, ",
];

/// Phrasings that state a standing preference.
const PREFERENCE: &[&str] = &[
    "i prefer ",
    "i always ",
    "i never ",
    "i usually ",
    "i hate ",
    "i love ",
    "my favourite ",
    "my favorite ",
    "from now on ",
];

/// Relations that make "X is my Y" a fact about a person.
const RELATIONS: &[&str] = &[
    "wife",
    "husband",
    "spouse",
    "partner",
    "son",
    "daughter",
    "kid",
    "child",
    "mother",
    "mom",
    "mum",
    "father",
    "dad",
    "sister",
    "brother",
    "boss",
    "manager",
    "friend",
    "colleague",
    "coworker",
    "therapist",
    "doctor",
];

/// Statements worth keeping from one user message.
pub fn candidates(text: &str) -> Vec<Candidate> {
    let message = normalize_spacing(text);
    if message.chars().count() < MIN_CHARS || message.chars().count() > MESSAGE_CHARS {
        return Vec::new();
    }
    let mut found: Vec<Candidate> = Vec::new();
    for sentence in sentences(&message) {
        let sentence = sentence.trim();
        if found.len() >= MAX_CANDIDATES {
            break;
        }
        if sentence.ends_with('?') {
            continue;
        }
        if let Some(candidate) = from_remember(sentence)
            .or_else(|| from_possessive(sentence))
            .or_else(|| from_relation(sentence))
            .or_else(|| from_preference(sentence))
            && acceptable(&candidate)
            && !found
                .iter()
                .any(|existing| existing.content == candidate.content)
        {
            found.push(candidate);
        }
    }
    found
}

fn normalize_spacing(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sentences(text: &str) -> Vec<&str> {
    let mut spans = Vec::new();
    let mut start = 0;
    for (index, character) in text.char_indices() {
        if matches!(character, '.' | '!' | '?' | '\n') {
            let end = index + character.len_utf8();
            let span = &text[start..end];
            if !span.trim().is_empty() {
                spans.push(span);
            }
            start = end;
        }
    }
    let rest = &text[start..];
    if !rest.trim().is_empty() {
        spans.push(rest);
    }
    spans
}

fn trim_end(sentence: &str) -> String {
    sentence
        .trim()
        .trim_end_matches(['.', '!', '?'])
        .trim()
        .to_owned()
}

fn from_remember(sentence: &str) -> Option<Candidate> {
    let lowered = sentence.to_lowercase();
    let phrase = REMEMBER
        .iter()
        .find(|phrase| lowered.starts_with(**phrase))?;
    let content = trim_end(&sentence[phrase.len()..]);
    if content.is_empty() {
        return None;
    }
    Some(Candidate {
        subject: leading_subject(&content),
        content,
        kind: Kind::Fact,
    })
}

/// "my sister is Alice" and "my favourite colour is green".
fn from_possessive(sentence: &str) -> Option<Candidate> {
    let lowered = sentence.to_lowercase();
    if !lowered.starts_with("my ") {
        return None;
    }
    let rest = sentence.get(3..)?;
    let (noun, value) = split_ignore_case(rest, " is ")?;
    let noun = noun.trim();
    let value = value.trim();
    if noun.is_empty() || value.is_empty() {
        return None;
    }
    let first = noun.split_whitespace().next().unwrap_or("").to_lowercase();
    let relation = RELATIONS.contains(&first.as_str());
    Some(Candidate {
        subject: if relation {
            leading_subject(value)
        } else {
            normalize(noun)
        },
        content: trim_end(sentence),
        kind: if relation { Kind::Person } else { Kind::Fact },
    })
}

/// "Alice is my sister", "Bob is my boss".
fn from_relation(sentence: &str) -> Option<Candidate> {
    let (left, right) = split_ignore_case(sentence, " is my ")?;
    let relation = right
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|character: char| !character.is_alphanumeric())
        .to_lowercase();
    if !RELATIONS.contains(&relation.as_str()) || left.trim().is_empty() {
        return None;
    }
    let subject = leading_subject(left.trim());
    if subject.is_empty() {
        return None;
    }
    Some(Candidate {
        subject,
        content: trim_end(sentence),
        kind: Kind::Person,
    })
}

/// Split on a lowercase marker while keeping the original case.
///
/// The marker is ASCII, so a length change during folding means the split would
/// not line up and the sentence is left alone.
fn split_ignore_case<'a>(text: &'a str, marker: &str) -> Option<(&'a str, &'a str)> {
    let lowered = text.to_lowercase();
    if lowered.len() != text.len() {
        return None;
    }
    let at = lowered.find(marker)?;
    Some((&text[..at], &text[at + marker.len()..]))
}

fn from_preference(sentence: &str) -> Option<Candidate> {
    let lowered = sentence.to_lowercase();
    if !PREFERENCE.iter().any(|phrase| lowered.starts_with(*phrase)) {
        return None;
    }
    let content = trim_end(sentence);
    Some(Candidate {
        subject: "user".to_owned(),
        content,
        kind: Kind::Preference,
    })
}

fn acceptable(candidate: &Candidate) -> bool {
    let words = candidate.content.split_whitespace().count();
    let length = candidate.content.chars().count();
    words >= 3 && (MIN_CHARS..=MAX_CHARS).contains(&length)
}

/// The name a statement is about, when it opens with one.
pub(crate) fn leading_subject(text: &str) -> String {
    let mut name: Vec<&str> = Vec::new();
    for (index, word) in text.split_whitespace().enumerate() {
        let trimmed = word.trim_matches(|character: char| !character.is_alphanumeric());
        if index == 0 {
            if !matches!(
                trimmed.to_lowercase().as_str(),
                "i" | "my" | "we" | "our" | "the" | "a" | "an"
            ) && looks_like_name(trimmed)
            {
                name.push(trimmed);
                continue;
            }
            return String::new();
        }
        if looks_like_name(trimmed) {
            name.push(trimmed);
        } else {
            break;
        }
    }
    normalize(&name.join(" "))
}

fn looks_like_name(word: &str) -> bool {
    word.chars().next().is_some_and(char::is_uppercase) && word.chars().any(char::is_alphabetic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_remember_statement_is_captured() {
        let found = candidates("Remember that Alice manages the OR board.");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].content, "Alice manages the OR board");
        assert_eq!(found[0].subject, "alice");
    }

    #[test]
    fn a_possessive_about_a_person_uses_the_name_as_the_subject() {
        let found = candidates("My sister is Alice.");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, Kind::Person);
        assert_eq!(found[0].subject, "alice");
    }

    #[test]
    fn a_relation_statement_about_a_name_is_a_person() {
        let found = candidates("Bob is my boss.");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].subject, "bob");
        assert_eq!(found[0].kind, Kind::Person);
    }

    #[test]
    fn a_stated_preference_is_captured() {
        let found = candidates("I prefer dark roast coffee in the morning.");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, Kind::Preference);
    }

    #[test]
    fn a_question_is_not_a_statement() {
        assert!(candidates("Do I prefer dark roast coffee?").is_empty());
        assert!(candidates("My sister is Alice?").is_empty());
    }

    #[test]
    fn an_ordinary_request_is_not_captured() {
        assert!(candidates("Can you summarise the quarterly report for me").is_empty());
        assert!(candidates("what time is the standup").is_empty());
    }

    #[test]
    fn a_greeting_is_not_captured() {
        assert!(candidates("hi").is_empty());
        assert!(candidates("thanks!").is_empty());
    }

    #[test]
    fn a_statement_at_the_content_limit_is_captured() {
        let content = format!("alpha beta {}", "a".repeat(189));
        assert_eq!(content.chars().count(), 200);
        let found = candidates(&format!("Remember that {content}"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].content.chars().count(), 200);
    }

    #[test]
    fn an_over_limit_statement_is_not_captured() {
        let content = format!("alpha beta {}", "a".repeat(190));
        assert_eq!(content.chars().count(), 201);
        assert!(candidates(&format!("Remember that {content}")).is_empty());
    }

    #[test]
    fn a_long_message_is_still_scanned_for_a_short_statement() {
        let message = format!(
            "{}.Remember that Alice manages the OR board.",
            "x".repeat(900)
        );
        let found = candidates(&message);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].content, "Alice manages the OR board");
    }

    #[test]
    fn at_most_three_statements_are_taken_from_one_message() {
        let found = candidates(
            "Remember that Alice manages the OR board. Remember that Bob is my boss. \
             I prefer dark roast. Remember that we ship on Fridays.",
        );
        assert_eq!(found.len(), MAX_CANDIDATES);
    }
}
