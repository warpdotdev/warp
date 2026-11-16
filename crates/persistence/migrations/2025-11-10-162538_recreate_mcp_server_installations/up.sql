DROP TABLE IF EXISTS mcp_server_installations;

CREATE TABLE mcp_server_installations (
    id TEXT NOT NULL PRIMARY KEY,
    templatable_mcp_server TEXT NOT NULL,
    template_version_ts TIMESTAMP NOT NULL,
    variable_values TEXT NOT NULL,
    restore_running BOOLEAN NOT NULL,
    last_modified_at TIMESTAMP NOT NULL
);
