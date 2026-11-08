/// Migration 0001 creates the full 1.0 schema. The SQL file is the executable
/// schema authority; `SCHEMA.md` documents it.
pub const MIGRATION_0001: &str = include_str!("../migrations/0001.sql");
