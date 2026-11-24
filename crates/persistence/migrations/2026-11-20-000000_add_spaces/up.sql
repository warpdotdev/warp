CREATE TABLE spaces (
  id INTEGER PRIMARY KEY NOT NULL,
  window_id INTEGER NOT NULL,
  name TEXT,
  color TEXT,
  position INTEGER NOT NULL,
  active_tab_index INTEGER NOT NULL DEFAULT 0,
  FOREIGN KEY(window_id) REFERENCES windows(id)
);

ALTER TABLE tabs ADD COLUMN space_id INTEGER REFERENCES spaces(id);
ALTER TABLE windows ADD COLUMN active_space_index INTEGER;
