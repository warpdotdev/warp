CREATE INDEX blocks_restore_order ON blocks (pane_leaf_uuid, start_ts DESC, id DESC);
