ALTER TABLE cloud_objects_refreshes DROP COLUMN time_of_next_refresh;

ALTER TABLE cloud_objects_refreshes ADD COLUMN description TEXT;
ALTER TABLE cloud_objects_refreshes ADD COLUMN refreshed BOOLEAN NOT NULL;
