ALTER TABLE notebooks RENAME TO old_notebooks;
CREATE TABLE notebooks (
  id INTEGER NOT NULL PRIMARY KEY,
  author_id INTEGER NOT NULL,
  title TEXT,
  data TEXT,
  is_pending BOOLEAN NOT NULL,
  server_id BIGINTEGER
);

INSERT INTO notebooks SELECT id, COALESCE(author_id,0), title, data, is_pending, server_id from old_notebooks;
DROP TABLE old_notebooks;
