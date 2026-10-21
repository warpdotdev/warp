use ai::hooks::test_util::{test_binding, test_source_id};
use ai::hooks::{HookBoundary, HookPayload};
use warp_multi_agent_api::client_action::{
    AddMessagesToTask, AppendToMessageContent, UpdateTaskMessage,
};
use warp_multi_agent_api::message::{AgentOutput, AgentReasoning, ToolCall};
use warpui::App;

use super::*;
use crate::ai::blocklist::hooks::testing::{hook_test_app, settle};
use crate::features::FeatureFlag;

fn stream_id() -> ResponseStreamId {
    ResponseStreamId::new_for_test()
}

fn text(id: &str, update: TextUpdate) -> StreamObservation {
    StreamObservation::Text {
        message_id: id.to_owned(),
        update,
    }
}

fn reasoning(id: &str, update: TextUpdate, finished: Option<Duration>) -> StreamObservation {
    StreamObservation::Reasoning {
        message_id: id.to_owned(),
        update,
        finished_duration: finished,
    }
}

fn completed_ids(completed: &[CompletedMessage]) -> Vec<(&str, &str)> {
    completed
        .iter()
        .map(|message| (message.message_id.as_str(), message.text.as_str()))
        .collect()
}

#[test]
fn a_text_message_is_emitted_once_when_the_next_text_message_starts() {
    let mut state = StreamObserverState::default();
    let stream = stream_id();
    let conversation = AIConversationId::new();

    let completed = state.apply_batch(
        &stream,
        conversation,
        Some(conversation),
        vec![
            text("m1", TextUpdate::Set("Hel".to_owned())),
            text("m1", TextUpdate::Append("lo".to_owned())),
        ],
    );
    assert!(completed.is_empty(), "chunks never emit");

    let completed = state.apply_batch(
        &stream,
        conversation,
        Some(conversation),
        vec![text("m2", TextUpdate::Set("Next".to_owned()))],
    );
    assert_eq!(completed_ids(&completed), vec![("m1", "Hello")]);
    assert_eq!(completed[0].conversation_id, Some(conversation));

    let completed = state.apply_batch(
        &stream,
        conversation,
        Some(conversation),
        vec![text("m1", TextUpdate::Append("late".to_owned()))],
    );
    assert!(
        completed.is_empty(),
        "a late chunk for an emitted message does not re-emit"
    );

    let completed = state.finish(&stream, false);
    assert_eq!(completed_ids(&completed), vec![("m2", "Next")]);
}

#[test]
fn reasoning_between_two_text_messages_ends_the_first_and_emits_on_finish() {
    let mut state = StreamObserverState::default();
    let stream = stream_id();
    let conversation = AIConversationId::new();

    let completed = state.apply_batch(
        &stream,
        conversation,
        Some(conversation),
        vec![
            text("t1", TextUpdate::Set("first".to_owned())),
            reasoning("r1", TextUpdate::Set("think".to_owned()), None),
            reasoning("r1", TextUpdate::Append("ing".to_owned()), None),
        ],
    );
    assert_eq!(completed_ids(&completed), vec![("t1", "first")]);
    assert!(matches!(completed[0].kind, CompletedKind::Text));

    let completed = state.apply_batch(
        &stream,
        conversation,
        Some(conversation),
        vec![
            reasoning(
                "r1",
                TextUpdate::Append(String::new()),
                Some(Duration::from_secs(3)),
            ),
            reasoning(
                "r1",
                TextUpdate::Append(String::new()),
                Some(Duration::from_secs(3)),
            ),
            text("t2", TextUpdate::Set("second".to_owned())),
        ],
    );
    assert_eq!(completed_ids(&completed), vec![("r1", "thinking")]);
    assert_eq!(
        completed[0].kind,
        CompletedKind::Reasoning {
            duration: Some(Duration::from_secs(3))
        }
    );

    let completed = state.finish(&stream, false);
    assert_eq!(completed_ids(&completed), vec![("t2", "second")]);
}

#[test]
fn a_text_message_still_streaming_is_emitted_when_the_stream_ends() {
    let mut state = StreamObserverState::default();
    let stream = stream_id();
    let conversation = AIConversationId::new();
    let completed = state.apply_batch(
        &stream,
        conversation,
        Some(conversation),
        vec![text("only", TextUpdate::Set("done".to_owned()))],
    );
    assert!(completed.is_empty());
    let completed = state.finish(&stream, false);
    assert_eq!(completed_ids(&completed), vec![("only", "done")]);
    assert!(state.finish(&stream, false).is_empty(), "state is dropped");
}

