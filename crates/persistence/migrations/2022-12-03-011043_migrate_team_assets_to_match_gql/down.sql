ALTER TABLE workflows DROP COLUMN user_can_delete;
ALTER TABLE workflows DROP COLUMN is_pending;
ALTER TABLE notebooks DROP COLUMN is_pending;

ALTER TABLE workflows DROP COLUMN server_id;
ALTER TABLE notebooks DROP COLUMN server_id;
