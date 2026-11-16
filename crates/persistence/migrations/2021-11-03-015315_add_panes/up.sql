CREATE TABLE pane_nodes (
  id INTEGER PRIMARY KEY NOT NULL,
  tab_id INTEGER NOT NULL REFERENCES tabs(id),
  parent_pane_node_id INTEGER REFERENCES pane_nodes(id),
  flex FLOAT,
  is_leaf BOOLEAN NOT NULL,
  CONSTRAINT root_or_has_parent CHECK (
	parent_pane_node_id IS NULL AND flex IS NULL
	OR parent_pane_node_id IS NOT NULL AND flex IS NOT NULL
  )
);

CREATE TABLE pane_branches (
  id INTEGER PRIMARY KEY NOT NULL,
  pane_node_id INTEGER NOT NULL UNIQUE REFERENCES pane_nodes(id),
  horizontal BOOLEAN NOT NULL
);

CREATE TABLE pane_leaves (
  id INTEGER PRIMARY KEY NOT NULL,
  pane_node_id INTEGER NOT NULL UNIQUE REFERENCES pane_nodes(id),
  cwd TEXT
);

-- Migrate the same tabs data into a pane structure (i.e. each tab has a single
-- pane node that's a leaf).
INSERT INTO pane_nodes (tab_id, is_leaf)
SELECT id AS tab_id, TRUE from tabs;

INSERT INTO pane_leaves (pane_node_id, cwd)
SELECT pane_nodes.id AS pane_node_id, cwd from tabs JOIN pane_nodes ON tab_id = tabs.id;

ALTER TABLE tabs DROP COLUMN cwd;
