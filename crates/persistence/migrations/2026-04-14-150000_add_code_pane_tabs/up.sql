-- Recreate code_panes with a proper PRIMARY KEY and the final column set.
-- The original table was created without an explicit PRIMARY KEY on `id`,
-- which prevents SQLite foreign keys from referencing it.
CREATE TABLE code_panes_new (
  id INTEGER PRIMARY KEY NOT NULL,
  active_tab_index INTEGER NOT NULL DEFAULT 0,
  source_data TEXT
);

INSERT INTO code_panes_new (id)
SELECT id FROM code_panes;

-- Create the code_pane_tabs table before dropping code_panes,
-- so we can backfill from the old local_path column.
CREATE TABLE code_pane_tabs (
  id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
  code_pane_id INTEGER NOT NULL,
  tab_index INTEGER NOT NULL,
  local_path BLOB,

  FOREIGN KEY (code_pane_id) REFERENCES code_panes_new (id) ON DELETE CASCADE,
  UNIQUE (code_pane_id, tab_index)
);

-- Backfill: for each existing code_panes row that has a local_path,
-- insert a single code_pane_tabs row at tab_index 0.
INSERT INTO code_pane_tabs (code_pane_id, tab_index, local_path)
SELECT id, 0, local_path
FROM code_panes
WHERE local_path IS NOT NULL;

-- Swap tables.
DROP TABLE code_panes;
ALTER TABLE code_panes_new RENAME TO code_panes;
