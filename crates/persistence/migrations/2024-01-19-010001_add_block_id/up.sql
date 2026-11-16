-- SQLite doesn't have a built-in UUID function
ALTER TABLE blocks ADD COLUMN block_id TEXT NOT NULL DEFAULT "";
