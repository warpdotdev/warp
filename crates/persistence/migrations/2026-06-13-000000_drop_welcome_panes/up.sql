-- Drop the welcome_panes table first to remove the FK constraint that
-- references pane_leaves, so we can then clean up the pane_leaves rows.
DROP TABLE IF EXISTS welcome_panes;

-- Delete pane_leaves rows for welcome panes. These would otherwise cause
-- "Unrecognized pane kind: welcome" errors in read_node during restoration.
DELETE FROM pane_leaves WHERE kind = 'welcome';

-- Delete the now-orphaned leaf pane_nodes (those with no corresponding
-- pane_leaves row).
DELETE FROM pane_nodes
    WHERE is_leaf = 1
    AND id NOT IN (SELECT pane_node_id FROM pane_leaves);
