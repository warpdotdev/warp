-- Adds session restoration info for local notebooks.
-- File paths aren't guaranteed to be UTF-8, so we store them as blobs.
ALTER TABLE notebook_panes ADD COLUMN local_path BLOB;
