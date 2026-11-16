CREATE TABLE ai_blocks (
    id INTEGER PRIMARY KEY NOT NULL,
    exchange_id TEXT NOT NULL,
    pane_leaf_uuid BLOB NOT NULL,
    output TEXT NOT NULL,
    FOREIGN KEY(exchange_id) REFERENCES ai_queries(exchange_id)
);
