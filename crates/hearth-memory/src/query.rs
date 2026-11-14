//! Turning a message into a query.
//!
//! Retrieval is only as good as the terms it searches for. A personal message is
//! mostly framing: "what was it we decided about the OR board last month" carries
//! two content terms and eight words of scaffolding. Dropping the scaffolding and
//! weighting what remains is the cheapest recall gain available, and it costs no
//! model call.
//!
//! Three things happen here:
//!
//! * **Scaffolding is removed.** Question words, auxiliaries, and pronouns carry
//!   no topic. They are dropped, with one exception: a capitalised word is kept
//!   even when it is also a stopword, because "the OR board" is a name.
//! * **Entities are found.** Quoted spans, proper-noun runs, identifiers, and any
//!   subject the store already knows are collected. A personal question almost
//!   always names one, and that name is the strongest signal there is.
//! * **Relations are expanded.** "boss" also searches "manager" and "reports to".
//!   A small static table covers the relations personal memory is asked about, so
//!   a paraphrase matches without an embedding or a model call.

use std::collections::HashSet;

/// One searchable term and how much it should count.
#[derive(Debug, Clone, PartialEq)]
pub struct Term {
    pub text: String,
    pub weight: f32,
}

/// A message, analysed for retrieval.
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub raw: String,
    /// Core terms, in message order.
    pub terms: Vec<Term>,
    /// Expanded relatives of the core terms, searched only as a fallback.
    pub expanded: Vec<Term>,
    /// Proper nouns and quoted spans, normalised.
    pub entities: Vec<String>,
    /// Whether the message carries no topic at all.
    pub trivial: bool,
}

/// Words that carry no topic.
const STOPWORDS: &[&str] = &[
    "a",
    "about",
    "after",
    "again",
    "against",
    "all",
    "also",
    "am",
    "an",
    "and",
    "any",
    "anything",
    "are",
    "aren",
    "as",
    "at",
    "be",
    "because",
    "been",
    "before",
    "being",
    "below",
    "between",
    "both",
    "but",
    "by",
    "can",
    "cannot",
    "could",
    "couldn",
    "did",
    "didn",
    "do",
    "does",
    "doesn",
    "doing",
    "don",
    "down",
    "during",
    "each",
    "either",
    "else",
    "even",
    "ever",
    "every",
    "for",
    "from",
    "further",
    "get",
    "got",
    "had",
    "hadn",
    "has",
    "hasn",
    "have",
    "haven",
    "having",
    "he",
    "her",
    "here",
    "hers",
    "herself",
    "him",
    "himself",
    "his",
    "how",
    "however",
    "i",
    "if",
    "in",
    "into",
    "is",
    "isn",
    "it",
    "its",
    "itself",
    "just",
    "know",
    "let",
    "like",
    "me",
    "might",
    "mine",
    "more",
    "most",
    "must",
    "my",
    "myself",
    "need",
    "no",
    "nor",
    "not",
    "now",
    "of",
    "off",
    "on",
    "once",
    "only",
    "or",
    "other",
    "ought",
    "our",
    "ours",
    "ourselves",
    "out",
    "over",
    "own",
    "recall",
    "remember",
    "remind",
    "said",
    "same",
    "say",
    "shall",
    "she",
    "should",
    "shouldn",
    "since",
    "so",
    "some",
    "something",
    "still",
    "such",
    "tell",
    "than",
    "that",
    "the",
    "their",
    "theirs",
    "them",
    "themselves",
    "then",
    "there",
    "these",
    "they",
    "thing",
    "things",
    "this",
    "those",
    "though",
    "through",
    "to",
    "too",
    "under",
    "until",
    "up",
    "us",
    "very",
    "want",
    "was",
    "wasn",
    "we",
    "were",
    "weren",
    "what",
    "when",
    "where",
    "whether",
    "which",
    "while",
    "who",
    "whom",
    "whose",
    "why",
    "will",
    "with",
    "won",
    "would",
    "wouldn",
    "you",
    "your",
    "yours",
    "yourself",
];

/// Utterances that never deserve a search.
const TRIVIAL: &[&str] = &[
    "hi",
    "hello",
    "hey",
    "thanks",
    "thank you",
    "ok",
    "okay",
    "sure",
    "yes",
    "yeah",
    "no",
    "nope",
    "lol",
    "good morning",
    "good afternoon",
    "good evening",
    "good night",
    "bye",
    "cool",
    "nice",
    "great",
    "got it",
];