#[test]
fn a_cancelled_stream_does_not_emit_its_partial_text() {
    let mut state = StreamObserverState::default();
    let stream = stream_id();
    let conversation = AIConversationId::new();
    state.apply_batch(
        &stream,
        conversation,
        Some(conversation),
        vec![text("partial", TextUpdate::Set("half".to_owned()))],
    );
    assert!(state.finish(&stream, true).is_empty());
}

#[test]
fn a_tool_call_message_ends_the_text_before_it() {
    let mut state = StreamObserverState::default();
    let stream = stream_id();
    let conversation = AIConversationId::new();
    let completed = state.apply_batch(
        &stream,
        conversation,
        Some(conversation),
        vec![
            text("t1", TextUpdate::Set("before".to_owned())),
            StreamObservation::OtherMessage {
                message_id: "tool".to_owned(),
            },
            text("t2", TextUpdate::Set("after".to_owned())),
        ],
    );
    assert_eq!(completed_ids(&completed), vec![("t1", "before")]);
}

#[test]
fn a_conversation_split_attributes_messages_to_the_right_conversation() {
    let mut state = StreamObserverState::default();
    let stream = stream_id();
    let before = AIConversationId::new();
    let after = AIConversationId::new();
    let completed = state.apply_batch(
        &stream,
        before,
        Some(after),
        vec![
            text("old", TextUpdate::Set("in old".to_owned())),
            StreamObservation::ConversationSplit,
            text("new", TextUpdate::Set("in new".to_owned())),
            text("new2", TextUpdate::Set("also new".to_owned())),
        ],
    );
    assert_eq!(
        completed_ids(&completed),
        vec![("old", "in old"), ("new", "in new")]
    );
    assert_eq!(completed[0].conversation_id, Some(before));
    assert_eq!(completed[1].conversation_id, Some(after));

    let completed = state.finish(&stream, false);
    assert_eq!(completed_ids(&completed), vec![("new2", "also new")]);
    assert_eq!(completed[0].conversation_id, Some(after));
}

fn message(id: &str, body: Message) -> api::Message {
    api::Message {
        id: id.to_owned(),
        message: Some(body),
        ..Default::default()
    }
}

#[test]
fn an_update_to_an_earlier_tool_call_does_not_end_the_streaming_text() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        app.update(|ctx| {
            harness.hooks.install_bindings(
                vec![test_binding(
                    test_source_id("s"),
                    HookBoundary::ResponseComplete,
                    0,
                    None,
                )],
                ctx,
            )
        });
        let tool_call = |id: &str| {
            message(
                id,
                Message::ToolCall(ToolCall {
                    tool_call_id: id.to_owned(),
                    ..Default::default()
                }),
            )
        };
        let actions = vec![
            api::ClientAction {
                action: Some(Action::AddMessagesToTask(AddMessagesToTask {
                    task_id: "task".to_owned(),
                    messages: vec![
                        tool_call("call"),
                        message(
                            "t1",
                            Message::AgentOutput(AgentOutput {
                                text: "Hel".to_owned(),
                            }),
                        ),
                    ],
                })),
            },
            api::ClientAction {
                action: Some(Action::UpdateTaskMessage(UpdateTaskMessage {
                    task_id: "task".to_owned(),
                    message: Some(tool_call("call")),
                    mask: None,
                })),
            },
            api::ClientAction {
                action: Some(Action::AppendToMessageContent(AppendToMessageContent {
                    task_id: "task".to_owned(),
                    message: Some(message(
                        "t1",
                        Message::AgentOutput(AgentOutput {
                            text: "lo".to_owned(),
                        }),
                    )),
                    mask: None,
                })),
            },
        ];
        let observations =
            app.read(|ctx| collect_observations(&actions, harness.conversation_id, ctx));
        assert_eq!(
            observations,
            vec![
                StreamObservation::OtherMessage {
                    message_id: "call".to_owned(),
                },
                text("t1", TextUpdate::Set("Hel".to_owned())),
                text("t1", TextUpdate::Append("lo".to_owned())),
            ],
            "the update to the tool call is not a new message"
        );

        let mut state = StreamObserverState::default();
        let stream = stream_id();
        let completed = state.apply_batch(
            &stream,
            harness.conversation_id,
            Some(harness.conversation_id),
            observations,
        );
        assert!(completed.is_empty());
        let completed = state.finish(&stream, false);
        assert_eq!(completed_ids(&completed), vec![("t1", "Hello")]);
    });
}

