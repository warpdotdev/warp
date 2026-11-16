-- Default value of 0 should be okay for creator_id as its not actually used anywhere.
ALTER TABLE teams ADD creator_id BIGINTEGER NOT NULL DEFAULT 0;
