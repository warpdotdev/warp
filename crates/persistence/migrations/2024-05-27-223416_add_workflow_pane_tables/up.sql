-- This adds a table for restoring open EVC panes.
CREATE TABLE workflow_panes (
  id INTEGER PRIMARY KEY NOT NULL,
  kind TEXT NOT NULL DEFAULT 'workflow' CHECK (kind = 'workflow'),

  -- The sync ID of the EVC. This may be null if the EVC has not yet been saved.
  workflow_id TEXT,
  
  FOREIGN KEY (id, kind) REFERENCES pane_leaves (pane_node_id, kind)
);
