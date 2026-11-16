ALTER TABLE windows ADD COLUMN fullscreen_state INTEGER NOT NULL DEFAULT 0;

UPDATE windows SET fullscreen_state = fullscreen;

ALTER TABLE windows DROP COLUMN fullscreen;
