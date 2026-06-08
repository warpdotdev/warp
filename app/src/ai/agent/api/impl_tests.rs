use std::collections::HashMap;
use std::sync::Arc;

use warp_core::features::FeatureFlag;
use warp_core::HostId;
use warp_multi_agent_api as api;

use super::{
    api_keys_with_warp_credit_fallback_setting, convert_input, get_supported_cli_agent_tools,
    get_supported_tools, maybe_prepend_conversation_handoff, supports_orchestration_v2,
};
use crate::ai::agent::api::{RequestParams, ServerConversationToken};
use crate::ai::agent::{AIAgentInput, UserQueryMode};
use crate::ai::blocklist::SessionContext;
use crate::ai::llms::LLMId;
use crate::terminal::model::session::SessionType;

fn request_params_with_ask_user_question_enabled(ask_user_question_enabled: bool) -> RequestParams {
    let model = LLMId::from("test-model");

    RequestParams {
        input: vec![],
        conversation_token: None,
        forked_from_conversation_token: None,
        ambient_agent_task_id: None,
        tasks: vec![],
        existing_suggestions: None,
        metadata: None,
        session_context: SessionContext::new_for_test(),
        model: model.clone(),
        coding_model: model.clone(),
        cli_agent_model: model.clone(),
        computer_use_model: model,
        is_memory_enabled: false,
        warp_drive_context_enabled: false,
        context_window_limit: None,
        mcp_context: None,
        planning_enabled: true,
        should_redact_secrets: false,
        api_keys: None,
        custom_model_providers: None,
        allow_use_of_warp_credits: false,
        autonomy_level: api::AutonomyLevel::Supervised,
        isolation_level: api::IsolationLevel::None,
        web_search_enabled: false,
        computer_use_enabled: false,
        ask_user_question_enabled,
        research_agent_enabled: false,
        orchestration_enabled: false,
        supported_tools_override: None,
        parent_agent_id: None,
        agent_name: None,
    }
}

fn request_params_for_remote(host_id: Option<HostId>) -> RequestParams {
    let mut params = request_params_with_ask_user_question_enabled(false);
    params.session_context =
        SessionContext::new_with_session_type_for_test(Some(SessionType::WarpifiedRemote {
            host_id,
        }));
    params
}

#[test]
fn api_keys_with_warp_credit_fallback_setting_returns_none_without_keys_or_fallback() {
    let api_keys = api_keys_with_warp_credit_fallback_setting(None, false);

    assert!(api_keys.is_none());
}

#[test]
fn api_keys_with_warp_credit_fallback_setting_creates_fallback_only_api_keys() {
    let api_keys = api_keys_with_warp_credit_fallback_setting(None, true)
        .expect("fallback setting should create ApiKeys");

    assert!(api_keys.allow_use_of_warp_credits);
    assert!(api_keys.anthropic.is_empty());
    assert!(api_keys.openai.is_empty());
    assert!(api_keys.google.is_empty());
    assert!(api_keys.open_router.is_empty());
    assert!(api_keys.aws_credentials.is_none());
}

#[test]
fn api_keys_with_warp_credit_fallback_setting_preserves_existing_keys() {
    let api_keys = api_keys_with_warp_credit_fallback_setting(
        Some(api::request::settings::ApiKeys {
            anthropic: "anthropic-key".to_string(),
            openai: String::new(),
            google: String::new(),
            open_router: String::new(),
            allow_use_of_warp_credits: false,
            aws_credentials: None,
        }),
        true,
    )
    .expect("existing ApiKeys should be preserved");

    assert_eq!(api_keys.anthropic, "anthropic-key");
    assert!(api_keys.allow_use_of_warp_credits);
}

#[test]
fn supports_orchestration_v2_matches_request_orchestration_setting() {
    assert!(supports_orchestration_v2(true));
    assert!(!supports_orchestration_v2(false));
}

#[test]
fn supported_tools_include_orchestration_tools_when_orchestration_enabled() {
    let mut params = request_params_with_ask_user_question_enabled(false);
    params.orchestration_enabled = true;

    let supported_tools = get_supported_tools(&params);

    assert!(supported_tools.contains(&api::ToolType::RunAgents));
    assert!(supported_tools.contains(&api::ToolType::SendMessageToAgent));
    assert!(!supported_tools.contains(&api::ToolType::StartAgent));
    assert!(!supported_tools.contains(&api::ToolType::StartAgentV2));
}

#[test]
fn supported_tools_omit_orchestration_tools_when_orchestration_disabled() {
    let params = request_params_with_ask_user_question_enabled(false);
    let supported_tools = get_supported_tools(&params);

    assert!(!supported_tools.contains(&api::ToolType::RunAgents));
    assert!(!supported_tools.contains(&api::ToolType::SendMessageToAgent));
    assert!(!supported_tools.contains(&api::ToolType::StartAgent));
    assert!(!supported_tools.contains(&api::ToolType::StartAgentV2));
}
#[test]
fn supported_tools_omits_ask_user_question_when_disabled() {
    let params = request_params_with_ask_user_question_enabled(false);
    let supported_tools = get_supported_tools(&params);

    assert!(!supported_tools.contains(&api::ToolType::AskUserQuestion));
}

