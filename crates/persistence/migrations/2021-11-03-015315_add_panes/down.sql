ALTER TABLE tabs ADD cwd TEXT;

UPDATE tabs set cwd = (SELECT pane_leaves.cwd from pane_nodes join pane_leaves where tabs.id = pane_nodes.tab_id order by pane_leaves.id limit 1);

DROP TABLE pane_leaves;
DROP TABLE pane_branches;
DROP TABLE pane_nodes;