/// Relations that are asked about by a different word from the one stored.
///
/// A group is unordered: every member expands to the others. It is deliberately
/// small. A large thesaurus invents matches; these forty groups are the relations
/// a personal assistant is actually asked to recall.
const RELATIONS: &[&[&str]] = &[
    &["boss", "manager", "lead", "head", "supervisor", "reports"],
    &["job", "work", "employer", "company", "workplace"],
    &["colleague", "coworker", "teammate", "peer"],
    &["wife", "husband", "spouse", "partner", "married"],
    &["son", "daughter", "child", "kid", "children"],
    &["mother", "mom", "mum", "parent"],
    &["father", "dad", "parent"],
    &["sister", "brother", "sibling"],
    &["friend", "buddy", "mate"],
    &["born", "birthday", "birth"],
    &["lives", "live", "home", "city", "town", "based", "resides"],
    &["moved", "move", "relocated", "relocation"],
    &[
        "prefers",
        "prefer",
        "like",
        "likes",
        "favourite",
        "favorite",
        "enjoy",
    ],
    &["hates", "hate", "dislike", "avoid", "avoids", "detest"],
    &["allergic", "allergy", "intolerance", "intolerant"],
    &["vegetarian", "vegan", "diet", "dietary"],
    &["database", "db", "store"],
    &["language", "lang", "stack"],
    &["deploy", "deployment", "deploys", "shipping", "release"],
    &["repo", "repository", "codebase"],
    &["decided", "decide", "decision", "chose", "chosen", "choice"],
    &["project", "initiative", "effort"],
    &["schedule", "calendar", "agenda"],
    &["meeting", "meet", "sync", "standup"],
    &["deadline", "due", "delivery"],
    &["phone", "mobile", "number"],
    &["email", "mail", "address"],
    &["coffee", "espresso", "caffeine"],
    &["tea", "chai"],
    &["doctor", "physician", "gp"],
    &["car", "vehicle", "drive"],
    &["pet", "dog", "cat"],
    &["apartment", "flat", "house", "home"],
    &["salary", "pay", "compensation"],
    &["age", "aged", "old"],
    &["started", "began", "kickoff"],
    &["finished", "done", "completed", "shipped"],
    &["blocked", "stuck", "waiting"],
    &["bug", "issue", "defect"],
    &["budget", "cost", "spend"],
];

/// One word from the message, with the case it was written in.
struct Token {
    text: String,
    capitalised: bool,
    sentence_start: bool,
}

/// Split a message into words, keeping internal decimal points.
fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut capitalised = false;
    let mut fresh = true;
    let characters: Vec<char> = text.chars().collect();
    let mut index = 0;

    while index < characters.len() {
        let character = characters[index];
        let decimal = character == '.'
            && !current.is_empty()
            && current.chars().all(|inner| inner.is_ascii_digit())
            && characters
                .get(index + 1)
                .is_some_and(|next| next.is_ascii_digit());
        if character.is_alphanumeric() || decimal {
            if current.is_empty() && character.is_uppercase() {
                capitalised = true;
            }
            current.push(character);
        } else if !current.is_empty() {
            tokens.push(Token {
                text: std::mem::take(&mut current),
                capitalised,
                sentence_start: fresh,
            });
            capitalised = false;
            fresh = matches!(character, '.' | '!' | '?' | '\n' | ';');
            index += 1;
            continue;
        } else {
            fresh = matches!(character, '.' | '!' | '?' | '\n' | ';');
        }
        index += 1;
    }
    if !current.is_empty() {
        tokens.push(Token {
            text: current,
            capitalised,
            sentence_start: fresh,
        });
    }
    tokens
}

/// The normalised form of a name or subject.
pub fn normalize(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    let mut space = false;
    for character in text.chars() {
        if character.is_alphanumeric() {
            if space && !folded.is_empty() {
                folded.push(' ');
            }
            space = false;
            folded.extend(character.to_lowercase());
        } else {
            space = true;
        }
    }
    let trimmed = folded.trim();
    // A leading article is noise in a subject key.
    for article in ["the ", "a ", "an "] {
        if let Some(rest) = trimmed.strip_prefix(article) {
            return rest.to_owned();
        }
    }
    trimmed.to_owned()
}

