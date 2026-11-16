DROP TABLE object_metadata;

ALTER TABLE workflows ADD is_pending BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE workflows ADD server_id BIGINTEGER;

ALTER TABLE notebooks ADD is_pending BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE notebooks ADD author_id INTEGER;
ALTER TABLE notebooks ADD client_id TEXT;
ALTER TABLE notebooks ADD server_id BIGINTEGER;
