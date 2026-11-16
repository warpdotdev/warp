ALTER TABLE windows ADD COLUMN fullscreen BOOLEAN NOT NULL DEFAULT 0;

UPDATE windows SET fullscreen = CASE fullscreen_state WHEN 2 THEN 0 ELSE fullscreen_state END;

ALTER TABLE windows DROP COLUMN fullscreen_state;
