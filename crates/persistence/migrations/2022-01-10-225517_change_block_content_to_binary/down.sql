DROP TABLE blocks;

CREATE TABLE blocks (
    id INTEGER PRIMARY KEY,
    pane_leaf_uuid BLOB NOT NULL,
    stylized_command TEXT NOT NULL,
    stylized_output TEXT NOT NULL,
    pwd TEXT,
    git_branch TEXT,
    virtual_env TEXT,
    conda_env TEXT,
    exit_code INTEGER NOT NULL,
    did_execute BOOLEAN NOT NULL
);
