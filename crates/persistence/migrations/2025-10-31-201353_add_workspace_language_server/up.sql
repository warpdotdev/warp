-- Rename codebase_index_metadata to workspace_metadata and add ID primary key
CREATE TABLE workspace_metadata (
    id integer NOT NULL PRIMARY KEY,
    repo_path TEXT NOT NULL UNIQUE,
    navigated_ts DATETIME,
    modified_ts DATETIME,
    queried_ts DATETIME
);

-- Copy data from old table to new table
INSERT INTO workspace_metadata (repo_path, navigated_ts, modified_ts, queried_ts)
SELECT repo_path, navigated_ts, modified_ts, queried_ts
FROM codebase_index_metadata;

-- Drop the old table
DROP TABLE codebase_index_metadata;

-- Create workspace_language_server table
CREATE TABLE workspace_language_server (
    id integer NOT NULL PRIMARY KEY,
    workspace_id integer NOT NULL,
    language_server_name TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT 0,
    FOREIGN KEY (workspace_id) REFERENCES workspace_metadata (id)
);
