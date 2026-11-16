-- Create the object_actions table
CREATE TABLE object_actions (
  id INTEGER PRIMARY KEY NOT NULL,
  hashed_object_id TEXT NOT NULL,
  timestamp DATETIME,
  -- An enum here would be overly restrictive for future action types.
  action TEXT NOT NULL,
  data TEXT,
  count INTEGER,
  oldest_timestamp DATETIME,
  latest_timestamp DATETIME,
  pending BOOLEAN
);
