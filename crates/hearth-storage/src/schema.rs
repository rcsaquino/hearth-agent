/// Migration 0001 creates the full 1.0 schema. The SQL file is the executable
/// schema authority; `SCHEMA.md` documents it.
pub const MIGRATION_0001: &str = include_str!("../migrations/0001.sql");

/// Migration 0002 rebuilds memory search with the porter stemmer.
pub const MIGRATION_0002: &str = include_str!("../migrations/0002.sql");

/// Migration 0003 indexes standing selection and promotable candidates.
pub const MIGRATION_0003: &str = include_str!("../migrations/0003.sql");
