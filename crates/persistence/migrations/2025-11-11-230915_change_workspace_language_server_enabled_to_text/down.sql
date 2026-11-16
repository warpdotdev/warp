-- Revert enabled column back to BOOLEAN

CREATE TABLE workspace_language_server_new (
    id integer NOT NULL PRIMARY KEY,
    workspace_id integer NOT NULL,
    language_server_name TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT 0,
    FOREIGN KEY (workspace_id) REFERENCES workspace_metadata (id)
);

-- Copy data from current table, converting text to bool
INSERT INTO workspace_language_server_new (id, workspace_id, language_server_name, enabled)
SELECT id, workspace_id, language_server_name, CASE WHEN enabled = '"Yes"' THEN 1 ELSE 0 END
FROM workspace_language_server;

-- Drop current table
DROP TABLE workspace_language_server;

-- Rename new table to original name
ALTER TABLE workspace_language_server_new RENAME TO workspace_language_server;

-- This file should undo anything in `up.sql`
