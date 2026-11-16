-- This file should undo anything in `up.sql`

-- Create new table
CREATE TABLE object_permissions_new (
  id INTEGER NOT NULL PRIMARY KEY,
  object_metadata_id INTEGER NOT NULL REFERENCES object_metadata(id) ON DELETE CASCADE,
  subject_type TEXT NOT NULL,
  -- This can be null in the case where the subject is a user. We don't know
  -- the user's ID so we currently are not able to backfill this field.
  -- We treat this case as though the current user is implicitly granted access.
  subject_id INTEGER,
  permissions_last_updated_at BIGINTEGER NOT NULL DEFAULT 0
  );

-- Copy values from old table
INSERT INTO object_permissions_new
  SELECT id, object_metadata_id, subject_type, subject_id, permissions_last_updated_at
  FROM object_permissions
  WHERE permissions_last_updated_at IS NOT NULL;

INSERT INTO object_permissions_new
  SELECT id, object_metadata_id, subject_type, subject_id, 0
  FROM object_permissions
  WHERE permissions_last_updated_at IS NULL;

-- Drop old table
DROP TABLE object_permissions;

-- Rename new table to old table name
ALTER TABLE object_permissions_new RENAME TO object_permissions;
