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

/// Adverbs that may sit between the subject and the verb.
const ADVERBS: &[&str] = &["always", "never", "usually", "often", "sometimes", "rarely"];

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
    for sentence in crate::text::sentences(&message) {
        let sentence = sentence.trim();
        if found.len() >= MAX_CANDIDATES {
            break;
        }
        if sentence.ends_with('?') {
            continue;
        }
        if let Some(candidate) = from_remember(sentence)
            .or_else(|| from_preference(sentence))
            .or_else(|| from_relation(sentence))
            .or_else(|| from_possessive(sentence))
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
    let content = rewrite_first_person(&trim_end(&sentence[phrase.len()..]));
    if content.is_empty() {
        return None;
    }
    // The rewrite names its actor, so the subject is the same one a stated
    // preference uses. A name the rewrite leaves in place is still the subject.
    let subject = if content.to_ascii_lowercase().starts_with("the user") {
        "user".to_owned()
    } else {
        leading_subject(&content)
    };
    Some(Candidate {
        subject,
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
    // Both word orders of a relation compose the same statement, so the
    // extractor writes one order and the store can recognise the other.
    let content = if relation {
        format!("{} is the user's {}", trim_end(value), trim_end(noun))
    } else {
        format!("the user's {}", trim_end(rest))
    };
    Some(Candidate {
        subject: if relation {
            leading_subject(value)
        } else {
            normalize(noun)
        },
        content,
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
        content: trim_end(&format!("{} is the user's {}", left.trim(), right.trim())),
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
    let lowered = sentence.to_ascii_lowercase();
    let content = if lowered.starts_with("my favourite ") {
        format!(
            "the user's favourite {}",
            trim_end(&sentence["my favourite ".len()..])
        )
    } else if lowered.starts_with("my favorite ") {
        format!(
            "the user's favorite {}",
            trim_end(&sentence["my favorite ".len()..])
        )
    } else if lowered.starts_with("from now on ") {
        trim_end(sentence)
    } else {
        let phrase = PREFERENCE
            .iter()
            .find(|phrase| lowered.starts_with(**phrase))?;
        let rest = trim_end(&sentence[phrase.len()..]);
        let clause = match *phrase {
            "i prefer " => format!("prefers {rest}"),
            "i hate " => format!("hates {rest}"),
            "i love " => format!("loves {rest}"),
            "i always " | "i never " | "i usually " => {
                let adverb = phrase.trim_start_matches("i ").trim();
                let mut parts = rest.splitn(2, ' ');
                let verb = third_person(parts.next().unwrap_or(""));
                let tail = parts.next().unwrap_or("");
                if tail.is_empty() {
                    format!("{adverb} {verb}")
                } else {
                    format!("{adverb} {verb} {tail}")
                }
            }
            _ => return None,
        };
        format!("the user {clause}")
    };
    Some(Candidate {
        subject: "user".to_owned(),
        content: trim_end(&content),
        kind: Kind::Preference,
    })
}

/// Rewrite a statement whose actor is the user.
///
/// First person has a determinate meaning without the message, so the extractor
/// can name it. A pronoun the rewrite does not cover is stored as written.
fn rewrite_first_person(text: &str) -> String {
    let lowered = text.to_ascii_lowercase();
    if lowered.starts_with("i ") {
        return format!("the user {}", third_person_rest(&text[2..]));
    }
    if lowered.starts_with("we ") {
        return format!("the user {}", third_person_rest(&text[3..]));
    }
    let mut rewritten = text.to_owned();
    for (from, to) in [
        ("my", "the user's"),
        ("our", "the user's"),
        ("mine", "the user's"),
        ("ours", "the user's"),
        ("me", "the user"),
        ("us", "the user"),
    ] {
        rewritten = replace_word(&rewritten, from, to);
    }
    rewritten
}

/// The rest of a third-person clause, with its verb agreed to `the user`.
fn third_person_rest(rest: &str) -> String {
    let mut parts = rest.splitn(2, ' ');
    let first = parts.next().unwrap_or("");
    let tail = parts.next().unwrap_or("");
    if ADVERBS.contains(&first.to_ascii_lowercase().as_str()) {
        let mut inner = tail.splitn(2, ' ');
        let verb = inner.next().unwrap_or("");
        let rest = inner.next().unwrap_or("");
        let verb = third_person(verb);
        return if rest.is_empty() {
            format!("{first} {verb}")
        } else {
            format!("{first} {verb} {rest}")
        };
    }
    let verb = third_person(first);
    if tail.is_empty() {
        verb
    } else {
        format!("{verb} {tail}")
    }
}

/// The third-person singular form of an English verb.
fn third_person(verb: &str) -> String {
    let lower = verb.to_ascii_lowercase();
    match lower.as_str() {
        "have" => return "has".to_owned(),
        "be" | "am" | "are" => return "is".to_owned(),
        "do" => return "does".to_owned(),
        "go" => return "goes".to_owned(),
        _ => {}
    }
    if lower.ends_with('y')
        && !matches!(
            lower.chars().rev().nth(1),
            Some('a' | 'e' | 'i' | 'o' | 'u')
        )
    {
        return format!("{}ies", &verb[..verb.len() - 1]);
    }
    if ["s", "x", "z", "ch", "sh", "o"]
        .iter()
        .any(|ending| lower.ends_with(ending))
    {
        return format!("{verb}es");
    }
    format!("{verb}s")
}

/// Replace a whole word, keeping the punctuation that follows it.
fn replace_word(text: &str, from: &str, to: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            let (core, suffix) = split_suffix(word);
            if core.eq_ignore_ascii_case(from) {
                format!("{to}{suffix}")
            } else {
                word.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn split_suffix(word: &str) -> (&str, &str) {
    let end = word
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_alphanumeric())
        .map(|(index, character)| index + character.len_utf8())
        .unwrap_or(0);
    (&word[..end], &word[end..])
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
        assert_eq!(found[0].content, "Alice is the user's sister");
    }

    #[test]
    fn both_relation_orders_produce_the_same_candidate() {
        let possessive = candidates("My sister is Alice.");
        let relation = candidates("Alice is my sister.");
        assert_eq!(possessive.len(), 1);
        assert_eq!(relation.len(), 1);
        assert_eq!(possessive[0], relation[0]);
    }

    #[test]
    fn a_remembered_preference_names_the_same_subject_as_a_stated_one() {
        let stated = candidates("I prefer dark mode.");
        let remembered = candidates("Remember that I prefer dark mode.");
        assert_eq!(stated.len(), 1);
        assert_eq!(remembered.len(), 1);
        assert_eq!(stated[0].content, remembered[0].content);
        assert_eq!(stated[0].subject, remembered[0].subject);
    }

    #[test]
    fn a_favourite_is_a_preference_not_a_fact() {
        let found = candidates("My favorite color is blue.");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, Kind::Preference);
        assert_eq!(found[0].subject, "user");
        assert_eq!(found[0].content, "the user's favorite color is blue");
    }

    #[test]
    fn a_relation_statement_about_a_name_is_a_person() {
        let found = candidates("Bob is my boss.");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].subject, "bob");
        assert_eq!(found[0].kind, Kind::Person);
        assert_eq!(found[0].content, "Bob is the user's boss");
    }

    #[test]
    fn a_stated_preference_is_captured() {
        let found = candidates("I prefer dark roast coffee in the morning.");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, Kind::Preference);
        assert_eq!(
            found[0].content,
            "the user prefers dark roast coffee in the morning"
        );
        assert_eq!(found[0].subject, "user");
    }

    #[test]
    fn a_first_person_preference_names_the_user() {
        let found = candidates("I prefer dark mode.");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].content, "the user prefers dark mode");
        assert_eq!(found[0].subject, "user");
    }

    #[test]
    fn a_third_person_statement_without_a_name_is_kept() {
        let found = candidates("Remember that he likes apples.");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].content, "he likes apples");
        assert_eq!(
            found[0].subject, "",
            "a pronoun invents no subject of its own"
        );
    }

    #[test]
    fn a_named_third_person_statement_keeps_the_name() {
        let found = candidates("Remember that Alice likes apples.");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].content, "Alice likes apples");
        assert_eq!(found[0].subject, "alice");
    }

    #[test]
    fn a_first_person_remember_statement_names_the_user() {
        let found = candidates("Remember that we ship on Fridays.");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].content, "the user ships on Fridays");
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
