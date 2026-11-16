CREATE TABLE welcome_panes (
  id INTEGER PRIMARY KEY NOT NULL,
  kind TEXT NOT NULL DEFAULT 'welcome' CHECK (kind = 'welcome'),
  startup_directory TEXT,
  FOREIGN KEY (id, kind) REFERENCES pane_leaves (pane_node_id, kind)
);
