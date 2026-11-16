CREATE TABLE ambient_agent_panes (
    id INTEGER PRIMARY KEY NOT NULL REFERENCES pane_nodes(id),
    kind TEXT NOT NULL DEFAULT 'ambient_agent',
    uuid BLOB NOT NULL,
    task_id TEXT
);
