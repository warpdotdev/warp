-- Rebuildable index of CLI-agent sessions ("task handles"). The agent's own
-- transcript directory (e.g. ~/.claude/projects) is the source of truth; this
-- table may be dropped and re-derived from it.
CREATE TABLE agent_session_handles (
    id            INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    agent         TEXT    NOT NULL,
    session_id    TEXT,
    cwd           TEXT    NOT NULL,
    pane_uuid     BINARY  NOT NULL,
    title         TEXT,
    created_at    TIMESTAMP NOT NULL,
    last_seen_at  TIMESTAMP NOT NULL
);

-- Task identity: one row per upstream session, whichever pane resumed it.
CREATE UNIQUE INDEX idx_ash_task
    ON agent_session_handles (agent, session_id) WHERE session_id IS NOT NULL;

-- In-flight slot: at most one un-identified launch per pane per agent.
CREATE UNIQUE INDEX idx_ash_inflight
    ON agent_session_handles (pane_uuid, agent) WHERE session_id IS NULL;

CREATE INDEX idx_ash_recent ON agent_session_handles (last_seen_at);
