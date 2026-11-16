-- First, change panes table to have a uuid as primary key.
ALTER TABLE pane_leaves RENAME TO old_pane_leaves;
CREATE TABLE pane_leaves (
  uuid BLOB PRIMARY KEY NOT NULL,
  pane_node_id INTEGER NOT NULL UNIQUE REFERENCES pane_nodes(id),
  cwd TEXT,
  is_active BOOLEAN NOT NULL DEFAULT FALSE
);

INSERT INTO pane_leaves SELECT id, pane_node_id, cwd, is_active from old_pane_leaves;

-- Drop the old pane leaves table.
DROP TABLE old_pane_leaves;

-- Now we can create a blocks table that has pane_leaf_uuid as a foreign key.
-- We don't establish a formal relationship for reasons documented in the code.
CREATE TABLE blocks (
	id INTEGER PRIMARY KEY,
	pane_leaf_uuid BLOB NOT NULL,
	stylized_command TEXT NOT NULL,
	stylized_output TEXT NOT NULL,
	pwd TEXT,
    git_branch TEXT,
    virtual_env TEXT,
    conda_env TEXT,
    exit_code INTEGER NOT NULL,
    did_execute BOOLEAN NOT NULL
);
