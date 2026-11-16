CREATE TABLE orchestration_events (
    id INTEGER NOT NULL PRIMARY KEY,
    event_id TEXT NOT NULL UNIQUE,
    event_type TEXT NOT NULL,
    source_agent_id TEXT NOT NULL,
    target_agent_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    delivered INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE orchestration_messages (
    id INTEGER NOT NULL PRIMARY KEY,
    event_id TEXT NOT NULL REFERENCES orchestration_events(event_id),
    message_id TEXT NOT NULL UNIQUE,
    sender_agent_id TEXT NOT NULL,
    address_agent_ids TEXT NOT NULL,
    subject TEXT NOT NULL,
    message_body TEXT NOT NULL
);
