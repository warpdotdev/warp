CREATE TABLE ai_memory_panes (
  id INTEGER PRIMARY KEY NOT NULL,
  kind TEXT NOT NULL DEFAULT 'ai_memory' CHECK (kind = 'ai_memory'),

  FOREIGN KEY (id, kind) REFERENCES pane_leaves (pane_node_id, kind)
);
