//! The rows.
//!
//! All memory SQL lives here so the rest of the crate reasons about `Memory`
//! values. Reads go through the shared reader pool; writes go through the one
//! writer, which is why a memory can never be half-saved and why provenance can
//! point at the entry that produced it.
//!
//! The write path is deliberately narrow:
//!
//! * Exact duplicates are refused and the existing row is returned.
//! * A near duplicate about the same subject is refused the same way, which is
//!   what stops eight weeks of "ordered pad thai" from becoming eight rows.
//! * `supersedes` closes the old row's validity instead of overwriting it, so the
//!   store keeps history and the ranking can prefer what is true now.
//! * A credential-shaped string never reaches the table.

use std::collections::HashSet;

use hearth_core::Session;
use rusqlite::Row;
use sha2::{Digest, Sha256};

use crate::errors::{Code, MemoryError, Result};
use crate::model::{Kind, Memory, NewMemory, Source, WriteOutcome};
use crate::query::normalize;

/// The longest statement a write may store.
pub const CONTENT_LIMIT: usize = 200;
/// The rows the prompt injects as rules.
///
/// A pin belongs to a live statement, and retirement clears it. The validity
/// check stays because injection must not trust a flag an older store left
/// behind.
const LIVE_RULES: &str = "pinned = 1 AND valid_to IS NULL";

/// The alphabet for a generated id.
///
/// Crockford base32 drops `I`, `L`, `O`, and `U`, so a copied or retyped id is
/// less likely to be wrong.
const ID_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
/// The characters after the `m_` prefix.
const ID_CHARS: usize = 10;
/// How many taken ids one write retries before it fails.
const ID_ATTEMPTS: usize = 8;

fn new_id() -> String {
    let bytes = uuid::Uuid::new_v4();
    let mut value = 0u64;
    for byte in &bytes.as_bytes()[..7] {
        value = (value << 8) | u64::from(*byte);
    }
    let mut id = String::with_capacity(2 + ID_CHARS);
    id.push_str("m_");
    for index in (0..ID_CHARS).rev() {
        let slot = ((value >> (index * 5)) & 0x1f) as usize;
        id.push(ID_ALPHABET[slot] as char);
    }
    id
}

/// The canonical form of an id for lookup.
///
/// The tool trims its input, and this accepts either case so a copied id is not
/// rejected for its case. It is an exact id, never a prefix.
pub fn normalize_id(id: &str) -> String {
    id.trim().to_ascii_lowercase()
}

/// The longest subject a write may store.
pub const SUBJECT_LIMIT: usize = 120;
/// How similar two statements about one subject must be to count as the same.
const MERGE_SIMILARITY: f32 = 0.85;
/// How many same-subject rows a write compares against.
const MERGE_WINDOW: i64 = 64;

const COLUMNS: &str = "id, kind, content, subject, pinned, source_kind, source_entry, \
     valid_from, valid_to, superseded_by, created_at, updated_at, revision";

/// The same columns, qualified for a query that joins the index.
const COLUMNS_OF_M: &str = "m.id, m.kind, m.content, m.subject, m.pinned, m.source_kind, \
     m.source_entry, m.valid_from, m.valid_to, m.superseded_by, m.created_at, m.updated_at, \
     m.revision";

/// A vector written beside its memory.
#[derive(Debug, Clone)]
pub struct VectorWrite {
    pub model: String,
    pub dims: usize,
    pub values: Vec<f32>,
}

/// One vector row loaded for scoring.
#[derive(Debug, Clone)]
pub struct VectorRow {
    pub memory_id: String,
    pub current: bool,
    pub quantised: Vec<i8>,
}

/// A statement ready to be written.
#[derive(Debug, Clone)]
pub struct Pending {
    pub item: NewMemory,
    pub vector: Option<VectorWrite>,
}

fn map_memory(row: &Row<'_>) -> rusqlite::Result<Memory> {
    let kind: String = row.get(1)?;
    let source: String = row.get(5)?;
    let pinned: i64 = row.get(4)?;
    Ok(Memory {
        id: row.get(0)?,
        kind: kind.parse().unwrap_or(Kind::Fact),
        content: row.get(2)?,
        subject: row.get(3)?,
        pinned: pinned != 0,
        source: Source::parse(&source),
        source_entry: row.get(6)?,
        valid_from: row.get(7)?,
        valid_to: row.get(8)?,
        superseded_by: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        revision: row.get(12)?,
    })
}

/// The validity predicate for one alias, without a leading `AND`.
///
/// A row is readable at an instant when it had already started and had not yet
/// been closed. A plain read is the same predicate evaluated at now, so there is
/// one definition of "current" rather than two that can drift: a statement dated
/// in the future does not answer a question asked today.
fn valid_clause(alias: &str) -> String {
    format!("{alias}.valid_from <= ? AND ({alias}.valid_to IS NULL OR {alias}.valid_to > ?)")
}

/// The instant a read uses when the caller did not name one.
fn instant(session: &Session, at: Option<i64>) -> i64 {
    at.unwrap_or_else(|| session.now_micros())
}

/// Content tokens used for duplicate comparison.
///
/// A single digit is kept even though a single letter is dropped, so `1.0` and
/// `1.1` stay distinct statements.
fn content_tokens(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric() && character != '.')
        .filter(|word| {
            word.chars().count() > 1
                || (word.len() == 1 && word.starts_with(|c: char| c.is_ascii_digit()))
        })
        .map(fold_token)
        .collect()
}

/// Fold the spelling and synonym variants that name the same thing.
///
/// The fold is for comparison only. Stored content and the fingerprint keep the
/// words the caller wrote, so history is never rewritten.
fn fold_token(word: &str) -> String {
    let folded = match word {
        "favourite" | "favorite" => "favorite",
        "favourites" | "favorites" => "favorites",
        "colour" | "color" => "color",
        "colours" | "colors" => "colors",
        "behaviour" | "behavior" => "behavior",
        "organise" | "organize" => "organize",
        "organised" | "organized" => "organized",
        "prefer" | "prefers" | "preferred" | "like" | "likes" | "liked" | "love" | "loves"
        | "loved" => "prefer",
        _ => word,
    };
    folded.to_owned()
}

