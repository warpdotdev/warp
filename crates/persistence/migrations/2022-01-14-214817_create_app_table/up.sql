CREATE TABLE app (
    id INTEGER PRIMARY KEY,
    active_window_id INTEGER REFERENCES windows(id)
);
