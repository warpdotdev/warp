-- Add an index on object_permissions(subject_type).
--
-- Without this index, SQLite builds a transient automatic index whenever a query
-- filters or joins object_permissions on subject_type, which emits the noisy
-- SQLITE_WARNING_AUTOINDEX diagnostic (error 284:
-- "automatic index on object_permissions(subject_type)"). Materializing the
-- index removes the warning and avoids the per-query autoindex work.
CREATE INDEX IF NOT EXISTS idx_object_permissions_subject_type
  ON object_permissions(subject_type);
