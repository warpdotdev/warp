DROP TABLE IF EXISTS cloud_objects_refreshes;
DROP TABLE IF EXISTS object_actions;
DROP TABLE IF EXISTS object_permissions;
DROP TABLE IF EXISTS object_metadata;
DROP TABLE IF EXISTS generic_string_objects;
DROP TABLE IF EXISTS team_members;
DROP TABLE IF EXISTS team_settings;
DROP TABLE IF EXISTS workspace_teams;
DROP TABLE IF EXISTS teams;

-- This migration is marked run_in_transaction=false so this pragma applies
-- while windows is rebuilt with the same local IDs for app/tabs references.
PRAGMA foreign_keys=off;

CREATE TABLE windows_local (
    id INTEGER NOT NULL PRIMARY KEY,
    active_tab_index INTEGER NOT NULL,
    window_width FLOAT,
    window_height FLOAT,
    origin_x FLOAT,
    origin_y FLOAT,
    quake_mode BOOLEAN NOT NULL,
    universal_search_width FLOAT,
    warp_ai_width FLOAT,
    voltron_width FLOAT,
    fullscreen_state INTEGER NOT NULL,
    agent_management_filters TEXT,
    left_panel_open BOOLEAN,
    vertical_tabs_panel_open BOOLEAN
);

INSERT INTO windows_local (
    id,
    active_tab_index,
    window_width,
    window_height,
    origin_x,
    origin_y,
    quake_mode,
    universal_search_width,
    warp_ai_width,
    voltron_width,
    fullscreen_state,
    agent_management_filters,
    left_panel_open,
    vertical_tabs_panel_open
)
SELECT
    id,
    active_tab_index,
    window_width,
    window_height,
    origin_x,
    origin_y,
    quake_mode,
    universal_search_width,
    warp_ai_width,
    voltron_width,
    fullscreen_state,
    agent_management_filters,
    left_panel_open,
    vertical_tabs_panel_open
FROM windows;

DROP TABLE windows;
ALTER TABLE windows_local RENAME TO windows;

PRAGMA foreign_keys=on;
