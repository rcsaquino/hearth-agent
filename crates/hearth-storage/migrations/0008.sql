-- Replace the two older memory attempts with one store in the main database.
--
-- `memory_items` and `memory_events` were written only by an unused core path,
-- and `hearth-memory` kept a second copy of the same idea in its own file. Both
-- are replaced by `memories`, which lives beside the transcript so a memory and
-- its provenance share one writer, one transaction, and one backup.
DROP TABLE IF EXISTS memory_vectors;
DROP TABLE IF EXISTS memory_events;
DROP TRIGGER IF EXISTS memory_items_ai;
DROP TRIGGER IF EXISTS memory_items_ad;
DROP TRIGGER IF EXISTS memory_items_au;
DROP TABLE IF EXISTS memory_fts;
DROP TABLE IF EXISTS memory_items;

-- One row is one statement. `subject` is the normalised entity the statement is
-- about, and it is the strongest retrieval signal a personal memory has.
-- `valid_to` is set only when a newer row supersedes this one, so history is
-- kept and a contradiction is a date range rather than an overwrite.
CREATE TABLE memories (
    rowid INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    subject TEXT NOT NULL DEFAULT '',
    terms TEXT NOT NULL DEFAULT '',
    fingerprint TEXT NOT NULL,
    pinned INTEGER NOT NULL DEFAULT 0,
    source_kind TEXT NOT NULL,
    source_entry TEXT,
    valid_from INTEGER NOT NULL,
    valid_to INTEGER,
    superseded_by TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1
) STRICT;

CREATE INDEX memories_subject ON memories(subject, valid_to);
CREATE INDEX memories_kind ON memories(kind, valid_to);
CREATE INDEX memories_fingerprint ON memories(fingerprint);
CREATE INDEX memories_pinned ON memories(pinned) WHERE pinned = 1;

-- `terms` carries the expansions a reader would not type: entity variants and
-- the head words of the statement. Content, subject, and terms share one index
-- so a single MATCH reaches all three.
CREATE VIRTUAL TABLE memories_fts USING fts5(
    content,
    subject,
    terms,
    content='memories',
    content_rowid='rowid',
    tokenize='porter unicode61 remove_diacritics 2',
    prefix='2 3'
);

CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, content, subject, terms)
    VALUES (new.rowid, new.content, new.subject, new.terms);
END;

CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, content, subject, terms)
    VALUES ('delete', old.rowid, old.content, old.subject, old.terms);
END;

CREATE TRIGGER memories_au AFTER UPDATE OF content, subject, terms ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, content, subject, terms)
    VALUES ('delete', old.rowid, old.content, old.subject, old.terms);
    INSERT INTO memories_fts(rowid, content, subject, terms)
    VALUES (new.rowid, new.content, new.subject, new.terms);
END;

-- One INT8 vector per memory, stored unit-length so cosine similarity is a dot
-- product with no per-row scale. The model name and dimension guard against a
-- different encoder; an edit creates a new row, so a vector cannot go stale.
CREATE TABLE memory_vectors (
    memory_id TEXT PRIMARY KEY,
    model TEXT NOT NULL,
    dims INTEGER NOT NULL,
    vector BLOB NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE INDEX memory_vectors_model ON memory_vectors(model, dims);
