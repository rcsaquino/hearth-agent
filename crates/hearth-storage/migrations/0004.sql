-- Reindex memory search only when the indexed columns change. The previous
-- trigger fired on every UPDATE, including last_used_at, importance, standing,
-- and status, which reindexed the whole row for a touch.
DROP TRIGGER IF EXISTS memory_items_au;

CREATE TRIGGER memory_items_au AFTER UPDATE OF content, tags ON memory_items BEGIN
    INSERT INTO memory_fts(memory_fts, rowid, content, tags)
    VALUES ('delete', old.rowid, old.content, old.tags);
    INSERT INTO memory_fts(rowid, content, tags)
    VALUES (new.rowid, new.content, new.tags);
END;
