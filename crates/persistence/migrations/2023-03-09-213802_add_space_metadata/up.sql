-- Your SQL goes here
CREATE TABLE teams (
    id integer NOT NULL PRIMARY KEY,
    server_id BIGINTEGER UNIQUE,
    name TEXT NOT NULL,
    creator_id BIGINTEGER NOT NULL
);