/// The content words of a statement, used to build its index expansion.
pub fn head_words(text: &str) -> Vec<String> {
    tokenize(text)
        .into_iter()
        .filter_map(|token| {
            let lowered = token.text.to_lowercase();
            (!is_stopword(&lowered) && lowered.chars().count() > 1).then_some(lowered)
        })
        .take(24)
        .collect()
}

fn is_stopword(word: &str) -> bool {
    STOPWORDS.contains(&word)
}

/// Words that mean the same relation as `word`.
pub fn related(word: &str) -> impl Iterator<Item = &'static str> {
    let lowered = word.to_lowercase();
    let key = lowered.clone();
    RELATIONS
        .iter()
        .filter(move |group| group.iter().any(|member| *member == key))
        .flat_map(|group| group.iter().copied())
        .filter(move |member| *member != lowered)
}

/// The expansion terms stored beside a statement so a paraphrase can find it.
pub fn expansions(text: &str, subject: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut words = Vec::new();
    for word in head_words(text).into_iter().chain(head_words(subject)) {
        for term in std::iter::once(word.clone()).chain(related(&word).map(str::to_owned)) {
            if seen.insert(term.clone()) {
                words.push(term);
            }
        }
    }
    words
}

/// Analyse a message.
///
/// `known` is the set of subjects the store already holds. It is used to notice
/// that "the board" refers to a subject named "board", and to weight that name
/// above an ordinary word.
pub fn analyze(text: &str, known: &HashSet<String>) -> Query {
    let trimmed = text.trim();
    let mut query = Query {
        raw: trimmed.to_owned(),
        ..Query::default()
    };
    if trimmed.is_empty() || trimmed.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
        query.trivial = true;
        return query;
    }
    let lowered = trimmed.to_lowercase();
    if TRIVIAL.contains(&lowered.as_str()) || TRIVIAL.contains(&normalize(trimmed).as_str()) {
        query.trivial = true;
        return query;
    }

    let tokens = tokenize(trimmed);
    let mut seen = HashSet::new();

    // A quoted span is a name or a phrase the user is repeating on purpose.
    for phrase in quoted_spans(trimmed) {
        let subject = normalize(&phrase);
        if subject.chars().count() >= 2 {
            query.entities.push(subject);
            for term in head_words(&phrase) {
                if seen.insert(term.clone()) {
                    query.terms.push(Term {
                        text: term,
                        weight: 1.6,
                    });
                }
            }
        }
    }

    // Consecutive capitalised words are a name. A sentence's first word is not,
    // because every sentence starts with a capital.
    let mut run: Vec<&Token> = Vec::new();
    let mut runs: Vec<Vec<&Token>> = Vec::new();
    for token in &tokens {
        if token.capitalised && !token.sentence_start {
            run.push(token);
        } else if !run.is_empty() {
            runs.push(std::mem::take(&mut run));
        }
    }
    if !run.is_empty() {
        runs.push(run);
    }
    for run in runs {
        let joined = run
            .iter()
            .map(|token| token.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let entity = normalize(&joined);
        if entity.chars().count() >= 2 {
            query.entities.push(entity);
        }
    }

    // Literal subjects the store already knows. A message can mention one
    // without capitalising it.
    let words: Vec<String> = tokens
        .iter()
        .map(|token| token.text.to_lowercase())
        .collect();
    for subject in known {
        if subject.is_empty() || query.entities.iter().any(|entity| entity == subject) {
            continue;
        }
        let parts: Vec<&str> = subject.split(' ').collect();
        if parts.len() <= words.len() && parts.iter().all(|part| words.iter().any(|w| w == part)) {
            query.entities.push(subject.clone());
        }
    }
    query.entities.sort();
    query.entities.dedup();

    for token in &tokens {
        let word = token.text.to_lowercase();
        let digits = word.chars().any(|c| c.is_ascii_digit());
        let named = token.capitalised && !token.sentence_start;
        if word.chars().count() < 2 && !digits {
            continue;
        }
        if is_stopword(&word) && !named {
            continue;
        }
        if !seen.insert(word.clone()) {
            continue;
        }
        let mut weight = 1.0;
        if named {
            weight += 0.6;
        }
        if word.chars().count() >= 9 {
            weight += 0.3;
        }
        if digits {
            weight += 0.5;
        }
        if known.contains(&word) {
            weight += 1.0;
        }
        query.terms.push(Term { text: word, weight });
    }

    // Relation expansions only ever act as a fallback, so they rank below a term
    // the user actually wrote.
    let mut expanded = HashSet::new();
    for term in &query.terms {
        for relation in related(&term.text) {
            if !seen.contains(relation) && expanded.insert(relation.to_owned()) {
                query.expanded.push(Term {
                    text: relation.to_owned(),
                    weight: 0.4,
                });
            }
        }
    }

    query.trivial = query.terms.is_empty();
    query
}

