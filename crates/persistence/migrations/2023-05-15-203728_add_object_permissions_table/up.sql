-- Create the object_permissions table
CREATE TABLE object_permissions (
  id INTEGER NOT NULL PRIMARY KEY,
  object_metadata_id INTEGER NOT NULL REFERENCES object_metadata(id) ON DELETE CASCADE,
  subject_type TEXT NOT NULL,
  -- This can be null in the case where the subject is a user. We don't know
  -- the user's ID so we currently are not able to backfill this field.
  -- We treat this case as though the current user is implicitly granted access.
  subject_id INTEGER,
  permissions_last_updated_at BIGINTEGER NOT NULL DEFAULT 0
);

-- Migrate the data from object_metadata to object_permissions
INSERT INTO object_permissions (object_metadata_id, subject_type, subject_id)
SELECT id, 'TEAM', team_id FROM object_metadata WHERE team_id IS NOT NULL;

INSERT INTO object_permissions(object_metadata_id, subject_type)
SELECT id, 'USER' FROM object_metadata WHERE team_id IS NULL;

-- Drop the column from the object_metadata table
ALTER TABLE object_metadata DROP COLUMN team_id;
