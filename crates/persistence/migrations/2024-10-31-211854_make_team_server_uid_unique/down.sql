CREATE TABLE IF NOT EXISTS teams_old (
  id integer NOT NULL PRIMARY KEY,
  server_id BIGINTEGER UNIQUE,
  name TEXT NOT NULL,
  server_uid TEXT
);

INSERT INTO teams_old SELECT * FROM teams;

DROP TABLE teams;

ALTER TABLE teams_old RENAME TO teams;
