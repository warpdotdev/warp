CREATE TABLE IF NOT EXISTS teams_new (
  id integer NOT NULL PRIMARY KEY,
  name TEXT NOT NULL,
  server_uid TEXT NOT NULL UNIQUE
);

INSERT INTO teams_new (id, name, server_uid)
SELECT id, name, server_uid
FROM teams
-- Some old team rows don't have a server_uid; it should be safe to ignore them
-- because there's a duplicate row that DOES have a server_uid for the same team.
-- Even if there isn't, the server will refresh their rows when they're online.
WHERE server_uid IS NOT NULL;

DROP TABLE teams;

ALTER TABLE teams_new RENAME TO teams;
