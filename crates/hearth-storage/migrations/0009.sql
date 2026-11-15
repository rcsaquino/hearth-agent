-- A pin belongs to a live statement, and retirement now clears it. Rows retired
-- by an older build kept their pin, so the stored flag still claimed they were
-- injected into every request. Clear it once here rather than in every read.
UPDATE memories SET pinned = 0 WHERE pinned = 1 AND valid_to IS NOT NULL;
