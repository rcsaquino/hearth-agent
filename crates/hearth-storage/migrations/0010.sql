-- The recall subject list reads the longest names first and stops at a bound.
-- Index the expression in that read order so the query walks an ordered covering
-- index instead of sorting every distinct subject on the turn path.
CREATE INDEX memories_subject_length ON memories(LENGTH(subject) DESC, subject)
WHERE subject <> '';
