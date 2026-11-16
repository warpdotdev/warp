CREATE TABLE active_mcp_servers (
    id INTEGER PRIMARY KEY NOT NULL,
    mcp_server_uuid TEXT NOT NULL,
    UNIQUE(mcp_server_uuid)
);
