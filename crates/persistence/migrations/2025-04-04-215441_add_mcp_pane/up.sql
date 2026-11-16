CREATE TABLE mcp_server_panes (
  id INTEGER PRIMARY KEY NOT NULL,
  kind TEXT NOT NULL DEFAULT 'mcp_server' CHECK (kind = 'mcp_server'),

  FOREIGN KEY (id, kind) REFERENCES pane_leaves (pane_node_id, kind)
);
