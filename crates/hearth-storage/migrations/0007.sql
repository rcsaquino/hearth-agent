-- Search the transcript without scanning every entry.
--
-- JSON is indexed as stored. The tokenizer ignores punctuation, so decoded
-- entry text remains searchable; malformed or partial JSON still falls back to
-- the application's literal scan.
CREATE VIRTUAL TABLE entries_fts USING fts5(
    content_json,
    content='entries',
    content_rowid='rowid',
    tokenize='porter unicode61 remove_diacritics 2',
    prefix='2 3'
);

INSERT INTO entries_fts(entries_fts) VALUES('rebuild');

CREATE TRIGGER entries_fts_insert AFTER INSERT ON entries BEGIN
    INSERT INTO entries_fts(rowid, content_json) VALUES(new.rowid, new.content_json);
END;

CREATE TRIGGER entries_fts_delete AFTER DELETE ON entries BEGIN
    INSERT INTO entries_fts(entries_fts, rowid, content_json)
      VALUES('delete', old.rowid, old.content_json);
END;

CREATE TRIGGER entries_fts_update AFTER UPDATE OF content_json ON entries BEGIN
    INSERT INTO entries_fts(entries_fts, rowid, content_json)
      VALUES('delete', old.rowid, old.content_json);
    INSERT INTO entries_fts(rowid, content_json) VALUES(new.rowid, new.content_json);
END;
