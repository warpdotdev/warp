DROP TABLE IF EXISTS notebook_blocks;

DROP TABLE IF EXISTS notebooks;

CREATE TABLE notebooks (
    id INTEGER NOT NULL PRIMARY KEY,
    author_id INTEGER NOT NULL,
    title TEXT,
    data TEXT
);
