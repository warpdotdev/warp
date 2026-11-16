-- This adds a table for restoring open notebook panes.
CREATE TABLE notebook_panes (
  id INTEGER PRIMARY KEY NOT NULL,
  kind TEXT NOT NULL DEFAULT 'notebook' CHECK (kind = 'notebook'),

  -- The sync ID of the notebook. This may be null if the notebook has not yet been saved.
  notebook_id TEXT,
  
  FOREIGN KEY (id, kind) REFERENCES pane_leaves (pane_node_id, kind)
);
