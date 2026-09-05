use warp_multi_agent_api as api;

use super::MessageExt;

#[test]
fn message_type_name_identifies_client_representation_categories() {
    let mut message = api::Message::default();
    assert_eq!(message.type_name(), "missing");

    message.message = Some(api::message::Message::AgentOutput(Default::default()));
    assert_eq!(message.type_name(), "agent_output");

    message.message = Some(api::message::Message::ModelUsed(Default::default()));
    assert_eq!(message.type_name(), "model_used");

    message.message = Some(api::message::Message::ToolCall(Default::default()));
    assert_eq!(message.type_name(), "tool_call");
}
