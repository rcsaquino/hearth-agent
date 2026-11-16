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
    let _ = text;
    None
}

/// The refusal for a statement that names no actor.
pub fn refusal(pronoun: &str, person: Person) -> String {
    let _ = (pronoun, person);
    String::new()
}
