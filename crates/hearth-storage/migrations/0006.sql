-- Cached embeddings for memory recall. One row per memory item; the model and
-- dimension are recorded so a model change invalidates stale vectors instead
-- of mixing incompatible ones.
CREATE TABLE memory_vectors (
    memory_id TEXT PRIMARY KEY,
    model TEXT NOT NULL,
    dims INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    vector BLOB NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (memory_id) REFERENCES memory_items(id) ON DELETE CASCADE
);

CREATE INDEX memory_vectors_model ON memory_vectors(model, dims);
