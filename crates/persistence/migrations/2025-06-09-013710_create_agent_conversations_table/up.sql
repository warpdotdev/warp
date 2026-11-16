CREATE TABLE agent_conversations (
    id INTEGER PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL,
    active_task_id TEXT,
    conversation_data TEXT NOT NULL,
    last_modified_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TRIGGER update_last_modified_at_for_agent_conversations AFTER
UPDATE ON agent_conversations FOR EACH ROW WHEN NEW.last_modified_at IS OLD.last_modified_at BEGIN
UPDATE agent_conversations
SET
    last_modified_at = CURRENT_TIMESTAMP
WHERE
    id = OLD.id;

END;

CREATE UNIQUE INDEX ux_agent_conversations_conversation_id ON agent_conversations (conversation_id);

CREATE TABLE agent_tasks (
    id INTEGER PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    task BLOB NOT NULL,
    last_modified_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (conversation_id) REFERENCES agent_conversations (conversation_id)
);

CREATE TRIGGER update_last_modified_at_for_agent_tasks AFTER
UPDATE ON agent_tasks FOR EACH ROW WHEN NEW.last_modified_at IS OLD.last_modified_at BEGIN
UPDATE agent_tasks
SET
    last_modified_at = CURRENT_TIMESTAMP
WHERE
    id = OLD.id;

END;

CREATE UNIQUE INDEX ux_agent_tasks_task_id ON agent_tasks (task_id);
