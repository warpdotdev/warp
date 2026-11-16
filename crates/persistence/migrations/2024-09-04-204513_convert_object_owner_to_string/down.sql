-- Convert the `object_permissions.subject_id` column back from text to an integer.

CREATE TABLE IF NOT EXISTS object_permissions_old (
  id INTEGER NOT NULL PRIMARY KEY,
  object_metadata_id INTEGER NOT NULL REFERENCES object_metadata(id) ON DELETE CASCADE,
  subject_type TEXT NOT NULL,
  -- This can be null in the case where the subject is the current user, and we haven't yet backfilled it.
  subject_id INTEGER,
  permissions_last_updated_at BIGINTEGER
);

INSERT INTO object_permissions_old SELECT * FROM object_permissions;

DROP TABLE object_permissions;

ALTER TABLE object_permissions_old RENAME TO object_permissions;
