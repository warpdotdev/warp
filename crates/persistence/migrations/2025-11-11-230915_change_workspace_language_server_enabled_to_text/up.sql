-- Change enabled column from BOOLEAN to TEXT to store EnablementState enum
-- Since no one is using this feature yet, we can simply drop and recreate the table

DROP TABLE workspace_language_server;

CREATE TABLE workspace_language_server (
    id integer NOT NULL PRIMARY KEY,
    workspace_id integer NOT NULL,
    language_server_name TEXT NOT NULL,
    enabled TEXT NOT NULL,
    FOREIGN KEY (workspace_id) REFERENCES workspace_metadata (id)
);
