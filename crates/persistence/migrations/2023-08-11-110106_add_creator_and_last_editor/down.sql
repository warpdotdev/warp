ALTER TABLE object_metadata ADD last_edited_by TEXT;
ALTER TABLE object_metadata DROP creator_uid;
ALTER TABLE object_metadata DROP last_editor_uid;
