CREATE TABLE project_rules (
    id INTEGER NOT NULL PRIMARY KEY,
    path TEXT NOT NULL,
    project_root TEXT NOT NULL
);

CREATE UNIQUE INDEX idx_project_rules_path_unique ON project_rules(path);
