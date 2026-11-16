DROP TABLE ai_blocks;

ALTER TABLE ai_queries ADD COLUMN pane_leaf_uuid BLOB NOT NULL;
ALTER TABLE ai_queries DROP COLUMN output_status;
ALTER TABLE ai_queries ADD COLUMN output TEXT NOT NULL;
ALTER TABLE ai_queries RENAME TO ai_exchanges;
