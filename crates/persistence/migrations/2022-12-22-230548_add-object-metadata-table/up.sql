CREATE TABLE object_metadata (
    id INTEGER NOT NULL PRIMARY KEY,
    is_pending BOOLEAN NOT NULL,
    object_type TEXT CHECK(object_type IN ('NOTEBOOK','WORKFLOW')) NOT NULL,
    revision_ts INTEGER,
    server_id TEXT,
    client_id TEXT,
    shareable_object_id INTEGER NOT NULL,
    last_edited_by TEXT,
    author_id INTEGER,
    retry_count INTEGER NOT NULL
);

INSERT INTO object_metadata SELECT null, is_pending, 'NOTEBOOK', null, null, null, id, null, author_id, 0 from notebooks;
INSERT INTO object_metadata SELECT null, is_pending, 'WORKFLOW', null, null, null, id, null, null, 0 from workflows;

ALTER TABLE workflows DROP COLUMN is_pending;
ALTER TABLE workflows DROP COLUMN server_id;

ALTER TABLE notebooks DROP COLUMN is_pending;
ALTER TABLE notebooks DROP COLUMN author_id;
ALTER TABLE notebooks DROP COLUMN client_id;
ALTER TABLE notebooks DROP COLUMN server_id;
