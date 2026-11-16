CREATE TABLE last_ai_conversations (
    id INTEGER PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL,
    exchanges TEXT NOT NULL,
    phase TEXT NOT NULL,
    has_dispatched_plan BOOLEAN NOT NULL,
    pane_leaf_uuid BLOB NOT NULL
);
