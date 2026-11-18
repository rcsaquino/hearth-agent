-- Inbound albums wait in one durable row per member until their quiet window
-- passes. The poller stages a member and advances the offset together, so a
-- restart keeps the album and no poll sleeps on it.

CREATE TABLE telegram_albums (
    chat_id INTEGER NOT NULL,
    group_id TEXT NOT NULL,
    thread_id INTEGER,
    ready_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (chat_id, group_id)
);

CREATE TABLE telegram_album_members (
    update_id INTEGER PRIMARY KEY,
    chat_id INTEGER NOT NULL,
    group_id TEXT NOT NULL,
    message_id INTEGER NOT NULL,
    message_json TEXT NOT NULL,
    FOREIGN KEY (chat_id, group_id) REFERENCES telegram_albums(chat_id, group_id)
);

CREATE INDEX telegram_albums_ready ON telegram_albums(ready_at);
CREATE INDEX telegram_album_members_group
    ON telegram_album_members(chat_id, group_id, message_id);
