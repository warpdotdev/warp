-- Agent session state lives outside the pane snapshot tables on purpose: those are deleted and
-- rebuilt wholesale by every session save, including the save that restore itself triggers, so a
-- row stored there would be erased before it could be read back.
CREATE TABLE IF NOT EXISTS agent_sessions (
    id INTEGER PRIMARY KEY NOT NULL,
    -- No foreign key to pane_leaves for the same reason blocks has none: pane rows are recreated
    -- by every snapshot, which would leave these rows violating the constraint.
    pane_leaf_uuid BLOB NOT NULL,
    -- Nullable because the writer degrades an unserializable value to NULL rather than failing;
    -- a NOT NULL column would turn that into a constraint error.
    agent_kind TEXT,
    session_id TEXT NOT NULL,
    flags TEXT,
    directory BLOB NOT NULL,
    observed_at TIMESTAMP NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS ux_agent_sessions_pane_leaf_uuid ON agent_sessions (pane_leaf_uuid);
