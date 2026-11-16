-- Notebook panes are new, so we can undo the migration by removing them completely.
-- To avoid foreign key errors, we have to delete any saved notebook panes.

DELETE FROM pane_leaves WHERE kind = 'notebook';
DROP TABLE notebook_panes;