#[test]
fn supported_tools_includes_ask_user_question_when_enabled_and_feature_flag_is_enabled() {
    if !FeatureFlag::AskUserQuestion.is_enabled() {
        return;
    }

    let params = request_params_with_ask_user_question_enabled(true);
    let supported_tools = get_supported_tools(&params);

    assert!(supported_tools.contains(&api::ToolType::AskUserQuestion));
}

#[test]
fn supported_tools_include_upload_artifact_when_feature_flag_is_enabled() {
    let _flag = FeatureFlag::ArtifactCommand.override_enabled(true);
    let params = request_params_with_ask_user_question_enabled(false);
    let supported_tools = get_supported_tools(&params);

    assert!(supported_tools.contains(&api::ToolType::UploadFileArtifact));
}

#[test]
fn supported_tools_omit_upload_artifact_when_feature_flag_is_disabled() {
    let _flag = FeatureFlag::ArtifactCommand.override_enabled(false);
    let params = request_params_with_ask_user_question_enabled(false);
    let supported_tools = get_supported_tools(&params);

    assert!(!supported_tools.contains(&api::ToolType::UploadFileArtifact));
}

#[test]
fn remote_supported_tools_include_search_codebase_when_connected_and_feature_flag_is_enabled() {
    let _flag = FeatureFlag::RemoteCodebaseIndexing.override_enabled(true);
    let params = request_params_for_remote(Some(HostId::new("host".to_string())));
    let supported_tools = get_supported_tools(&params);
    let supported_cli_agent_tools = get_supported_cli_agent_tools(&params);

    assert!(supported_tools.contains(&api::ToolType::SearchCodebase));
    assert!(supported_cli_agent_tools.contains(&api::ToolType::SearchCodebase));
}
#[test]
fn remote_supported_tools_omit_search_codebase_when_feature_flag_is_disabled() {
    let _flag = FeatureFlag::RemoteCodebaseIndexing.override_enabled(false);
    let params = request_params_for_remote(Some(HostId::new("host".to_string())));
    let supported_tools = get_supported_tools(&params);
    let supported_cli_agent_tools = get_supported_cli_agent_tools(&params);

    assert!(!supported_tools.contains(&api::ToolType::SearchCodebase));
    assert!(!supported_cli_agent_tools.contains(&api::ToolType::SearchCodebase));
}

#[test]
fn remote_supported_tools_omit_search_codebase_when_remote_is_not_connected() {
    let _flag = FeatureFlag::RemoteCodebaseIndexing.override_enabled(true);
    let params = request_params_for_remote(None);
    let supported_tools = get_supported_tools(&params);
    let supported_cli_agent_tools = get_supported_cli_agent_tools(&params);

    assert!(!supported_tools.contains(&api::ToolType::SearchCodebase));
    assert!(!supported_cli_agent_tools.contains(&api::ToolType::SearchCodebase));
}

fn normal_user_query_input(query: &str) -> AIAgentInput {
    AIAgentInput::UserQuery {
        query: query.to_string(),
        context: Arc::from(Vec::new()),
        static_query_type: None,
        referenced_attachments: HashMap::new(),
        user_query_mode: UserQueryMode::Normal,
        running_command: None,
        intended_agent: None,
    }
}

#[test]
fn forked_first_request_prepends_and_converts_conversation_handoff() {
    let _flag = FeatureFlag::ExplicitConversationHandoff.override_enabled(true);
    let mut params = request_params_with_ask_user_question_enabled(false);
    params.forked_from_conversation_token =
        Some(ServerConversationToken::new("cloud-token".to_string()));
    params.input = vec![normal_user_query_input("continue here")];

    maybe_prepend_conversation_handoff(
        &mut params.input,
        params.conversation_token.as_ref(),
        params.forked_from_conversation_token.as_ref(),
    );

    assert_eq!(params.input.len(), 2);
    assert!(matches!(
        params.input.first(),
        Some(AIAgentInput::ConversationHandoff)
    ));

    let converted = convert_input(params.input).expect("inputs should convert");
    let Some(api::request::input::Type::UserInputs(user_inputs)) = converted.r#type else {
        panic!("expected a UserInputs batch");
    };
    assert!(matches!(
        user_inputs.inputs[0].input,
        Some(api::request::input::user_inputs::user_input::Input::ConversationHandoff(_))
    ));
    assert!(matches!(
        user_inputs.inputs[1].input,
        Some(api::request::input::user_inputs::user_input::Input::UserQuery(_))
    ));
}

#[test]
fn non_forked_request_does_not_prepend_conversation_handoff() {
    let _flag = FeatureFlag::ExplicitConversationHandoff.override_enabled(true);
    let mut params = request_params_with_ask_user_question_enabled(false);
    params.input = vec![normal_user_query_input("hello")];

    maybe_prepend_conversation_handoff(
        &mut params.input,
        params.conversation_token.as_ref(),
        params.forked_from_conversation_token.as_ref(),
    );

    assert_eq!(params.input.len(), 1);
    assert!(matches!(
        params.input.first(),
        Some(AIAgentInput::UserQuery { .. })
    ));
}
