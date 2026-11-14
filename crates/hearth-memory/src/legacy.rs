//! Reading the memory store this release replaced.
//!
//! The previous design kept `memory/long-term.sqlite` and `memory/standing.md`
//! beside the database. A home that has run before holds the user's real memory
//! in those files, and a rewrite that starts empty has lost it. This reads them
//! once, writes the rows through the normal store, and renames the directory so
//! the import cannot run twice.
//!
//! The old rows had no subject. A statement that opens with a name gets one from
//! the same rule the extractor uses; the rest are imported without a subject and
//! stay reachable by keyword, by meaning, and by enumeration.

use std::path::Path;

use crate::MemoryService;
use crate::errors::Result;
use crate::model::{Kind, NewMemory, Source};

/// What an import did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LegacyImport {
    pub statements: usize,
    pub rules: usize,
    /// Statements and rules read but not stored.
    pub skipped: usize,
    /// Where the old directory was moved, when there was one.
    pub archived: Option<std::path::PathBuf>,
}

impl LegacyImport {
    pub fn is_empty(&self) -> bool {
        self.statements == 0 && self.rules == 0 && self.skipped == 0
    }

    /// Whether every statement and rule that was read also reached the store.
    ///
    /// An import that dropped something keeps the old files, so the next start
    /// retries after the operator makes room. Retiring them would turn a refused
    /// write into a deletion.
    pub fn complete(&self) -> bool {
        self.skipped == 0
    }
}

/// Import and retire the previous memory files, if they exist.
pub fn import(service: &MemoryService, home: &Path) -> Result<LegacyImport> {
    let directory = home.join("memory");
    let database = directory.join("long-term.sqlite");
    let standing = directory.join("standing.md");
    if !database.is_file() && !standing.is_file() {
        return Ok(LegacyImport::default());
    }

    let mut report = LegacyImport::default();
    if database.is_file() {
        for (content, source) in read_statements(&database)? {
            let mut item = NewMemory::new(content.clone())
                .source(Source::Import)
                .subject(crate::suggest_subject(&content));
            if content.starts_with("The user prefers") || content.starts_with("Prefers") {
                item = item.kind(Kind::Preference);
            }
            item = item.from_entry(source);
            if service.write(item).is_ok() {
                report.statements += 1;
            } else {
                report.skipped += 1;
            }
        }
    }
    if standing.is_file()
        && let Ok(text) = std::fs::read_to_string(&standing)
    {
        for content in read_bullets(&text) {
            if service
                .write(
                    NewMemory::new(content)
                        .kind(Kind::Rule)
                        .pinned(true)
                        .source(Source::Import),
                )
                .is_ok()
            {
                report.rules += 1;
            } else {
                report.skipped += 1;
            }
        }
    }

    // Retire the old files only when everything in them reached the store.
    // A refused write is usually a full rules budget, which the operator can
    // fix; renaming first would make the retry impossible.
    let archived = directory.with_extension("imported");
    if report.complete() && !archived.exists() {
        std::fs::rename(&directory, &archived).map_err(|error| {
            crate::MemoryError::unavailable(format!(
                "the previous memory files could not be retired: {error}"
            ))
        })?;
        report.archived = Some(archived);
    }
    Ok(report)
}

/// Read the old statements, newest first.
fn read_statements(path: &Path) -> Result<Vec<(String, Option<String>)>> {
    let connection = hearth_storage::open_read_only(path).map_err(|error| {
        crate::MemoryError::unavailable(format!(
            "the previous memory store could not be read: {error}"
        ))
    })?;
    let mut statement = connection
        .prepare("SELECT content, source FROM memories ORDER BY created_at, rowid")
        .map_err(|error| {
            crate::MemoryError::unavailable(format!("the previous rows could not be read: {error}"))
        })?;
    let rows = statement
        .query_map([], |row| {
            let content: String = row.get(0)?;
            let source: Option<String> = row.get(1)?;
            Ok((content, source.filter(|value| !value.is_empty())))
        })
        .map_err(|error| {
            crate::MemoryError::unavailable(format!("the previous rows could not be read: {error}"))
        })?;
    Ok(rows.filter_map(std::result::Result::ok).collect())
}

