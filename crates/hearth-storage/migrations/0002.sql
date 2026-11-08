DROP TABLE memory_fts;

CREATE VIRTUAL TABLE memory_fts USING fts5(
    content,
    tags,
    content='memory_items',
    content_rowid='rowid',
    tokenize='porter unicode61'
);

INSERT INTO memory_fts(memory_fts) VALUES('rebuild');
