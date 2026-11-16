DROP TABLE IF EXISTS mcp_server_installations;

CREATE TABLE mcp_server_installations (
    id TEXT NOT NULL PRIMARY KEY,
    template_uuid TEXT NOT NULL,
    template_json TEXT,
    template_version_ts TIMESTAMP NOT NULL,
    variable_values TEXT,
    restore_running BOOLEAN,
    last_modified_at TIMESTAMP NOT NULL
);

CREATE UNIQUE INDEX idx_mcp_server_installations_template_uuid ON mcp_server_installations(template_uuid ASC);