/// Whether a statement says what it does not do.
///
/// A negation is the opposite statement, not a duplicate. The check runs on the
/// words, so it still sees a contraction the token split would break apart.
fn negated(text: &str) -> bool {
    let lowered = text.to_lowercase();
    if lowered.contains("n't") {
        return true;
    }
    lowered
        .split(|character: char| !character.is_alphanumeric())
        .any(|word| {
            matches!(
                word,
                "not"
                    | "no"
                    | "never"
                    | "none"
                    | "nothing"
                    | "neither"
                    | "nor"
                    | "without"
                    | "cannot"
            )
        })
}

/// The Jaccard similarity of two statements.
pub fn similarity(left: &str, right: &str) -> f32 {
    let left = content_tokens(left);
    let right = content_tokens(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let shared = left.intersection(&right).count() as f32;
    let union = left.union(&right).count() as f32;
    shared / union
}

/// Whether a new statement says the same thing as an existing one.
///
/// Jaccard alone misses a statement that merely adds a word, which is exactly what
/// "ordered pad thai on friday again" is. Containment covers that case without
/// merging a statement that swaps its subject, which is how a contradiction looks.
fn same_statement(left: &str, right: &str) -> bool {
    if negated(left) != negated(right) {
        return false;
    }
    let left = content_tokens(left);
    let right = content_tokens(right);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    let shared = left.intersection(&right).count() as f32;
    if shared < 3.0 {
        return false;
    }
    let union = left.union(&right).count() as f32;
    let smallest = left.len().min(right.len()) as f32;
    shared / union >= MERGE_SIMILARITY || shared / smallest >= 0.9
}

/// The stable digest of a normalised statement.
pub fn fingerprint(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalize(content).as_bytes());
    let digest = hasher.finalize();
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Validate a statement before it reaches the writer.
pub fn validate(item: &NewMemory) -> Result<()> {
    let content = item.content.trim();
    if content.is_empty() {
        return Err(MemoryError::invalid("memory content cannot be empty."));
    }
    let length = content.chars().count();
    if length > CONTENT_LIMIT {
        return Err(MemoryError::invalid(format!(
            "memory content is {length} characters; the limit is {CONTENT_LIMIT}. \
             Split it into separate memories."
        )));
    }
    if content.contains('\0') {
        return Err(MemoryError::invalid("memory content cannot contain NUL."));
    }
    if item.subject.chars().count() > SUBJECT_LIMIT {
        return Err(MemoryError::invalid(format!(
            "memory subject must be at most {SUBJECT_LIMIT} characters."
        )));
    }
    if item.pinned && item.kind != Kind::Rule {
        return Err(MemoryError::invalid(
            "only a rule can be pinned, because only a rule is injected every turn.",
        ));
    }
    if item.pinned && item.valid_to.is_some() {
        return Err(MemoryError::invalid(
            "a statement that is written already retired cannot be pinned.",
        ));
    }
    if hearth_core::secrets::contains_secret(content) {
        return Err(MemoryError::new(
            Code::Credential,
            "credential-like memory content was refused.",
        ));
    }
    Ok(())
}

/// Refuse a rule that would overflow the rules budget.
fn check_rules_budget(
    writer: &hearth_storage::Writer<'_>,
    content: &str,
    replaces: Option<&str>,
    budget: u64,
) -> Result<()> {
    let existing = writer.query(
        &format!("SELECT {COLUMNS} FROM memories WHERE {LIVE_RULES} ORDER BY created_at, id"),
        [],
        map_memory,
    )?;
    // The row this write replaces stops being live in the same transaction, so
    // its cost is not part of the space the new rule needs.
    let counted: Vec<&crate::model::Memory> = existing
        .iter()
        .filter(|rule| Some(rule.id.as_str()) != replaces)
        .collect();
    if crate::digest::rules_fit(&counted, content, budget) {
        return Ok(());
    }
    let used = crate::digest::rules_cost(&counted);
    let listed: Vec<String> = counted.iter().map(|rule| rule.id.clone()).collect();
    Err(MemoryError::new(
        Code::RulesFull,
        format!(
            "the rules budget is full at {used} of {budget} tokens; replace or forget one of {}.",
            listed.join(", ")
        ),
    ))
}

/// Every rule the prompt injects, oldest first.
pub fn rules(session: &Session) -> Result<Vec<Memory>> {
    session
        .storage()
        .read(|reader| {
            reader.query(
                &format!(
                    "SELECT {COLUMNS} FROM memories WHERE {LIVE_RULES} ORDER BY created_at, id"
                ),
                [],
                map_memory,
            )
        })
        .map_err(Into::into)
}

/// One memory by id.
pub fn get(session: &Session, id: &str) -> Result<Option<Memory>> {
    let id = normalize_id(id);
    session
        .storage()
        .read(|reader| {
            reader.query_optional(
                &format!("SELECT {COLUMNS} FROM memories WHERE id = ?"),
                [id.as_str()],
                map_memory,
            )
        })
        .map_err(Into::into)
}

/// Every current memory about a subject.
pub fn by_subject(
    session: &Session,
    subject: &str,
    at: Option<i64>,
    kind: Option<Kind>,
    limit: i64,
) -> Result<Vec<Memory>> {
    let filter = valid_clause("m");
    let observed = instant(session, at);
    let kind = kind.map(Kind::as_str);
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(subject.to_owned())];
    params.push(Box::new(observed));
    params.push(Box::new(observed));
    params.push(Box::new(kind));
    params.push(Box::new(kind));
    params.push(Box::new(limit.max(1)));
    session
        .storage()
        .read(|reader| {
            reader.query(
                &format!(
                    "SELECT {COLUMNS} FROM memories m WHERE m.subject = ? AND {filter} \
                     AND (? IS NULL OR m.kind = ?) ORDER BY m.created_at DESC LIMIT ?"
                ),
                rusqlite::params_from_iter(params.iter().map(|value| value.as_ref())),
                map_memory,
            )
        })
        .map_err(Into::into)
}

/// Current memories whose subject starts with a prefix.
///
/// A message that says "Alice" should reach the subject "alice smith". The
/// comparison is a `LIKE` prefix over a binary-collated index, so SQLite reads the
/// index for the prefix but still filters each candidate row.
pub fn by_subject_prefix(
    session: &Session,
    prefix: &str,
    at: Option<i64>,
    limit: i64,
) -> Result<Vec<Memory>> {
    let filter = valid_clause("m");
    let observed = instant(session, at);
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(format!("{prefix}%"))];
    params.push(Box::new(observed));
    params.push(Box::new(observed));
    params.push(Box::new(limit.max(1)));
    session
        .storage()
        .read(|reader| {
            reader.query(
                &format!(
                    "SELECT {COLUMNS} FROM memories m WHERE m.subject LIKE ? AND {filter} \
                     ORDER BY m.created_at DESC LIMIT ?"
                ),
                rusqlite::params_from_iter(params.iter().map(|value| value.as_ref())),
                map_memory,
            )
        })
        .map_err(Into::into)
}

