PRAGMA foreign_keys = off;

-- This also follows the required SQLite schema-change approach from
-- https://www.sqlite.org/lang_altertable.html#making_other_kinds_of_table_schema_changes,
-- but using the pre-migration schema.

CREATE TABLE rollback_pane_leaves (
    uuid BLOB PRIMARY KEY NOT NULL,
    pane_node_id INTEGER NOT NULL UNIQUE REFERENCES pane_nodes(id),
    cwd TEXT,
    is_active BOOLEAN NOT NULL DEFAULT FALSE
);

INSERT INTO rollback_pane_leaves (uuid, pane_node_id, cwd, is_active)
    SELECT uuid, id as pane_node_id, cwd, is_active
    FROM terminal_panes;

DROP TABLE terminal_panes;
DROP TABLE pane_leaves;
ALTER TABLE rollback_pane_leaves RENAME TO pane_leaves;

PRAGMA foreign_key_check;
PRAGMA foreign_keys = on;
