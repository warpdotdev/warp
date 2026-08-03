CREATE TABLE tab_groups (
  id INTEGER PRIMARY KEY NOT NULL,
  window_id INTEGER NOT NULL,
  name TEXT,
  color TEXT,
  collapsed BOOLEAN NOT NULL,
  FOREIGN KEY(window_id) REFERENCES windows(id)
);

ALTER TABLE tabs ADD COLUMN tab_group_id INTEGER REFERENCES tab_groups(id);
