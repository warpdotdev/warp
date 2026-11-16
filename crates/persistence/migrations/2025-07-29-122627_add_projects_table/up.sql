CREATE TABLE projects (
    path TEXT NOT NULL PRIMARY KEY,
    added_ts DATETIME NOT NULL,
    last_opened_ts DATETIME
);
