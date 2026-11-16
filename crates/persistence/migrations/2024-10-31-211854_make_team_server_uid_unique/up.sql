CREATE TABLE IF NOT EXISTS teams_new (
  id integer NOT NULL PRIMARY KEY,
  server_id BIGINTEGER UNIQUE,
  name TEXT NOT NULL,
  server_uid TEXT UNIQUE -- Added unique constraint
);

INSERT INTO teams_new SELECT DISTINCT * FROM teams;

DROP TABLE teams;

ALTER TABLE teams_new RENAME TO teams;