/// Memories by id, in one query.
/// Build the cache row for a vector that was just written.
pub(crate) fn vector_row(
    memory_id: &str,
    vector: &VectorWrite,
    current: bool,
) -> Result<VectorRow> {
    Ok(VectorRow {
        memory_id: memory_id.to_owned(),
        current,
        quantised: quantise(&vector.values, vector.dims)?,
    })
}

pub fn by_ids(session: &Session, ids: &[String]) -> Result<Vec<Memory>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    session
        .storage()
        .read(|reader| {
            reader.query(
                &format!(
                    "SELECT {COLUMNS} FROM memories WHERE valid_to IS NULL AND id IN ({placeholders})"
                ),
                rusqlite::params_from_iter(ids.iter()),
                map_memory,
            )
        })
        .map_err(Into::into)
}

/// How many memories exist about a subject, valid at an instant.
pub fn subject_count(session: &Session, subject: &str, at: Option<i64>) -> Result<usize> {
    let filter = valid_clause("m");
    let observed = instant(session, at);
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(subject.to_owned())];
    params.push(Box::new(observed));
    params.push(Box::new(observed));
    session
        .storage()
        .read(|reader| {
            reader.query_row(
                &format!("SELECT COUNT(*) FROM memories m WHERE m.subject = ? AND {filter}"),
                rusqlite::params_from_iter(params.iter().map(|value| value.as_ref())),
                |row| row.get::<_, i64>(0),
            )
        })
        .map(|count| count as usize)
        .map_err(Into::into)
}

/// How many earlier versions exist about a subject.
pub fn historical_count(session: &Session, subject: &str) -> Result<usize> {
    session
        .storage()
        .read(|reader| {
            reader.query_row(
                "SELECT COUNT(*) FROM memories WHERE subject = ? AND valid_to IS NOT NULL",
                [subject],
                |row| row.get::<_, i64>(0),
            )
        })
        .map(|count| count as usize)
        .map_err(Into::into)
}

/// Lexical matches, best first.
pub fn lexical(
    session: &Session,
    expression: &str,
    at: Option<i64>,
    kind: Option<Kind>,
    limit: i64,
) -> Result<Vec<Memory>> {
    let filter = valid_clause("m");
    let observed = instant(session, at);
    let kind = kind.map(Kind::as_str);
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(expression.to_owned())];
    params.push(Box::new(observed));
    params.push(Box::new(observed));
    params.push(Box::new(kind));
    params.push(Box::new(kind));
    params.push(Box::new(limit.max(1)));
    session
        .storage()
        .read(|reader| {
            reader.query(
                &format!(
                    "SELECT {COLUMNS_OF_M} FROM memories_fts f JOIN memories m ON m.rowid = f.rowid \
                     WHERE memories_fts MATCH ? AND {filter} AND (? IS NULL OR m.kind = ?) \
                     ORDER BY bm25(memories_fts) LIMIT ?"
                ),
                rusqlite::params_from_iter(params.iter().map(|value| value.as_ref())),
                map_memory,
            )
        })
        .map_err(|error| match error {
            hearth_storage::StorageError::Sqlite(inner) => {
                MemoryError::unavailable(format!("memory index query failed: {inner}"))
            }
            other => other.into(),
        })
}

/// Recent memories, newest first.
pub fn recent(
    session: &Session,
    at: Option<i64>,
    kind: Option<Kind>,
    limit: i64,
) -> Result<Vec<Memory>> {
    let filter = valid_clause("m");
    let observed = instant(session, at);
    let kind = kind.map(Kind::as_str);
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    params.push(Box::new(observed));
    params.push(Box::new(observed));
    params.push(Box::new(kind));
    params.push(Box::new(kind));
    params.push(Box::new(limit.max(1)));
    session
        .storage()
        .read(|reader| {
            reader.query(
                &format!(
                    "SELECT {COLUMNS} FROM memories m WHERE {filter} \
                     AND (? IS NULL OR m.kind = ?) \
                     ORDER BY m.updated_at DESC, m.id LIMIT ?"
                ),
                rusqlite::params_from_iter(params.iter().map(|value| value.as_ref())),
                map_memory,
            )
        })
        .map_err(Into::into)
}

/// The most distinct subjects the recall path will consider.
///
/// The subject list is a supplement to the lexical channel: it lets a lowercase
/// mention match a known name. A store with more names than this keeps its
/// longest, most specific ones; a statement about a dropped name is still found
/// by its words. The bound keeps the per-turn scan flat as the store grows.
pub const MAX_KNOWN_SUBJECTS: i64 = 4_096;

/// Every subject the store holds, longest first, up to the recall bound.
pub fn subjects(session: &Session) -> Result<Vec<String>> {
    subjects_limited(session, MAX_KNOWN_SUBJECTS)
}

fn subjects_limited(session: &Session, limit: i64) -> Result<Vec<String>> {
    session
        .storage()
        .read(|reader| {
            reader.query(
                "SELECT DISTINCT subject FROM memories WHERE subject <> '' \
                 ORDER BY LENGTH(subject) DESC, subject LIMIT ?",
                [limit.max(1)],
                |row| row.get::<_, String>(0),
            )
        })
        .map_err(Into::into)
}

/// The total number of current memories.
pub fn count(session: &Session) -> Result<usize> {
    let observed = session.now_micros();
    session
        .storage()
        .read(|reader| {
            reader.query_row(
                &format!(
                    "SELECT COUNT(*) FROM memories m WHERE {}",
                    valid_clause("m")
                ),
                rusqlite::params![observed, observed],
                |row| row.get::<_, i64>(0),
            )
        })
        .map(|count| count as usize)
        .map_err(Into::into)
}