/// Spans the user wrapped in quotes.
fn quoted_spans(text: &str) -> Vec<String> {
    let mut spans = Vec::new();
    for quote in ['"', '\u{201c}', '\u{2018}'] {
        let closing = match quote {
            '\u{201c}' => '\u{201d}',
            '\u{2018}' => '\u{2019}',
            other => other,
        };
        let mut rest = text;
        while let Some(start) = rest.find(quote) {
            let after = &rest[start + quote.len_utf8()..];
            let Some(end) = after.find(closing) else {
                break;
            };
            let span = after[..end].trim();
            if !span.is_empty() {
                spans.push(span.to_owned());
            }
            rest = &after[end + closing.len_utf8()..];
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> HashSet<String> {
        ["alice", "or board", "project atlas"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn scaffolding_is_dropped() {
        let query = analyze(
            "what was it we decided about the database last month",
            &known(),
        );
        let words: Vec<&str> = query.terms.iter().map(|t| t.text.as_str()).collect();
        assert!(words.contains(&"decided"), "{words:?}");
        assert!(words.contains(&"database"), "{words:?}");
        assert!(!words.contains(&"what"), "{words:?}");
        assert!(!words.contains(&"about"), "{words:?}");
    }

    #[test]
    fn a_capitalised_stopword_stays_because_it_is_a_name() {
        let query = analyze("did we finish the OR board", &known());
        let words: Vec<&str> = query.terms.iter().map(|t| t.text.as_str()).collect();
        assert!(words.contains(&"or"), "{words:?}");
        assert!(query.entities.contains(&"or board".to_owned()));
    }

    #[test]
    fn a_known_subject_is_found_without_capitals() {
        let query = analyze("who runs the or board", &known());
        assert!(query.entities.contains(&"or board".to_owned()), "{query:?}");
    }

    #[test]
    fn a_quoted_span_is_an_entity() {
        let query = analyze("what did \"Project Atlas\" decide", &known());
        assert!(
            query.entities.contains(&"project atlas".to_owned()),
            "{query:?}"
        );
    }

    #[test]
    fn a_relation_is_expanded_below_the_written_terms() {
        let query = analyze("who is bob's boss", &known());
        assert!(query.terms.iter().any(|t| t.text == "bob"));
        assert!(
            query
                .expanded
                .iter()
                .any(|t| t.text == "manager" || t.text == "reports"),
            "{query:?}"
        );
        let lowest_written = query
            .terms
            .iter()
            .map(|term| term.weight)
            .fold(f32::MAX, f32::min);
        let highest_expanded = query
            .expanded
            .iter()
            .map(|term| term.weight)
            .fold(0.0_f32, f32::max);
        assert!(
            highest_expanded < lowest_written,
            "an expanded term must weigh less than a written one"
        );
    }

    #[test]
    fn a_greeting_carries_no_query() {
        for greeting in ["hi", "thanks", "  hello!  ", ""] {
            assert!(analyze(greeting, &known()).trivial, "{greeting:?}");
        }
    }

    #[test]
    fn a_decimal_stays_one_term() {
        let query = analyze("did we pin 1.0 or 1.1", &known());
        let words: Vec<&str> = query.terms.iter().map(|t| t.text.as_str()).collect();
        assert!(words.contains(&"1.0"), "{words:?}");
        assert!(words.contains(&"1.1"), "{words:?}");
    }

    #[test]
    fn a_stored_statement_expands_its_relations() {
        let terms = expansions("Alice manages Bob", "alice");
        assert!(terms.contains(&"alice".to_owned()));
        assert!(terms.contains(&"manages".to_owned()));
    }
}
