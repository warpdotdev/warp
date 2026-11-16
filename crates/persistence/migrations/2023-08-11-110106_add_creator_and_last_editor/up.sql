ALTER TABLE object_metadata DROP last_edited_by;
ALTER TABLE object_metadata ADD creator_uid TEXT;
ALTER TABLE object_metadata ADD last_editor_uid TEXT;
