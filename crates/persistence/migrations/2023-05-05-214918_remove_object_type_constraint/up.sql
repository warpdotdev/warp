-- The goal is to remove the constraint on object_type.

-- create new table without CHECK constraint
CREATE TABLE object_metadata_new (
    id INTEGER NOT NULL PRIMARY KEY,
    is_pending BOOLEAN NOT NULL,
    object_type TEXT NOT NULL,
    revision_ts INTEGER,
    server_id TEXT,
    client_id TEXT,
    shareable_object_id INTEGER NOT NULL,
    last_edited_by TEXT,
    author_id INTEGER,
    retry_count INTEGER NOT NULL,
    team_id BIGINTEGER,
    metadata_last_updated_ts BIGINTEGER,
    trashed_ts BIGINTEGER,
    folder_id TEXT
);

-- copy data from old table to new table
INSERT INTO object_metadata_new
SELECT id, is_pending, object_type, revision_ts, server_id, client_id, shareable_object_id, last_edited_by, author_id, retry_count, team_id, metadata_last_updated_ts, trashed_ts, folder_id
FROM object_metadata;

-- drop old table
DROP TABLE object_metadata;

-- rename new table to old table name
ALTER TABLE object_metadata_new RENAME TO object_metadata;
