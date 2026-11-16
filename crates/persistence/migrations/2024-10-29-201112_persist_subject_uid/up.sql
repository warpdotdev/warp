CREATE TABLE IF NOT EXISTS object_permissions_new (
  id INTEGER NOT NULL PRIMARY KEY,
  object_metadata_id INTEGER NOT NULL REFERENCES object_metadata(id) ON DELETE CASCADE,
  subject_type TEXT NOT NULL,
  subject_id TEXT,
  subject_uid TEXT NOT NULL,
  permissions_last_updated_at BIGINTEGER,
  object_guests BLOB
);

-- Insert data from `object_permissions` to `object_permissions_new` for `USER` type.
-- For users, `object_permissions.subject_id` is the same as the `subject_uid`.
INSERT INTO object_permissions_new (id, object_metadata_id, subject_type, subject_id, subject_uid, permissions_last_updated_at, object_guests)
SELECT id, object_metadata_id, subject_type, subject_id, subject_id, permissions_last_updated_at, object_guests
FROM object_permissions
WHERE subject_type = 'USER' AND subject_id IS NOT NULL; -- Check for `subject_id` to avoid attempting to insert NULL values for `subject_uid` and failing migration (possible for really old clients that don't have `subject_id` populated).

-- Insert data from `object_permissions` to `object_permissions_new` for `TEAM` type.
-- For teams, join `object_permissions` with `teams` table on `object_permissions.subject_id == teams.server_id`
-- and set `object_permissions.subject_uid = teams.server_uid`.
INSERT INTO object_permissions_new (id, object_metadata_id, subject_type, subject_id, subject_uid, permissions_last_updated_at, object_guests)
SELECT object_permissions.id, object_permissions.object_metadata_id, object_permissions.subject_type, object_permissions.subject_id, teams.server_uid, object_permissions.permissions_last_updated_at, object_permissions.object_guests
FROM object_permissions
JOIN teams ON object_permissions.subject_id = CAST(teams.server_id as CHAR) -- have to cast `teams.server_id` to CHAR because `object_permissions.subject_id` is TEXT
WHERE object_permissions.subject_type = 'TEAM' AND teams.server_uid IS NOT NULL; -- Check for `server_uid` to avoid attempting to insert NULL values for `subject_uid` and failing migration (possible for really old clients that don't have `server_uid` populated).

DROP TABLE object_permissions;

ALTER TABLE object_permissions_new RENAME TO object_permissions;
