CREATE TABLE code_review_panes (
  id INTEGER PRIMARY KEY NOT NULL,
  kind TEXT NOT NULL DEFAULT 'code_review' CHECK (kind = 'code_review'),

  -- The UUID of the terminal this code review pane is associated with, for attaching context.
  terminal_uuid BLOB NOT NULL,

  -- The repository path the code review pane was created for.
  repo_path TEXT NOT NULL,

  FOREIGN KEY (id, kind) REFERENCES pane_leaves (pane_node_id, kind)
);