/// Write one statement.
///
/// `rules_budget` is the token budget for pinned rules. It is read and compared
/// inside the write transaction, so two writes racing to pin a rule cannot both
/// pass the check.
pub fn insert(session: &Session, pending: Pending, rules_budget: u64) -> Result<WriteOutcome> {
    let now = session.now_micros();
    session
        .storage()
        .write(move |writer| insert_in(writer, pending, now, rules_budget))
}

/// Write several statements in one transaction.
///
/// Used by the benchmark and by any bulk import. An ordinary turn writes one
/// statement at a time, so the single-writer path stays the common one.
pub fn insert_many(
    session: &Session,
    items: Vec<Pending>,
    rules_budget: u64,
) -> Result<Vec<WriteOutcome>> {
    let now = session.now_micros();
    session.storage().write(move |writer| {
        let mut outcomes = Vec::with_capacity(items.len());
        for pending in items {
            outcomes.push(insert_in(writer, pending, now, rules_budget)?);
        }
        Ok::<Vec<WriteOutcome>, MemoryError>(outcomes)
    })
}

/// Whether an extract would replace a stronger row with a weaker one.
fn weaker_than(item: &NewMemory, earlier: &Memory) -> bool {
    if earlier.pinned {
        return true;
    }
    if !earlier.subject.is_empty() && item.subject.is_empty() {
        return true;
    }
    earlier.kind != Kind::Fact && item.kind == Kind::Fact
}

fn insert_in(
    writer: &hearth_storage::Writer<'_>,
    pending: Pending,
    now: i64,
    rules_budget: u64,
) -> Result<WriteOutcome> {
    let mut generate = new_id;
    insert_with(writer, pending, now, rules_budget, &mut generate)
}

fn insert_with(
    writer: &hearth_storage::Writer<'_>,
    pending: Pending,
    now: i64,
    rules_budget: u64,
    ids: &mut dyn FnMut() -> String,
) -> Result<WriteOutcome> {
    let Pending { mut item, vector } = pending;
    validate(&item)?;
    item.supersedes = item.supersedes.map(|id| normalize_id(&id));
    if item.pinned {
        check_rules_budget(
            writer,
            &item.content,
            item.supersedes.as_deref(),
            rules_budget,
        )?;
    }
    item.content = item.content.trim().to_owned();
    item.subject = normalize(&item.subject);
    let digest = fingerprint(&item.content);
    let terms = crate::query::expansions(&item.content, &item.subject).join(" ");
    let valid_from = item.valid_from.unwrap_or(now);
    let valid_to = item.valid_to;
    if valid_to.is_some_and(|end| end <= valid_from) {
        return Err(MemoryError::invalid(
            "a memory cannot stop being true before it starts.",
        ));
    }
    let supersedes = item.supersedes.clone();

    // An exact duplicate with the same kind and the same pinning is the same
    // statement; nothing to write. The same words in a different role are not:
    // a fact the extractor caught and then a rule the model pinned are different
    // statements, and returning the fact would leave the rule unpinned.
    if let Some(existing) = writer.query_optional(
        &format!(
            "SELECT {COLUMNS} FROM memories WHERE fingerprint = ? AND valid_to IS NULL \
             AND kind = ? AND pinned = ? LIMIT 1"
        ),
        rusqlite::params![digest, item.kind.as_str(), i64::from(item.pinned)],
        map_memory,
    )? {
        return Ok(WriteOutcome::Unchanged(existing));
    }

    // The same words in another role replace the earlier row, so the statement
    // is current once rather than twice.
    let earlier = if supersedes.is_none() {
        writer.query_optional(
            &format!(
                "SELECT {COLUMNS} FROM memories WHERE fingerprint = ? AND valid_to IS NULL LIMIT 1"
            ),
            [&digest],
            map_memory,
        )?
    } else {
        None
    };

    // The pin is why a rule is injected, so an unpinned copy of it is a no-op:
    // the rule stands and the copy is not written. Retiring it takes a write
    // that names the row.
    if let Some(current) = &earlier {
        if current.pinned && !item.pinned {
            return Ok(WriteOutcome::Unchanged(current.clone()));
        }
        // The extractor writes what it heard. It cannot turn a stronger row into
        // a weaker one, or the better subject and kind would be lost.
        if item.source == Source::Extract && weaker_than(&item, current) {
            return Ok(WriteOutcome::Unchanged(current.clone()));
        }
    }

    // Replacement is deliberate, so it skips the duplicate merge: a caller that
    // names the row it replaces has already decided.
    if supersedes.is_none() && earlier.is_none() {
        let mut candidates = Vec::new();
        if item.subject.is_empty() {
            candidates.extend(writer.query(
                &format!(
                    "SELECT {COLUMNS} FROM memories WHERE valid_to IS NULL \
                     ORDER BY updated_at DESC LIMIT {MERGE_WINDOW}"
                ),
                [],
                map_memory,
            )?);
        } else {
            // A subject is indexed, so every row about it is visible however old
            // it is. A subjectless row is not, so those are read newest first
            // within the window.
            candidates.extend(writer.query(
                &format!(
                    "SELECT {COLUMNS} FROM memories WHERE subject = ? AND valid_to IS NULL \
                     ORDER BY updated_at DESC"
                ),
                [&item.subject],
                map_memory,
            )?);
            candidates.extend(writer.query(
                &format!(
                    "SELECT {COLUMNS} FROM memories WHERE subject = '' AND valid_to IS NULL \
                     ORDER BY updated_at DESC LIMIT {MERGE_WINDOW}"
                ),
                [],
                map_memory,
            )?);
        }
        if let Some(near) = candidates
            .into_iter()
            .find(|candidate| same_statement(&candidate.content, &item.content))
        {
            return Ok(WriteOutcome::Unchanged(near));
        }
    }

    // A replacement ends when the new statement begins, not when the write
    // happened. One statement can be written weeks after the change it records.
    let id = reserve_id(writer, ids)?;
    let supersedes = supersedes.or(earlier.map(|earlier| earlier.id));
    if let Some(previous) = &supersedes {
        let closed = writer.execute(
            "UPDATE memories SET valid_to = ?, superseded_by = ?, pinned = 0 \
             WHERE id = ? AND valid_to IS NULL",
            rusqlite::params![valid_from, id, previous],
        )?;
        if closed == 0 {
            return Err(MemoryError::new(
                Code::NotFound,
                format!("memory {previous} does not exist or was already replaced."),
            ));
        }
    }

    writer.execute(
        "INSERT INTO memories \
         (id, kind, content, subject, terms, fingerprint, pinned, source_kind, source_entry, \
          valid_from, valid_to, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            id,
            item.kind.as_str(),
            item.content,
            item.subject,
            terms,
            digest,
            i64::from(item.pinned),
            item.source.as_str(),
            item.source_entry,
            valid_from,
            valid_to,
            now,
            now,
        ],
    )?;

    if let Some(vector) = vector {
        write_vector(writer, &id, &vector, now)?;
    }

    Ok(WriteOutcome::Created(writer.query_row(
        &format!("SELECT {COLUMNS} FROM memories WHERE id = ?"),
        [&id],
        map_memory,
    )?))
}

