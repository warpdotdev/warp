use std::sync::Arc;

use warp_core::channel::{Channel, ChannelState};
use warp_multi_agent_api::request::input::Type;

use super::{RequestParams, ServerConversationToken};
use crate::ai::agent::{AIAgentInput, ServerOutputId};
use crate::server::ids::ServerId;
use crate::server::team_scope::RequestTeamScope;
use crate::workspaces::user_workspaces::{TeamContextForOperation, TeamlessScopeForTest};

#[test]
fn deferred_repos_query_inline_serializes_separate_runtime_prompt_and_authored_skill() {
    let skill = ai::skills::ParsedSkill {
        path: warp_util::local_or_remote_path::LocalOrRemotePath::Local(
            "/skills/reviewer/SKILL.md".into(),
        ),
        name: "deferred-repositories".to_string(),
        description: "Reviews billing changes.".to_string(),
        content: "Review the diff carefully.".to_string(),
        line_range: None,
        provider: ai::skills::SkillProvider::Agents,
        scope: ai::skills::SkillScope::Project,
    };
    let input = AIAgentInput::StartFromAmbientRunPrompt {
        ambient_run_id: "run-123".into(),
        context: Arc::from([]),
        runtime_base_prompt: Some("clone acme/billing first".into()),
        runtime_skill: Some(skill),
        attachments_dir: Some("/workspace/.warp/attachments".into()),
    };

    let converted = super::convert_to::convert_input(vec![input]).unwrap();
    let Some(Type::StartFromAmbientRunPrompt(start)) = converted.r#type else {
        panic!("expected ambient-run startup input");
    };
    assert_eq!(start.runtime_base_prompt, "clone acme/billing first");
    assert_eq!(start.ambient_run_id, "run-123");
    assert_eq!(start.attachments_dir, "/workspace/.warp/attachments");
    let authored = start.runtime_skill.expect("authored skill must survive");
    let descriptor = authored
        .descriptor
        .expect("authored skill descriptor must survive");
    assert_eq!(descriptor.name, "deferred-repositories");
    assert_eq!(descriptor.description, "Reviews billing changes.");
    assert_eq!(
        authored
            .content
            .expect("authored skill content must survive")
            .content,
        "Review the diff carefully."
    );
}

#[test]
fn debugging_payload_is_link_on_dogfood_channels() {
    let token = ServerConversationToken::new("conversation-token".to_owned());
    let request_id = ServerOutputId::new("request-id".to_owned());
    let expected_link = format!(
        "{}/debug/maa/conversation-token",
        ChannelState::server_root_url()
    );

    for channel in [Channel::Dev, Channel::Local] {
        assert_eq!(
            token.debugging_payload_for_channel(None, channel),
            expected_link
        );
        assert_eq!(
            token.debugging_payload_for_channel(Some(&request_id), channel),
            format!("{expected_link}?request=request-id")
        );
    }
}

#[test]
fn debugging_payload_is_id_on_non_dogfood_channels() {
    let token = ServerConversationToken::new("conversation-token".to_owned());
    let request_id = ServerOutputId::new("request-id".to_owned());

    for channel in [
        Channel::Stable,
        Channel::Preview,
        Channel::Integration,
        Channel::Oss,
    ] {
        assert_eq!(
            token.debugging_payload_for_channel(None, channel),
            "{\"conversation_id\":\"conversation-token\"}"
        );
        assert_eq!(
            token.debugging_payload_for_channel(Some(&request_id), channel),
            "{\"request_id\":\"request-id\",\"conversation_id\":\"conversation-token\"}"
        );
    }
}

/// [`RequestParams::new`] resolves `member_byo_credentials_allowed` from a real scope; this
/// test-only constructor has none to give, so it has to default to the safe answer for a
/// credential gate: not permitted. The scope-driven behaviour itself --
/// `UserWorkspaces::are_member_byo_keys_allowed`/`are_member_byo_endpoints_allowed` gating
/// `RequestParams::api_keys`/`custom_model_providers` by the requesting window's team, with
/// AWS Bedrock/GEAP credentials surviving regardless -- is covered where that policy lives: the
/// `member_byo_policy_*` tests in `workspaces::user_workspaces::user_workspaces_tests`.
#[test]
fn request_params_do_not_allow_member_credentials_until_the_policy_has_been_applied() {
    assert!(!RequestParams::new_for_test().member_byo_credentials_allowed);
}

/// The team header a request sends is whatever its scope named, including when that is no team
/// at all -- a scope resolving to `None` must send no team rather than fall back to one.
#[test]
fn request_team_scope_sends_the_team_its_scope_named() {
    let team_uid = ServerId::from(7);
    let scope = TeamContextForOperation::new_for_test(team_uid);

    assert_eq!(
        RequestTeamScope::from_scope(&scope).team_uid(),
        Some(team_uid)
    );
    assert_eq!(
        RequestTeamScope::from_scope(&TeamlessScopeForTest).team_uid(),
        None
    );
}
