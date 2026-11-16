-- This adds a table for restoring open EVC panes.
CREATE TABLE env_var_collection_panes (
  id INTEGER PRIMARY KEY NOT NULL,
  kind TEXT NOT NULL DEFAULT 'env_var_collection' CHECK (kind = 'env_var_collection'),

  -- The sync ID of the EVC. This may be null if the EVC has not yet been saved.
  env_var_collection_id TEXT,
  
  FOREIGN KEY (id, kind) REFERENCES pane_leaves (pane_node_id, kind)
);
