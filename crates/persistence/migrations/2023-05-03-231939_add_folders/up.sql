CREATE TABLE folders (
    id INTEGER NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    is_open BOOLEAN NOT NULL
);

ALTER TABLE object_metadata ADD folder_id TEXT;
