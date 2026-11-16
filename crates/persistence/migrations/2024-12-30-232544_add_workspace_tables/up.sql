-- Your SQL goes here
CREATE TABLE workspaces (
    id integer NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    server_uid TEXT NOT NULL UNIQUE
);

CREATE TABLE workspace_teams (
    id integer NOT NULL PRIMARY KEY,
    workspace_server_uid TEXT NOT NULL UNIQUE,
    team_server_uid TEXT NOT NULL UNIQUE,
    FOREIGN KEY (workspace_server_uid) REFERENCES workspaces (server_uid),
    FOREIGN KEY (team_server_uid) REFERENCES teams (server_uid)
);
