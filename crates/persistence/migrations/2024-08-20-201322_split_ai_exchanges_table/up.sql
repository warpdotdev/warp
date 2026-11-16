CREATE TABLE ai_blocks (
    id INTEGER PRIMARY KEY NOT NULL,
    exchange_id TEXT NOT NULL,
    -- Would be marked FOREIGN KEY REFERENCES pane_leaves(uuid) but we don't because we can't enforce it properly when handling pane removal.
    pane_leaf_uuid BLOB NOT NULL,
    output TEXT NOT NULL,
    FOREIGN KEY(exchange_id) REFERENCES ai_exchanges(exchange_id)
);

ALTER TABLE ai_exchanges DROP COLUMN pane_leaf_uuid;
ALTER TABLE ai_exchanges DROP COLUMN output;
ALTER TABLE ai_exchanges ADD COLUMN output_status TEXT NOT NULL;
ALTER TABLE ai_exchanges RENAME TO ai_queries;
