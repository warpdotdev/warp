CREATE TABLE ai_document_panes (
    id INTEGER PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL DEFAULT 'ai_document' CHECK (kind = 'ai_document'),
    document_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    FOREIGN KEY (id, kind) REFERENCES pane_leaves (pane_node_id, kind)
);
