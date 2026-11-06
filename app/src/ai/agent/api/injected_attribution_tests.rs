//! Attribution travels with a query from the message it was restored or streamed from, through
//! the request the client sends, without being re-stamped as the current user's.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use ai::skills::{ParsedSkill, SkillProvider, SkillScope};
use warp_multi_agent_api as api;
use warp_util::local_or_remote_path::LocalOrRemotePath;

use super::convert_conversation::ConvertToExchanges;
use super::convert_from::user_inputs_from_messages;
use super::convert_to::convert_input;
use crate::ai::agent::base_user_query::warp_client_origin;
use crate::ai::agent::{
    AIAgentInput, BaseUserQuery, InvokeSkillUserQuery, RunningCommand, UserQueryMode,
};
use crate::ai::blocklist::PersistedAIInputType;
use crate::terminal::model::block::BlockId;

fn attributed_query(author_uid: &str) -> api::message::UserQuery {
    api::message::UserQuery {
        query: "formatted follow-up".into(),
        origin: Some(api::UserQueryOrigin {
            variant: Some(api::user_query_origin::Variant::ExternalPlatform(
                api::user_query_origin::ExternalPlatform {},
            )),
        }),
        author: Some(api::QueryAuthor {
            principal: Some(api::query_author::Principal::User(api::WarpUser {
                uid: author_uid.into(),
                email: format!("{author_uid}@example.com"),
                team_uid: "team".into(),
            })),
            resolution: api::IdentityResolution::ExternalAccountBinding.into(),
        }),
        source_message: Some(api::ExternalMessage {
            body: format!("message from {author_uid}"),
            platform: Some(api::external_message::Platform::Slack(
                api::external_message::Slack {
                    channel_id: "channel".into(),
                    thread_ts: "thread".into(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn query_input(query: &api::message::UserQuery) -> AIAgentInput {
    AIAgentInput::UserQuery {
        query: query.query.clone(),
        context: Arc::new([]),
        static_query_type: None,
        referenced_attachments: HashMap::new(),
        user_query_mode: UserQueryMode::Normal,
        running_command: None,
        intended_agent: None,
        base: BaseUserQuery::from_message(query),
    }
}

fn assert_echo(expected: &api::message::UserQuery, actual: &api::request::input::UserQuery) {
    assert_eq!(actual.query, expected.query);
    assert_eq!(actual.origin, expected.origin);
    assert_eq!(actual.author, expected.author);
    assert_eq!(actual.source_message, expected.source_message);
}

#[test]
fn batched_queries_and_retry_keep_each_authors_metadata() {
    let queries = [
        attributed_query("first-author"),
        attributed_query("second-author"),
    ];
    let inputs: Vec<_> = queries.iter().map(query_input).collect();
    let first_request = convert_input(inputs.clone()).unwrap();
    assert_eq!(first_request, convert_input(inputs).unwrap());
    let Some(api::request::input::Type::UserInputs(inputs)) = first_request.r#type else {
        panic!("expected user inputs")
    };
    assert_eq!(inputs.inputs.len(), 2);
    for (expected, input) in queries.iter().zip(inputs.inputs) {
        let Some(api::request::input::user_inputs::user_input::Input::UserQuery(actual)) =
            input.input
        else {
            panic!("expected user query")
        };
        assert_echo(expected, &actual);
    }
}

#[test]
fn cli_subagent_query_retains_the_original_author() {
    let query = attributed_query("external-author");
    let mut input = query_input(&query);
    if let AIAgentInput::UserQuery {
        running_command, ..
    } = &mut input
    {
        *running_command = Some(RunningCommand {
            command: "sh".into(),
            block_id: BlockId::from("block".to_owned()),
            grid_contents: String::new(),
            cursor: String::new(),
            requested_command_id: None,
            is_alt_screen_active: false,
        });
    }
    let Some(api::request::input::Type::UserInputs(mut inputs)) =
        convert_input(vec![input]).unwrap().r#type
    else {
        panic!("expected user inputs")
    };
    let Some(api::request::input::user_inputs::user_input::Input::CliAgentUserQuery(cli)) =
        inputs.inputs.remove(0).input
    else {
        panic!("expected CLI input")
    };
    assert_echo(&query, &cli.user_query.unwrap());
}

fn skill() -> ParsedSkill {
    ParsedSkill {
        path: LocalOrRemotePath::Local(PathBuf::from("/tmp/skill/SKILL.md")),
        name: "skill".into(),
        description: "Test skill".into(),
        content: "instructions".into(),
        line_range: None,
        provider: SkillProvider::Agents,
        scope: SkillScope::Project,
    }
}

fn skill_query_input(query: &api::message::UserQuery, base: Option<BaseUserQuery>) -> AIAgentInput {
    AIAgentInput::InvokeSkill {
        context: Arc::new([]),
        skill: skill(),
        user_query: Some(InvokeSkillUserQuery {
            query: query.query.clone(),
            referenced_attachments: HashMap::new(),
            base,
        }),
    }
}

fn converted_skill_query(input: AIAgentInput) -> api::request::input::UserQuery {
    let Some(api::request::input::Type::InvokeSkill(actual)) =
        convert_input(vec![input]).unwrap().r#type
    else {
        panic!("expected invoke-skill input")
    };
    actual.user_query.unwrap()
}

#[test]
fn a_locally_typed_skill_query_carries_the_fresh_marker() {
    let query = converted_skill_query(skill_query_input(&api::message::UserQuery::default(), None));
    assert_eq!(query.origin, Some(warp_client_origin()));
    assert!(query.author.is_none());
}

#[test]
fn invoke_skill_query_keeps_attribution_through_send_live_echo_and_restore() {
    let query = attributed_query("external-author");
    assert_echo(
        &query,
        &converted_skill_query(skill_query_input(
            &query,
            BaseUserQuery::from_message(&query),
        )),
    );
    let message = api::Message {
        id: "message".into(),
        task_id: "task".into(),
        request_id: "request".into(),
        message: Some(api::message::Message::InvokeSkill(
            api::message::InvokeSkill {
                skill: Some(skill().into()),
                user_query: Some(query.clone()),
            },
        )),
        ..Default::default()
    };
    let live = user_inputs_from_messages(std::slice::from_ref(&message));
    let task = api::Task {
        id: "task".into(),
        messages: vec![message],
        ..Default::default()
    };
    let restored = task.into_exchanges();
    for input in [&live[0], &restored[0].input[0]] {
        let AIAgentInput::InvokeSkill {
            user_query: Some(actual),
            ..
        } = input
        else {
            panic!("expected invoke-skill input")
        };
        assert_eq!(actual.base, BaseUserQuery::from_message(&query));
    }
}

#[test]
fn restored_and_live_queries_keep_their_attribution_but_recalled_text_is_fresh() {
    let query = attributed_query("external-author");
    let message = api::Message {
        id: "message".into(),
        task_id: "task".into(),
        request_id: "request".into(),
        message: Some(api::message::Message::UserQuery(query.clone())),
        ..Default::default()
    };
    let live = user_inputs_from_messages(std::slice::from_ref(&message));
    let task = api::Task {
        id: "task".into(),
        messages: vec![message],
        ..Default::default()
    };
    let restored = task.into_exchanges();
    for input in [&live[0], &restored[0].input[0]] {
        let AIAgentInput::UserQuery { base, .. } = input else {
            panic!("expected user query")
        };
        assert_eq!(*base, BaseUserQuery::from_message(&query));
        // Recalling the text with the up arrow makes a new query the user is typing now.
        let recalled: AIAgentInput = PersistedAIInputType::try_from(input)
            .unwrap()
            .try_into()
            .unwrap();
        let AIAgentInput::UserQuery {
            base,
            query: recalled_text,
            ..
        } = recalled
        else {
            panic!("expected recalled user query")
        };
        assert!(base.is_none());
        assert_eq!(recalled_text, query.query);
    }
}

#[test]
fn unattributed_history_is_not_stamped_with_the_current_user() {
    let message = api::Message {
        id: "message".into(),
        task_id: "task".into(),
        request_id: "request".into(),
        message: Some(api::message::Message::UserQuery(api::message::UserQuery {
            query: "old question".into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    let live = user_inputs_from_messages(std::slice::from_ref(&message));
    let AIAgentInput::UserQuery { base, .. } = &live[0] else {
        panic!("expected user query")
    };
    assert!(
        base.is_none(),
        "history without attribution stays unattributed"
    );
}
