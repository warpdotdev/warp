ALTER TABLE cloud_objects_refreshes DROP description;
ALTER TABLE cloud_objects_refreshes DROP refreshed;

ALTER TABLE cloud_objects_refreshes ADD COLUMN time_of_next_refresh DATETIME NOT NULL;
