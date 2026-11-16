CREATE TABLE code_panes (
  id INTEGER PRIMARY KEY NOT NULL,
  kind TEXT NOT NULL DEFAULT 'code' CHECK (kind = 'code'),

  -- The sync ID of the notebook. This may be null if the notebook has not yet been saved.
  local_path BLOB,

  FOREIGN KEY (id, kind) REFERENCES pane_leaves (pane_node_id, kind)
);
