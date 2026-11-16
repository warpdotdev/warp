ALTER TABLE object_metadata ADD COLUMN team_id INTEGER;

-- Backfilling team_id column with values from object_permissions
UPDATE object_metadata
    SET team_id = object_permissions.subject_id
    FROM object_permissions
    WHERE object_permissions.subject_type = 'TEAM'
    AND object_metadata.id = object_permissions.object_metadata_id;

-- Dropping object_permissions table
DROP TABLE IF EXISTS object_permissions;
