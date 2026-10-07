use warp_multi_agent_api as api;

pub fn create_message(id: &str, task_id: &str) -> api::Message {
    api::Message {
        fetched_memories: vec![],
        id: id.to_string(),
        task_id: task_id.to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::AgentOutput(
            api::message::AgentOutput {
                text: format!("Message content for {id}"),
            },
        )),
        request_id: String::new(),
        timestamp: None,
    }
}

pub fn create_subagent_tool_call_message(
    id: &str,
    task_id: &str,
    subtask_id: &str,
    metadata: Option<api::message::tool_call::subagent::Metadata>,
) -> api::Message {
    api::Message {
        fetched_memories: vec![],
        id: id.to_string(),
        task_id: task_id.to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::ToolCall(api::message::ToolCall {
            tool_call_id: format!("{id}_tool_call"),
            tool: Some(api::message::tool_call::Tool::Subagent(
                api::message::tool_call::Subagent {
                    task_id: subtask_id.to_string(),
                    payload: String::new(),
                    metadata,
                },
            )),
        })),
        request_id: String::new(),
        timestamp: None,
    }
}

pub fn create_api_task(id: &str, messages: Vec<api::Message>) -> api::Task {
    api::Task {
        id: id.to_string(),
        messages,
        dependencies: None,
        description: String::new(),
        summary: String::new(),
        server_data: String::new(),
    }
}

pub fn create_api_subtask(
    id: &str,
    parent_task_id: &str,
    messages: Vec<api::Message>,
) -> api::Task {
    api::Task {
        id: id.to_string(),
        messages,
        dependencies: Some(api::task::Dependencies {
            parent_task_id: parent_task_id.to_string(),
        }),
        description: String::new(),
        summary: String::new(),
        server_data: String::new(),
    }
}
