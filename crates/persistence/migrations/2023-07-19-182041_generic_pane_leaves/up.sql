-- Schema change approach from https://www.sqlite.org/lang_altertable.html#making_other_kinds_of_table_schema_changes

PRAGMA foreign_keys = off;

CREATE TABLE new_pane_leaves (
    pane_node_id INTEGER NOT NULL UNIQUE REFERENCES pane_nodes(id),
    -- This does not have a CHECK constraint because, when we add new kinds of panes in the future,
    -- it's difficult to update the constraint.
    kind TEXT NOT NULL,
    is_focused BOOLEAN NOT NULL DEFAULT FALSE,

    PRIMARY KEY (pane_node_id, kind)
);

CREATE TABLE terminal_panes (
    id INTEGER PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL DEFAULT 'terminal' CHECK (kind = 'terminal'),

    uuid BLOB NOT NULL UNIQUE,
    cwd TEXT,
    is_active BOOLEAN NOT NULL DEFAULT FALSE,

    FOREIGN KEY (id, kind) REFERENCES new_pane_leaves(pane_node_id, kind)
);

INSERT INTO new_pane_leaves (pane_node_id, kind, is_focused)
    SELECT pane_node_id, 'terminal' as kind, is_active as is_focused
    FROM pane_leaves;

INSERT INTO terminal_panes (id, uuid, cwd, is_active)
    SELECT pane_node_id, uuid, cwd, is_active
    FROM pane_leaves;

DROP TABLE pane_leaves;
ALTER TABLE new_pane_leaves RENAME TO pane_leaves;

PRAGMA foreign_key_check;
PRAGMA foreign_keys = on;
