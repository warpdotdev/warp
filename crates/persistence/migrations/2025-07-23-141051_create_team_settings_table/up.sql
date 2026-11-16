CREATE TABLE team_settings (
    id INTEGER PRIMARY KEY NOT NULL,
    team_id INTEGER NOT NULL UNIQUE,
    settings_json TEXT NOT NULL,
    FOREIGN KEY (team_id) REFERENCES teams (id)
);