#[test]
fn collect_observations_reads_text_reasoning_and_splits_from_client_actions() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        let actions = vec![
            api::ClientAction {
                action: Some(Action::AddMessagesToTask(AddMessagesToTask {
                    task_id: "task".to_owned(),
                    messages: vec![
                        message(
                            "t1",
                            Message::AgentOutput(AgentOutput {
                                text: "Hi".to_owned(),
                            }),
                        ),
                        message(
                            "r1",
                            Message::AgentReasoning(AgentReasoning {
                                reasoning: "hm".to_owned(),
                                finished_duration: Some(prost_types::Duration {
                                    seconds: 2,
                                    nanos: 0,
                                }),
                            }),
                        ),
                    ],
                })),
            },
            api::ClientAction {
                action: Some(Action::AppendToMessageContent(AppendToMessageContent {
                    task_id: "task".to_owned(),
                    message: Some(message(
                        "t1",
                        Message::AgentOutput(AgentOutput {
                            text: " there".to_owned(),
                        }),
                    )),
                    mask: None,
                })),
            },
            api::ClientAction {
                action: Some(Action::StartNewConversation(Default::default())),
            },
        ];

        let conversation_id = harness.conversation_id;
        let observations = app.read(|ctx| collect_observations(&actions, conversation_id, ctx));
        assert!(
            observations.is_empty(),
            "an empty registry means nothing is collected"
        );

        let source = test_source_id("s");
        app.update(|ctx| {
            harness.hooks.install_bindings(
                vec![
                    test_binding(source.clone(), HookBoundary::ResponseComplete, 0, None),
                    test_binding(source.clone(), HookBoundary::ReasoningComplete, 0, None),
                ],
                ctx,
            )
        });
        assert!(
            app.read(|ctx| collect_observations(&actions, AIConversationId::new(), ctx))
                .is_empty(),
            "a conversation this client does not drive is not observed"
        );
        let observations = app.read(|ctx| collect_observations(&actions, conversation_id, ctx));
        assert_eq!(
            observations,
            vec![
                text("t1", TextUpdate::Set("Hi".to_owned())),
                reasoning(
                    "r1",
                    TextUpdate::Set("hm".to_owned()),
                    Some(Duration::from_secs(2))
                ),
                text("t1", TextUpdate::Append(" there".to_owned())),
                StreamObservation::ConversationSplit,
            ]
        );

        // Applying the batch finishes the reasoning (it carries a duration) and the text (the
        // split ends it), so exactly two envelopes reach the adapter.
        let stream = stream_id();
        app.update(|ctx| {
            observe(
                &stream,
                conversation_id,
                Some(conversation_id),
                observations,
                &harness.active_session,
                ctx,
            )
        });
        settle(&mut app, |_| harness.hooks.adapter.encoded_count() == 2).await;
        let encoded = harness.hooks.adapter.encoded_envelopes();
        let kinds = encoded
            .iter()
            .map(|envelope| match &envelope.payload {
                HookPayload::ResponseComplete { message_id, text } => {
                    format!("text:{message_id}:{text}")
                }
                HookPayload::ReasoningComplete {
                    message_id, text, ..
                } => format!("reasoning:{message_id}:{text}"),
                HookPayload::SessionStart { .. }
                | HookPayload::PromptSubmit { .. }
                | HookPayload::ToolPre { .. }
                | HookPayload::ToolResult { .. }
                | HookPayload::ToolFailure { .. }
                | HookPayload::TurnStop { .. } => "other".to_owned(),
            })
            .collect::<HashSet<_>>();
        assert_eq!(
            kinds,
            HashSet::from(["text:t1:Hi".to_owned(), "reasoning:r1:hm".to_owned()])
        );
        assert!(encoded.iter().all(|e| e.conversation_id == conversation_id));
    });
}
