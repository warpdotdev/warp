CREATE TABLE workflows (
    id INTEGER NOT NULL PRIMARY KEY,
    -- Diesel does not let you specify JSON as data type
    data TEXT NOT NULL,
    last_updated_at DATETIME NOT NULL,
    -- `user_id` column helps to support multiple users logged into same device.
    -- It ensures the user is only served data meant for their Warp account.
    user_id INTEGER NOT NULL REFERENCES users(id)
);