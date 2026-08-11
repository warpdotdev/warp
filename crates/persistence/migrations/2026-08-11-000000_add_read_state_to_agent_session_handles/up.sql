-- Read/unread acknowledgement for task handles, so a session's green "unseen
-- result" state (and any manual unread mark) survives the pane and app restarts.
ALTER TABLE agent_session_handles
    ADD COLUMN success_seen BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE agent_session_handles
    ADD COLUMN marked_unread BOOLEAN NOT NULL DEFAULT 0;
