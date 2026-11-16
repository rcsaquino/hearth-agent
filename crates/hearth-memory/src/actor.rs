//! Who a statement is about.
//!
//! A memory is read with no conversation around it, so a pronoun has no
//! antecedent. The store refuses one; the extractor rewrites what it can.

/// The person a pronoun refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Person {
    /// The user.
    First,
    /// The assistant.
    Second,
    /// Anyone else.
    Third,
}

/// The first pronoun in a statement, with the person it refers to.
pub fn pronoun(text: &str) -> Option<(&str, Person)> {
    text.split(|character: char| !character.is_alphanumeric())
        .find_map(|word| {
            let lower = word.to_ascii_lowercase();
            person_of(&lower).map(|person| (word, person))
        })
}

fn person_of(word: &str) -> Option<Person> {
    match word {
        "i" | "me" | "my" | "mine" | "myself" | "we" | "us" | "our" | "ours" | "ourselves" => {
            Some(Person::First)
        }
        "you" | "your" | "yours" | "yourself" | "yourselves" => Some(Person::Second),
        "he" | "him" | "his" | "himself" | "she" | "her" | "hers" | "herself" | "they" | "them"
        | "their" | "theirs" | "themselves" | "it" | "its" | "itself" => Some(Person::Third),
        _ => None,
    }
}

/// The refusal for a statement that names no actor.
pub fn refusal(pronoun: &str, person: Person) -> String {
    let hint = match person {
        Person::First => "write \"the user\" instead",
        Person::Second => "name the assistant instead",
        Person::Third => "replace it with the name from the message",
    };
    format!(
        "memory content says \"{pronoun}\"; {hint}, because a memory must read correctly with no \
         conversation around it."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_person_is_found_and_named() {
        for (text, word, person) in [
            ("I prefer dark mode", "I", Person::First),
            ("my sister is Alice", "my", Person::First),
            ("you should deploy", "you", Person::Second),
            ("your branch is behind", "your", Person::Second),
            ("he likes apples", "he", Person::Third),
            ("they shipped it", "they", Person::Third),
            ("it is ready", "it", Person::Third),
        ] {
            assert_eq!(pronoun(text), Some((word, person)), "{text}");
        }
    }

    #[test]
    fn a_named_statement_has_no_pronoun() {
        assert_eq!(pronoun("Alice likes apples"), None);
        assert_eq!(pronoun("the user prefers dark mode"), None);
        assert_eq!(pronoun("Never deploy on Friday"), None);
    }

    #[test]
    fn a_contraction_still_names_its_pronoun() {
        assert_eq!(pronoun("I'm ready"), Some(("I", Person::First)));
        assert_eq!(pronoun("you're late"), Some(("you", Person::Second)));
        assert_eq!(pronoun("it's done"), Some(("it", Person::Third)));
    }

    #[test]
    fn a_pronoun_substring_is_not_a_pronoun() {
        assert_eq!(pronoun("this item is in the theme"), None);
        assert_eq!(pronoun("the shipment is ready"), None);
    }

    #[test]
    fn the_refusal_names_the_pronoun_and_the_hint() {
        let message = refusal("he", Person::Third);
        assert!(message.contains("\"he\""), "{message}");
        assert!(message.contains("name from the message"), "{message}");
        assert!(message.contains("no conversation"), "{message}");
        let first = refusal("I", Person::First);
        assert!(first.contains("the user"), "{first}");
    }
}
