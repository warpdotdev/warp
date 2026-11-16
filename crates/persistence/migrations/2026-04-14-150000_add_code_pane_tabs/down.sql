-- Re-add the columns that were dropped.
ALTER TABLE code_panes ADD COLUMN local_path BLOB;
ALTER TABLE code_panes DROP COLUMN source_data;
ALTER TABLE code_panes DROP COLUMN active_tab_index;

DROP TABLE IF EXISTS code_pane_tabs;
