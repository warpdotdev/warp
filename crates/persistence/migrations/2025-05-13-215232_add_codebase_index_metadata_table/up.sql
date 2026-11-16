-- Your SQL goes here
CREATE TABLE codebase_index_metadata (
    repo_path TEXT NOT NULL PRIMARY KEY,
    navigated_ts DATETIME,
    modified_ts DATETIME,
    queried_ts DATETIME
);