/// Reserve an id no row holds.
///
/// One writer thread owns every insert, so checking inside the write transaction
/// is the same as catching the primary-key conflict. A taken id is retried with
/// a fresh one; a row is never given an id that already exists.
fn reserve_id(
    writer: &hearth_storage::Writer<'_>,
    ids: &mut dyn FnMut() -> String,
) -> Result<String> {
    for _ in 0..ID_ATTEMPTS {
        let id = ids();
        let taken = writer
            .query_optional("SELECT 1 FROM memories WHERE id = ?", [&id], |row| {
                row.get::<_, i64>(0)
            })?
            .is_some();
        if !taken {
            return Ok(id);
        }
    }
    Err(MemoryError::unavailable(format!(
        "memory id generation failed after {ID_ATTEMPTS} attempts."
    )))
}

fn quantise(values: &[f32], dims: usize) -> Result<Vec<i8>> {
    if values.len() != dims {
        return Err(MemoryError::invalid(
            "the embedding length did not match its model.",
        ));
    }
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    let denominator = if norm > 0.0 { norm } else { 1.0 };
    Ok(values
        .iter()
        .map(|value| {
            let scaled = (value / denominator * 127.0).round();
            scaled.clamp(-127.0, 127.0) as i8
        })
        .collect())
}

fn write_vector(
    writer: &hearth_storage::Writer<'_>,
    id: &str,
    vector: &VectorWrite,
    now: i64,
) -> std::result::Result<(), hearth_storage::StorageError> {
    let quantised = quantise(&vector.values, vector.dims)
        .map_err(|error| hearth_storage::StorageError::Integrity(error.to_string()))?;
    let bytes: Vec<u8> = quantised.iter().map(|value| *value as u8).collect();
    writer.execute(
        "INSERT OR REPLACE INTO memory_vectors (memory_id, model, dims, vector, updated_at) \
         VALUES (?, ?, ?, ?, ?)",
        rusqlite::params![id, vector.model, vector.dims as i64, bytes, now],
    )?;
    Ok(())
}

/// Delete one memory permanently.
pub fn forget(session: &Session, id: &str) -> Result<()> {
    let identifier = normalize_id(id);
    let removed = session.storage().write(move |writer| {
        writer.execute(
            "DELETE FROM memory_vectors WHERE memory_id = ?",
            [&identifier],
        )?;
        writer.execute("DELETE FROM memories WHERE id = ?", [&identifier])
    })?;
    if removed == 0 {
        return Err(MemoryError::new(
            Code::NotFound,
            format!("memory {id} does not exist."),
        ));
    }
    Ok(())
}

/// A current statement about the same subject that a new one may replace.
///
/// This is the write-time reconciliation signal. The store never decides to
/// overwrite on its own; it reports the neighbour and its overlap so the model
/// can replace it in the same turn.
pub fn related(
    session: &Session,
    subject: &str,
    content: &str,
    exclude: &str,
) -> Result<Option<(Memory, f32)>> {
    let candidates = if subject.is_empty() {
        recent(session, None, None, 32)?
    } else {
        by_subject(session, subject, None, None, 32)?
    };
    Ok(candidates
        .into_iter()
        .filter(|candidate| candidate.id != exclude)
        .map(|candidate| {
            let overlap = similarity(&candidate.content, content);
            (candidate, overlap)
        })
        .filter(|(_, overlap)| *overlap >= 0.2)
        .max_by(|left, right| left.1.total_cmp(&right.1)))
}

/// Attach or refresh one memory's vector without touching its row.
pub fn set_vector(session: &Session, id: &str, vector: &VectorWrite) -> Result<()> {
    let id = id.to_owned();
    let vector = vector.clone();
    let now = session.now_micros();
    session
        .storage()
        .write(move |writer| write_vector(writer, &id, &vector, now))?;
    Ok(())
}

/// Every stored vector for a model.
pub fn vectors(session: &Session, model: &str, dims: usize) -> Result<Vec<VectorRow>> {
    session
        .storage()
        .read(|reader| {
            reader.query(
                "SELECT v.memory_id, v.vector, m.valid_to IS NULL \
                 FROM memory_vectors v JOIN memories m ON m.id = v.memory_id \
                 WHERE v.model = ? AND v.dims = ?",
                rusqlite::params![model, dims as i64],
                |row| {
                    let bytes: Vec<u8> = row.get(1)?;
                    Ok(VectorRow {
                        memory_id: row.get(0)?,
                        current: row.get::<_, i64>(2)? != 0,
                        quantised: bytes.into_iter().map(|byte| byte as i8).collect(),
                    })
                },
            )
        })
        .map_err(Into::into)
}

/// Current memories that have no vector for the active model.
pub fn pending_vectors(
    session: &Session,
    model: &str,
    dims: usize,
    limit: i64,
) -> Result<Vec<Memory>> {
    session
        .storage()
        .read(|reader| {
            reader.query(
                &format!(
                    "SELECT {COLUMNS} FROM memories m \
                     WHERE m.valid_to IS NULL AND NOT EXISTS ( \
                       SELECT 1 FROM memory_vectors v WHERE v.memory_id = m.id AND v.model = ? \
                       AND v.dims = ?) \
                     ORDER BY m.updated_at DESC LIMIT ?"
                ),
                rusqlite::params![model, dims as i64, limit],
                map_memory,
            )
        })
        .map_err(Into::into)
}

