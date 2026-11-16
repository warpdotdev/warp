DROP TABLE blocks;

ALTER TABLE pane_leaves RENAME TO old_pane_leaves;

CREATE TABLE pane_leaves (
  id INTEGER PRIMARY KEY NOT NULL,
  pane_node_id INTEGER NOT NULL UNIQUE REFERENCES pane_nodes(id),
  cwd TEXT,
  is_active BOOLEAN NOT NULL DEFAULT FALSE
);

-- Select 1, 2, 3, ... for the ID.
INSERT INTO pane_leaves SELECT (ROW_NUMBER() OVER (ORDER BY (SELECT 0))), pane_node_id, cwd, is_active from old_pane_leaves;
DROP TABLE old_pane_leaves;
