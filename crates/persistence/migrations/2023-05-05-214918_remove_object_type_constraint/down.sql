-- remove rows with where object_type != NOTEBOOK | WORKFLOW
DELETE FROM object_metadata WHERE object_type NOT IN ('NOTEBOOK', 'WORKFLOW');

-- clear folders table (this restores consistency after deleting their metadata)
DELETE FROM folders;

-- create new table with the CHECK constraint back in
CREATE TABLE object_metadata_new (
    id INTEGER NOT NULL PRIMARY KEY,
    is_pending BOOLEAN NOT NULL,
    object_type TEXT CHECK(object_type IN ('NOTEBOOK','WORKFLOW')) NOT NULL,
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

-- migrate data from old table to new table
INSERT INTO object_metadata_new
SELECT id, is_pending, object_type, revision_ts, server_id, client_id, shareable_object_id, last_edited_by, author_id, retry_count, team_id, metadata_last_updated_ts, trashed_ts, folder_id
FROM object_metadata;

-- drop old table
DROP TABLE object_metadata;

-- rename new table to old table name
ALTER TABLE object_metadata_new RENAME TO object_metadata;
