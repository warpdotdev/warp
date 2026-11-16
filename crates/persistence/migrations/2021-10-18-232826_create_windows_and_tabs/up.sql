CREATE TABLE windows (
  id INTEGER PRIMARY KEY NOT NULL,
  active_tab_index INTEGER NOT NULL CHECK (active_tab_index >= 0),
  window_width FLOAT CHECK (window_width >= 0),
  window_height FLOAT CHECK (window_height >= 0),
  origin_x FLOAT,
  origin_y FLOAT,
  CONSTRAINT Bound_integrity CHECK (
    COALESCE(window_width, window_height, origin_x, origin_y) IS NOT NULL
    OR COALESCE(window_width, window_height, origin_x, origin_y) IS NULL
  )
);

CREATE TABLE tabs (
  id INTEGER PRIMARY KEY NOT NULL,
  window_id INTEGER NOT NULL,
  cwd TEXT,
  FOREIGN KEY(window_id) REFERENCES windows(id)
);
