CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE commits (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at INTEGER NOT NULL,
    conversation_id TEXT,
    task_id TEXT,
    kind TEXT NOT NULL
);

CREATE TABLE conversations (
    id TEXT PRIMARY KEY,
    parent_id TEXT,
    parent_entry_id TEXT,
    title TEXT NOT NULL,
    transport TEXT NOT NULL,
    transport_key TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (parent_id) REFERENCES conversations(id),
    FOREIGN KEY (parent_entry_id) REFERENCES entries(id)
);

CREATE TABLE prompt_blobs (
    hash TEXT PRIMARY KEY,
    text TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE entries (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    content_json TEXT NOT NULL,
    tokens_est INTEGER NOT NULL DEFAULT 0,
    source TEXT,
    prompt_hash TEXT,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (conversation_id) REFERENCES conversations(id),
    FOREIGN KEY (prompt_hash) REFERENCES prompt_blobs(hash)
);

CREATE INDEX entries_conversation_seq ON entries(conversation_id, seq);

CREATE TABLE documents (
    conversation_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    version INTEGER NOT NULL,
    json TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (conversation_id, kind),
    FOREIGN KEY (conversation_id) REFERENCES conversations(id)
);

CREATE TABLE submissions (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    request_id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    content_json TEXT NOT NULL,
    status TEXT NOT NULL,
    answer_entry_id TEXT,
    error_json TEXT,
    created_at INTEGER NOT NULL,
    settled_at INTEGER,
    FOREIGN KEY (conversation_id) REFERENCES conversations(id),
    FOREIGN KEY (answer_entry_id) REFERENCES entries(id)
);

CREATE INDEX submissions_status ON submissions(status);

CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    conversation_id TEXT,
    parent_task_id TEXT,
    kind TEXT NOT NULL,
    version INTEGER NOT NULL,
    status TEXT NOT NULL,
    phase TEXT,
    attempt INTEGER NOT NULL DEFAULT 0,
    input_json TEXT NOT NULL,
    checkpoint_json TEXT,
    result_json TEXT,
    lease_owner TEXT,
    lease_until INTEGER,
    next_wake_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    terminal_at INTEGER,
    FOREIGN KEY (conversation_id) REFERENCES conversations(id),
    FOREIGN KEY (parent_task_id) REFERENCES tasks(id)
);

CREATE INDEX tasks_status_wake ON tasks(status, next_wake_at);

CREATE TABLE effects (
    id TEXT PRIMARY KEY,
    task_id TEXT,
    kind TEXT NOT NULL,
    key TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    status TEXT NOT NULL,
    result_json TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    settled_at INTEGER,
    UNIQUE (kind, key),
    FOREIGN KEY (task_id) REFERENCES tasks(id)
);

CREATE INDEX effects_status ON effects(status);

CREATE TABLE usage_events (
    id TEXT PRIMARY KEY,
    ts INTEGER NOT NULL,
    task_id TEXT,
    conversation_id TEXT,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    kind TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cost_micros INTEGER NOT NULL DEFAULT 0,
    latency_ms INTEGER,
    FOREIGN KEY (task_id) REFERENCES tasks(id),
    FOREIGN KEY (conversation_id) REFERENCES conversations(id)
);

CREATE INDEX usage_events_ts ON usage_events(ts);

CREATE TABLE telegram_updates (
    update_id INTEGER PRIMARY KEY,
    chat_id INTEGER,
    message_id INTEGER,
    processed_at INTEGER NOT NULL
);

CREATE TABLE memory_items (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    tags TEXT NOT NULL DEFAULT '[]',
    fingerprint TEXT NOT NULL,
    importance INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL,
    valid_from INTEGER,
    valid_to INTEGER,
    supersedes_id TEXT,
    source_kind TEXT NOT NULL,
    source_ref TEXT,
    standing TEXT NOT NULL DEFAULT 'none',
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    FOREIGN KEY (supersedes_id) REFERENCES memory_items(id)
);

CREATE INDEX memory_items_status_kind ON memory_items(status, kind);
CREATE INDEX memory_items_fingerprint ON memory_items(fingerprint);

CREATE VIRTUAL TABLE memory_fts USING fts5(
    content,
    tags,
    content='memory_items',
    content_rowid='rowid'
);

CREATE TRIGGER memory_items_ai AFTER INSERT ON memory_items BEGIN
    INSERT INTO memory_fts(rowid, content, tags)
    VALUES (new.rowid, new.content, new.tags);
END;

CREATE TRIGGER memory_items_ad AFTER DELETE ON memory_items BEGIN
    INSERT INTO memory_fts(memory_fts, rowid, content, tags)
    VALUES ('delete', old.rowid, old.content, old.tags);
END;

CREATE TRIGGER memory_items_au AFTER UPDATE ON memory_items BEGIN
    INSERT INTO memory_fts(memory_fts, rowid, content, tags)
    VALUES ('delete', old.rowid, old.content, old.tags);
    INSERT INTO memory_fts(rowid, content, tags)
    VALUES (new.rowid, new.content, new.tags);
END;

CREATE TABLE memory_events (
    id TEXT PRIMARY KEY,
    item_id TEXT NOT NULL,
    action TEXT NOT NULL,
    actor TEXT NOT NULL,
    old_hash TEXT,
    new_hash TEXT,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (item_id) REFERENCES memory_items(id)
);
