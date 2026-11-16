-- This file should undo anything in `up.sql`

-- Drop the new workspace_language_server table
DROP TABLE workspace_language_server;

-- Recreate the original codebase_index_metadata table
CREATE TABLE codebase_index_metadata (
    repo_path TEXT NOT NULL PRIMARY KEY,
    navigated_ts DATETIME,
    modified_ts DATETIME,
    queried_ts DATETIME
);

-- Copy data back from workspace_metadata to codebase_index_metadata
INSERT INTO codebase_index_metadata (repo_path, navigated_ts, modified_ts, queried_ts)
SELECT repo_path, navigated_ts, modified_ts, queried_ts
FROM workspace_metadata;

-- Drop the workspace_metadata table
DROP TABLE workspace_metadata;
