//! Splitting text into statements.
//!
//! One conservative splitter, used by the write gate. A write is refused when it
//! holds more than one statement, so the
//! splitter must not turn a title, an abbreviation, a decimal, or an ellipsis
//! into a boundary. It also must not guess at a lowercase continuation, because
//! a false split would refuse a legitimate single statement.

/// Titles and other words whose full stop does not end a sentence.
const ABBREVIATIONS: &[&str] = &[
    "al", "approx", "apr", "aug", "ave", "blvd", "co", "corp", "dec", "dept", "dr", "e.g", "est",
    "etc", "feb", "fig", "fri", "i.e", "inc", "jan", "jr", "jul", "jun", "ltd", "mar", "mon", "mr",
    "mrs", "ms", "nov", "oct", "prof", "sat", "sep", "sept", "sr", "st", "sun", "thu", "thur",
    "thurs", "tue", "tues", "u.s", "vs", "wed",
];

/// The statements in one text.
pub(crate) fn sentences(text: &str) -> Vec<&str> {
    let mut spans = Vec::new();
    let mut start = 0;
    for (index, character) in text.char_indices() {
        if !boundary(text, index, character) {
            continue;
        }
        let end = index + character.len_utf8();
        let span = &text[start..end];
        if !span.trim().is_empty() {
            spans.push(span);
        }
        start = end;
    }
    let rest = &text[start..];
    if !rest.trim().is_empty() {
        spans.push(rest);
    }
    spans
}

/// How many statements one text holds.
pub(crate) fn statement_count(text: &str) -> usize {
    sentences(text).len()
}

fn boundary(text: &str, index: usize, character: char) -> bool {
    let before = text[..index].chars().next_back();
    match character {
        '\n' => true,
        '!' | '?' => {
            let after = text[index + character.len_utf8()..]
                .chars()
                .next()
                .is_some_and(|next| !matches!(next, '.' | '!' | '?'));
            after && !matches!(before, Some('.' | '!' | '?'))
        }
        '.' => {
            if matches!(before, Some('.')) {
                return false;
            }
            let next = text[index + 1..].chars().find(|next| !next.is_whitespace());
            if !next.is_some_and(char::is_uppercase) {
                return false;
            }
            let previous = text[..index].split_whitespace().last().unwrap_or("");
            let word = previous
                .trim_matches(|character: char| !character.is_alphanumeric())
                .to_lowercase();
            !(word.chars().count() == 1 || ABBREVIATIONS.contains(&word.as_str()))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_capitalised_sentences_are_two_statements() {
        assert_eq!(
            statement_count("User likes apples. He likes to bring it to work."),
            2
        );
    }

    #[test]
    fn an_abbreviation_is_not_a_boundary() {
        assert_eq!(statement_count("Dr. Smith likes apples."), 1);
        assert_eq!(statement_count("the user prefers e.g. dark mode."), 1);
        assert_eq!(statement_count("the meeting is at 1.5 p.m."), 1);
        assert_eq!(statement_count("Project Atlas, Inc. ships on Friday."), 1);
    }

    #[test]
    fn a_lowercase_continuation_is_not_a_boundary() {
        assert_eq!(statement_count("the user prefers node.js for tools."), 1);
        assert_eq!(statement_count("one thought. then another."), 1);
    }

    #[test]
    fn repeated_punctuation_is_one_statement() {
        assert_eq!(statement_count("What?! Really."), 1);
        assert_eq!(statement_count("Wait... Really?"), 1);
    }

    #[test]
    fn a_newline_is_a_boundary() {
        assert_eq!(statement_count("one statement\ntwo statement"), 2);
    }
}
