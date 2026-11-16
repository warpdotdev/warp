CREATE TABLE settings_panes (
  id INTEGER PRIMARY KEY NOT NULL,
  kind TEXT NOT NULL DEFAULT 'settings' CHECK (kind = 'settings'),

  current_page TEXT NOT NULL DEFAULT 'Account',

  FOREIGN KEY (id, kind) REFERENCES pane_leaves (pane_node_id, kind)
);
