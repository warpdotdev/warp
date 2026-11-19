DROP INDEX ux_agent_tasks_task_id;
CREATE UNIQUE INDEX ux_agent_tasks_conversation_id_task_id ON agent_tasks (conversation_id, task_id);
