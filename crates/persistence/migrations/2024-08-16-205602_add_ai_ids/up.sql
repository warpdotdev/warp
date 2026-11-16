CREATE TABLE ai_exchanges_new_cols (
  id INTEGER PRIMARY KEY NOT NULL,
  exchange_id TEXT NOT NULL,
  conversation_id TEXT NOT NULL,
  start_ts DATETIME NOT NULL,
  -- FOREIGN KEY REFERENCES pane_leaves(uuid) but we don't mark it as a foreign key because it causes problems with cascading deletes.
  pane_leaf_uuid BLOB NOT NULL,
  output TEXT NOT NULL,
  input TEXT NOT NULL,
  working_directory TEXT
);

DROP TABLE ai_exchanges;
ALTER TABLE ai_exchanges_new_cols RENAME TO ai_exchanges;
