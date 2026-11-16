DROP TABLE blocks;

CREATE TABLE blocks (
    id INTEGER PRIMARY KEY,
    pane_leaf_uuid BLOB NOT NULL,
    stylized_command BLOB NOT NULL,
    stylized_output BLOB NOT NULL,
    pwd TEXT,
    git_branch TEXT,
    virtual_env TEXT,
    conda_env TEXT,
    exit_code INTEGER NOT NULL,
    did_execute BOOLEAN NOT NULL
);