/// Remove vectors whose memory is gone, or whose model is no longer active.
pub fn prune_vectors(session: &Session, model: &str, dims: usize) -> Result<usize> {
    let model = model.to_owned();
    session.storage().write(move |writer| {
        let orphans = writer.execute(
            "DELETE FROM memory_vectors WHERE memory_id NOT IN (SELECT id FROM memories)",
            [],
        )?;
        let stale = writer.execute(
            "DELETE FROM memory_vectors WHERE model <> ? OR dims <> ?",
            rusqlite::params![model, dims as i64],
        )?;
        Ok::<usize, MemoryError>(orphans + stale)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryConfig;
    use hearth_core::Harness;
    use tempfile::TempDir;

    fn home() -> (TempDir, Session) {
        let directory = TempDir::new().expect("temp home");
        let harness = Harness::open(directory.path()).expect("harness");
        let session = harness.session().clone();
        (directory, session)
    }

    fn write(session: &Session, item: NewMemory) -> WriteOutcome {
        insert(
            session,
            Pending {
                item,
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("write")
    }

    #[test]
    fn the_subject_list_stops_at_its_limit_longest_first() {
        let (_directory, session) = home();
        let items: Vec<Pending> = (0..20)
            .map(|index| Pending {
                item: NewMemory::new(format!("note {index}"))
                    .subject(format!("subject{}", "x".repeat(index))),
                vector: None,
            })
            .collect();
        insert_many(&session, items, MemoryConfig::default().rules_max_tokens)
            .expect("seed subjects");

        let subjects = subjects_limited(&session, 10).expect("subjects");
        assert_eq!(subjects.len(), 10, "the list must stop at its limit");
        let lengths: Vec<usize> = subjects.iter().map(|subject| subject.len()).collect();
        let mut sorted = lengths.clone();
        sorted.sort_unstable_by(|left, right| right.cmp(left));
        assert_eq!(lengths, sorted, "the longest subjects must be kept first");
    }

    #[test]
    fn an_identical_statement_is_not_stored_twice() {
        let (_directory, session) = home();
        let first = insert(
            &session,
            Pending {
                item: NewMemory::new("Alice manages the OR board."),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("first");
        assert!(first.created());
        let second = insert(
            &session,
            Pending {
                item: NewMemory::new("  alice MANAGES the OR board.  "),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("second");
        assert!(!second.created());
        assert_eq!(first.memory().id, second.memory().id);
        assert_eq!(count(&session).unwrap(), 1);
    }

    #[test]
    fn a_near_duplicate_about_a_subject_merges() {
        let (_directory, session) = home();
        insert(
            &session,
            Pending {
                item: NewMemory::new("ordered pad thai on friday").subject("thai place"),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("first");
        let again = insert(
            &session,
            Pending {
                item: NewMemory::new("ordered pad thai on friday again").subject("thai place"),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("second");
        assert!(!again.created(), "the second order must not be a new row");
        assert_eq!(count(&session).unwrap(), 1);
    }

    #[test]
    fn the_same_words_in_a_new_role_replace_the_old_statement() {
        let (_directory, session) = home();
        let fact = insert(
            &session,
            Pending {
                item: NewMemory::new("keep replies short"),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("fact");
        assert!(fact.created());
        let rule = insert(
            &session,
            Pending {
                item: NewMemory::new("keep replies short")
                    .kind(Kind::Rule)
                    .pinned(true),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("rule");
        assert!(
            rule.created(),
            "promoting a fact to a rule must write a row, not return the fact"
        );
        assert!(rule.memory().pinned);
        assert_eq!(rules(&session).unwrap().len(), 1);
        assert_eq!(count(&session).unwrap(), 1);
    }

    #[test]
    fn a_replacement_closes_the_old_row_without_deleting_it() {
        let (_directory, session) = home();
        let old = insert(
            &session,
            Pending {
                item: NewMemory::new("the primary database is Postgres")
                    .kind(Kind::Decision)
                    .subject("primary database"),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("old");
        let new = insert(
            &session,
            Pending {
                item: NewMemory::new("the primary database is SQLite")
                    .kind(Kind::Decision)
                    .subject("primary database")
                    .supersedes(old.memory().id.clone()),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("new");
        assert!(new.created());
        let closed = get(&session, &old.memory().id).unwrap().expect("old row");
        assert!(!closed.is_current());
        assert_eq!(
            closed.superseded_by.as_deref(),
            Some(new.memory().id.as_str())
        );
        assert_eq!(
            by_subject(&session, "primary database", None, None, 10)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(historical_count(&session, "primary database").unwrap(), 1);
    }

    #[test]
    fn a_pin_on_a_statement_written_already_retired_is_refused() {
        let (_directory, session) = home();
        let start = session.now_micros() - 1_000_000;
        let error = insert(
            &session,
            Pending {
                item: NewMemory::new("never deploy twice")
                    .kind(Kind::Rule)
                    .pinned(true)
                    .valid_from(start)
                    .valid_to(Some(session.now_micros())),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect_err("a pin on a closed window");
        assert_eq!(error.code, Code::InvalidInput);
    }

    #[test]
    fn a_pin_left_on_a_retired_row_is_not_a_rule() {
        let (_directory, session) = home();
        let rule = insert(
            &session,
            Pending {
                item: NewMemory::new("never force push")
                    .kind(Kind::Rule)
                    .pinned(true),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("rule")
        .memory()
        .clone();
        // A store written before retirement cleared the pin.
        let instant = session.now_micros();
        let id = rule.id.clone();
        session
            .storage()
            .write(move |writer| {
                writer.execute(
                    "UPDATE memories SET valid_to = ?, superseded_by = 'm_old' WHERE id = ?",
                    rusqlite::params![instant, id],
                )
            })
            .expect("retire");

        assert!(
            rules(&session).expect("rules").is_empty(),
            "a closed window cannot be injected as a rule"
        );
        let budget = session.storage().write(|writer| {
            check_rules_budget(
                writer,
                "a new rule",
                None,
                crate::digest::rule_cost_text("a new rule"),
            )
        });
        assert!(
            budget.is_ok(),
            "a retired row must not hold the rules budget"
        );
    }

    #[test]
    fn a_memory_at_the_content_limit_is_accepted_and_one_over_is_refused() {
        let (_directory, session) = home();
        let at_limit = "a".repeat(CONTENT_LIMIT);
        insert(
            &session,
            Pending {
                item: NewMemory::new(&at_limit),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("the content limit is accepted");

        let over = "b".repeat(CONTENT_LIMIT + 1);
        let error = insert(
            &session,
            Pending {
                item: NewMemory::new(&over),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect_err("one over the content limit is refused");
        assert_eq!(error.code, Code::InvalidInput);
        assert!(
            error.message.contains(&(CONTENT_LIMIT + 1).to_string()),
            "{}",
            error.message
        );
        assert!(
            error.message.contains(&CONTENT_LIMIT.to_string()),
            "{}",
            error.message
        );
        assert_eq!(
            count(&session).expect("count"),
            1,
            "the refused write must save nothing"
        );
    }

    #[test]
    fn a_pinned_rule_uses_the_same_content_limit() {
        let (_directory, session) = home();
        let at_limit = "r".repeat(CONTENT_LIMIT);
        insert(
            &session,
            Pending {
                item: NewMemory::new(&at_limit).kind(Kind::Rule).pinned(true),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("the content limit is accepted as a rule");

        let over = "s".repeat(CONTENT_LIMIT + 1);
        let error = insert(
            &session,
            Pending {
                item: NewMemory::new(&over).kind(Kind::Rule).pinned(true),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect_err("one over the content limit is refused as a rule");
        assert_eq!(error.code, Code::InvalidInput);
        assert!(
            error.message.contains(&(CONTENT_LIMIT + 1).to_string()),
            "{}",
            error.message
        );
        assert!(
            error.message.contains(&CONTENT_LIMIT.to_string()),
            "{}",
            error.message
        );
        assert!(
            !error.message.contains("pinned rule"),
            "a rule uses the content error, not a separate one: {}",
            error.message
        );
    }

    #[test]
    fn a_credential_is_refused_before_it_is_stored() {
        let (_directory, session) = home();
        // A token-shaped string, assembled so the repository holds no literal.
        let secret = format!("api_key={}", "sk-".to_owned() + &"a".repeat(32));
        let error = insert(
            &session,
            Pending {
                item: NewMemory::new(format!("the deploy uses {secret}")),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect_err("credential");
        assert_eq!(error.code, Code::Credential);
        assert_eq!(count(&session).unwrap(), 0);
    }

    #[test]
    fn a_pinned_fact_is_refused_because_only_rules_are_injected() {
        let (_directory, session) = home();
        let error = insert(
            &session,
            Pending {
                item: NewMemory::new("a fact").pinned(true),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect_err("pinned fact");
        assert_eq!(error.code, Code::InvalidInput);
    }

    #[test]
    fn a_statement_dated_in_the_future_does_not_answer_today() {
        let (_directory, session) = home();
        let later = session.now_micros() + 86_400_000_000 * 30;
        insert(
            &session,
            Pending {
                item: NewMemory::new("the office moves to Lisbon")
                    .subject("office")
                    .valid_from(later),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("write");
        assert!(
            by_subject(&session, "office", None, None, 10)
                .unwrap()
                .is_empty(),
            "a statement that starts being true next month is not true now"
        );
        assert_eq!(
            by_subject(&session, "office", Some(later + 1), None, 10)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn an_as_of_read_sees_the_row_that_was_current_then() {
        let (_directory, session) = home();
        let early = session.now_micros();
        let old = insert(
            &session,
            Pending {
                item: NewMemory::new("the lab is in building 4")
                    .subject("lab")
                    .valid_from(early),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("old");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let middle = session.now_micros();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let new = insert(
            &session,
            Pending {
                item: NewMemory::new("the lab is in building 9")
                    .subject("lab")
                    .supersedes(old.memory().id.clone()),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("new");

        let then = by_subject(&session, "lab", Some(middle), None, 10).unwrap();
        assert_eq!(then.len(), 1);
        assert_eq!(then[0].id, old.memory().id);
        let now = by_subject(&session, "lab", None, None, 10).unwrap();
        assert_eq!(now.len(), 1);
        assert_eq!(now[0].id, new.memory().id);
    }
    #[test]
    fn a_new_id_is_short_and_crockford() {
        let id = new_id();
        assert!(id.starts_with("m_"), "{id}");
        let suffix = &id[2..];
        assert_eq!(suffix.chars().count(), 10, "{id}");
        assert!(
            suffix
                .chars()
                .all(|character| "0123456789abcdefghjkmnpqrstvwxyz".contains(character)),
            "an id must use Crockford base32: {id}"
        );
    }

    #[test]
    fn a_taken_id_is_retried() {
        let (_directory, session) = home();
        let first = insert(
            &session,
            Pending {
                item: NewMemory::new("the first fact"),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("first");
        let taken = first.memory().id.clone();
        let fresh = new_id();
        let mut ids = vec![taken.clone(), fresh.clone()].into_iter();
        let now = session.now_micros();
        let outcome = session
            .storage()
            .write(move |writer| {
                insert_with(
                    writer,
                    Pending {
                        item: NewMemory::new("the second fact"),
                        vector: None,
                    },
                    now,
                    MemoryConfig::default().rules_max_tokens,
                    &mut || ids.next().expect("an id"),
                )
            })
            .expect("second");
        assert!(outcome.created());
        assert_eq!(outcome.memory().id, fresh, "the taken id must be retried");
        assert_ne!(outcome.memory().id, taken);
        assert_eq!(count(&session).expect("count"), 2);
    }

    #[test]
    fn an_id_is_read_case_insensitively_and_trimmed() {
        let (_directory, session) = home();
        let row = insert(
            &session,
            Pending {
                item: NewMemory::new("a fact worth finding"),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("row")
        .memory()
        .clone();
        let upper = format!("  {}  ", row.id.to_uppercase());
        assert!(
            get(&session, &upper).expect("get").is_some(),
            "an id must be read in either case with surrounding whitespace"
        );
        forget(&session, &upper).expect("forget");
        assert!(get(&session, &row.id).expect("get").is_none());
    }

    #[test]
    fn a_prefix_is_not_an_id() {
        let (_directory, session) = home();
        let row = insert(
            &session,
            Pending {
                item: NewMemory::new("a fact worth finding"),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("row")
        .memory()
        .clone();
        let prefix = &row.id[..4];
        assert!(
            get(&session, prefix).expect("get").is_none(),
            "a prefix must not address a row"
        );
    }

    #[test]
    fn a_legacy_id_is_still_found_and_replaced() {
        let (_directory, session) = home();
        let row = insert(
            &session,
            Pending {
                item: NewMemory::new("the lab is in building 4").subject("lab"),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("row")
        .memory()
        .clone();
        let legacy = "m_00000000000000000000000000000000";
        let old_id = row.id.clone();
        session
            .storage()
            .write(move |writer| {
                writer.execute(
                    "UPDATE memories SET id = ? WHERE id = ?",
                    rusqlite::params![legacy, old_id],
                )
            })
            .expect("rename to a legacy id");

        let found = get(&session, legacy).expect("get").expect("legacy row");
        assert_eq!(found.content, "the lab is in building 4");
        let replaced = insert(
            &session,
            Pending {
                item: NewMemory::new("the lab is in building 9")
                    .subject("lab")
                    .supersedes(legacy),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("replace");
        assert!(replaced.created());
        let old = get(&session, legacy).expect("get").expect("old row");
        assert_eq!(
            old.superseded_by.as_deref(),
            Some(replaced.memory().id.as_str())
        );
    }
    #[test]
    fn a_statement_with_a_pronoun_is_stored() {
        let (_directory, session) = home();
        let mut written = Vec::new();
        for content in [
            "He likes apples",
            "I prefer dark mode",
            "you should deploy on Friday",
            "we ship on Fridays",
        ] {
            let outcome = insert(
                &session,
                Pending {
                    item: NewMemory::new(content),
                    vector: None,
                },
                MemoryConfig::default().rules_max_tokens,
            )
            .unwrap_or_else(|error| panic!("{content} must store: {error:?}"));
            written.push(outcome.memory().content.clone());
        }
        assert_eq!(written.len(), 4);
        assert_eq!(
            count(&session).expect("count"),
            4,
            "a pronoun saves like any other word"
        );
    }

    #[test]
    fn a_statement_with_a_named_actor_is_accepted() {
        let (_directory, session) = home();
        for content in ["Bob likes apples", "the user likes apples"] {
            insert(
                &session,
                Pending {
                    item: NewMemory::new(content),
                    vector: None,
                },
                MemoryConfig::default().rules_max_tokens,
            )
            .expect("a named statement stores");
        }
        assert_eq!(count(&session).expect("count"), 2);
    }

    #[test]
    fn a_pinned_rule_is_stored() {
        let (_directory, session) = home();
        insert(
            &session,
            Pending {
                item: NewMemory::new("Never deploy on Friday")
                    .kind(Kind::Rule)
                    .pinned(true),
                vector: None,
            },
            MemoryConfig::default().rules_max_tokens,
        )
        .expect("an imperative rule stores");
    }

    #[test]
    fn a_synonym_about_the_same_subject_is_refused() {
        let (_directory, session) = home();
        let first = write(
            &session,
            NewMemory::new("the user prefers dark mode")
                .kind(Kind::Preference)
                .subject("user"),
        );
        assert!(first.created());
        let second = write(
            &session,
            NewMemory::new("the user likes dark mode")
                .kind(Kind::Preference)
                .subject("user"),
        );
        assert!(!second.created(), "a synonym must not become a second row");
        assert_eq!(second.memory().id, first.memory().id);
        assert_eq!(count(&session).expect("count"), 1);
    }

    #[test]
    fn a_spelling_variant_about_the_same_subject_is_refused() {
        let (_directory, session) = home();
        let first = write(
            &session,
            NewMemory::new("the user's favorite color is blue")
                .kind(Kind::Preference)
                .subject("user"),
        );
        assert!(first.created());
        let second = write(
            &session,
            NewMemory::new("the user's favourite colour is blue")
                .kind(Kind::Preference)
                .subject("user"),
        );
        assert!(!second.created(), "a spelling variant is the same statement");
        assert_eq!(count(&session).expect("count"), 1);
    }

    #[test]
    fn a_negation_is_not_a_duplicate_of_its_affirmative() {
        let (_directory, session) = home();
        write(
            &session,
            NewMemory::new("the user prefers dark mode")
                .kind(Kind::Preference)
                .subject("user"),
        );
        let negated = write(
            &session,
            NewMemory::new("the user does not prefer dark mode")
                .kind(Kind::Preference)
                .subject("user"),
        );
        assert!(
            negated.created(),
            "a negation states the opposite and must stay current"
        );
        assert_eq!(count(&session).expect("count"), 2);
    }

    #[test]
    fn a_subjectless_write_is_checked_against_the_store() {
        let (_directory, session) = home();
        let first = write(
            &session,
            NewMemory::new("the user prefers dark mode")
                .kind(Kind::Preference)
                .subject("user"),
        );
        assert!(first.created());
        let second = write(&session, NewMemory::new("the user prefers dark mode please"));
        assert!(
            !second.created(),
            "the near check must not depend on both subjects being set"
        );
        assert_eq!(count(&session).expect("count"), 1);
    }

    #[test]
    fn an_older_duplicate_is_found_beyond_the_subjectless_window() {
        let (_directory, session) = home();
        let first = write(
            &session,
            NewMemory::new("ordered pad thai on friday").subject("thai place"),
        );
        assert!(first.created());
        for index in 0..70 {
            write(
                &session,
                NewMemory::new(format!("ordered duck on monday number {index}"))
                    .subject("thai place"),
            );
        }
        let again = write(
            &session,
            NewMemory::new("ordered pad thai on friday again").subject("thai place"),
        );
        assert!(
            !again.created(),
            "a same-subject duplicate must be found however old it is"
        );
        assert_eq!(again.memory().id, first.memory().id);
    }

    #[test]
    fn an_automatic_extract_cannot_weaken_a_stored_row() {
        let (_directory, session) = home();
        let preference = write(
            &session,
            NewMemory::new("the user prefers dark mode")
                .kind(Kind::Preference)
                .subject("user"),
        );
        let extracted = write(
            &session,
            NewMemory::new("the user prefers dark mode")
                .source(Source::Extract)
                .subject(""),
        );
        assert!(!extracted.created());
        assert_eq!(extracted.memory().id, preference.memory().id);
        assert_eq!(extracted.memory().kind, Kind::Preference);
        assert_eq!(extracted.memory().subject, "user");
        assert_eq!(count(&session).expect("count"), 1);
    }

    #[test]
    fn a_subjectless_write_still_reports_a_related_neighbour() {
        let (_directory, session) = home();
        write(
            &session,
            NewMemory::new("Alice manages the OR board").subject("alice"),
        );
        let neighbour = related(&session, "", "Alice manages the OR board and budget", "none")
            .expect("related");
        assert!(
            neighbour.is_some(),
            "an empty subject must not hide a neighbour"
        );
    }

}
