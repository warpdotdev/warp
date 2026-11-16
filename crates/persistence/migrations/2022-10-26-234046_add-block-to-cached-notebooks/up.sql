ALTER TABLE notebooks ADD title TEXT;

CREATE TABLE notebook_blocks (
    id INTEGER NOT NULL PRIMARY KEY,
    notebook_id INTEGER NOT NULL,
    is_documentation BOOLEAN NOT NULL,
    data TEXT NOT NULL,
    FOREIGN KEY(notebook_id) REFERENCES notebooks(id)
);
