-- A submission records the user entry it answers. A later pass reads the run's
-- own words instead of whichever message arrived last.
ALTER TABLE submissions ADD COLUMN user_entry_id TEXT REFERENCES entries(id);

-- Existing rows link to the entry saved with them: same conversation, same
-- content, same instant. The earliest such entry wins when a legacy save wrote
-- more than one.
UPDATE submissions SET user_entry_id = (
    SELECT e.id FROM entries e
    WHERE e.conversation_id = submissions.conversation_id
      AND e.kind = 'user'
      AND e.content_json = submissions.content_json
      AND e.created_at = submissions.created_at
    ORDER BY e.seq, e.rowid
    LIMIT 1
);

CREATE INDEX submissions_user_entry ON submissions(user_entry_id);
