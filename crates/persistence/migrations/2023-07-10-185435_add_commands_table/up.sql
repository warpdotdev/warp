CREATE TABLE commands (
    id INTEGER NOT NULL PRIMARY KEY,
    command TEXT NOT NULL,
    exit_code INTEGER,
    start_ts DATETIME,
    completed_ts DATETIME,
    pwd TEXT,
    shell TEXT,
    username TEXT,
    hostname TEXT,
    session_id BIGINTEGER,
    git_branch TEXT,
    cloud_workflow_id TEXT
);
