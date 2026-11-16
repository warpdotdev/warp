ALTER TABLE notebooks RENAME TO old_notebooks;
CREATE TABLE notebooks (
  id INTEGER NOT NULL PRIMARY KEY,
  author_id INTEGER,
  title TEXT,
  data TEXT,
  client_id TEXT,
  is_pending BOOLEAN NOT NULL,
  server_id BIGINTEGER
);

INSERT INTO notebooks SELECT id, author_id, title, data, null, is_pending, server_id from old_notebooks;
DROP TABLE old_notebooks;