/// Read the bullets of a `standing.md` file.
///
/// The old format was `- [s_<hex>] [p=<0-100>] content`. A hand edit could drop
/// either bracket, so the parser takes whatever follows the last `]` on a line
/// that starts with a dash.
fn read_bullets(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.trim_start().starts_with('-'))
        .filter_map(|line| {
            let rest = line.trim_start().trim_start_matches('-').trim();
            let content = match rest.rfind(']') {
                Some(end) => rest[end + 1..].trim(),
                None => rest,
            };
            (!content.is_empty() && content.len() <= crate::store::RULE_LIMIT)
                .then(|| content.to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hearth_core::Harness;
    use tempfile::TempDir;

    #[test]
    fn a_bullet_with_brackets_is_read_as_its_content() {
        let text = "- [s_0123456789abcdef] [p=80] Never use em dashes.\n- plain bullet\n";
        assert_eq!(
            read_bullets(text),
            vec!["Never use em dashes.".to_owned(), "plain bullet".to_owned()]
        );
    }

    #[test]
    fn the_old_store_is_imported_once_and_retired() {
        let home = TempDir::new().expect("temp home");
        let harness = Harness::open(home.path()).expect("harness");
        let service = MemoryService::new(harness.session().clone());
        std::fs::create_dir_all(home.path().join("memory")).expect("memory directory");

        let legacy = home.path().join("memory/long-term.sqlite");
        let connection = hearth_storage::rusqlite::Connection::open(&legacy).expect("legacy");
        connection
            .execute_batch(
                "CREATE TABLE memories (rowid INTEGER PRIMARY KEY, id TEXT, content TEXT, \
                 tags TEXT, tag_text TEXT, source TEXT, created_at TEXT, updated_at TEXT, \
                 revision INTEGER);
                 INSERT INTO memories(content, source, created_at, updated_at) \
                 VALUES('Alice manages the OR board', 'conversation', 'now', 'now');",
            )
            .expect("legacy rows");
        drop(connection);
        std::fs::write(
            home.path().join("memory/standing.md"),
            "- [s_0123456789abcdef] [p=90] Keep replies short.\n",
        )
        .expect("standing file");

        let report = import(&service, home.path()).expect("import");
        assert_eq!(report.statements, 1);
        assert_eq!(report.rules, 1);
        assert!(report.archived.is_some());
        assert!(service.count().expect("count") >= 1);
        assert!(
            service
                .rules()
                .expect("rules")
                .text
                .contains("Keep replies short.")
        );

        let again = import(&service, home.path()).expect("second import");
        assert!(again.is_empty(), "a second import must find nothing");
    }

    #[test]
    fn an_import_that_could_not_store_everything_keeps_the_old_files() {
        let home = TempDir::new().expect("temp home");
        let harness = Harness::open(home.path()).expect("harness");
        // A rules budget smaller than the standing file, which is what a home
        // with a full standing list sees after the cap was lowered.
        let service =
            MemoryService::new(harness.session().clone()).with_config(crate::MemoryConfig {
                rules_max_tokens: 50,
                ..crate::MemoryConfig::default()
            });
        std::fs::create_dir_all(home.path().join("memory")).expect("memory directory");
        std::fs::write(
            home.path().join("memory/standing.md"),
            "- [s_0123456789abcdef] [p=90] Keep every reply short, never use em dashes, and \
             prefer a plain sentence to a bulleted list unless the user asks for one.\n\
             - [s_fedcba9876543210] [p=80] Use metric units everywhere, write dates in ISO 8601, \
             and name the timezone when a time could be ambiguous to the reader.\n",
        )
        .expect("standing file");

        let report = import(&service, home.path()).expect("import");
        assert_eq!(report.rules, 1, "the first rule fits the budget");
        assert_eq!(report.skipped, 1, "the second does not");
        assert!(!report.complete());
        assert!(
            report.archived.is_none(),
            "a partial import must keep the old files so it can retry"
        );
        assert!(
            home.path().join("memory/standing.md").is_file(),
            "the standing file was retired after a refused write"
        );
    }
}
