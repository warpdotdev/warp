use super::*;
use crate::persistence::model::{AgentConversation, AgentConversationRecord};

fn persisted_conversation(conversation_id: AIConversationId) -> AgentConversation {
    let task_id = format!("task-{conversation_id}");
    AgentConversation {
        conversation: AgentConversationRecord {
            id: 0,
            conversation_id: conversation_id.to_string(),
            conversation_data: r#"{"server_conversation_token":null}"#.to_string(),
            last_modified_at: chrono::NaiveDateTime::default(),
            summary: None,
        },
        tasks: vec![warp_multi_agent_api::Task {
            id: task_id,
            messages: vec![],
            dependencies: None,
            description: "Test conversation".to_string(),
            summary: String::new(),
            server_data: String::new(),
        }],
    }
}

fn ai_conversation(conversation_id: AIConversationId) -> AIConversation {
    convert_persisted_conversation_to_ai_conversation_with_metadata(persisted_conversation(
        conversation_id,
    ))
    .expect("test conversation should convert")
}

#[test]
fn take_conversation_hands_out_each_conversation_at_most_once() {
    let conversation_id = AIConversationId::new();
    let mut store =
        RestoredAgentConversations::new_seeded(vec![persisted_conversation(conversation_id)]);

    assert!(store.take_conversation(&conversation_id).is_some());
    assert!(
        store.take_conversation(&conversation_id).is_none(),
        "a taken conversation must not be handed out again"
    );
    assert!(
        store.get_conversation(&conversation_id).is_none(),
        "a taken conversation must not be readable either"
    );
}

#[test]
fn failed_take_does_not_consume_the_restore_opportunity() {
    let conversation_id = AIConversationId::new();
    // No seed and no backing database: the first take fails to load.
    let mut store = RestoredAgentConversations::new_seeded(vec![]);
    assert!(store.take_conversation(&conversation_id).is_none());

    // Once the conversation becomes available (e.g. the earlier failure was
    // transient), a retry must still succeed — a failed load must not have
    // marked the ID as taken.
    store
        .conversations
        .insert(conversation_id, ai_conversation(conversation_id));
    assert!(
        store.take_conversation(&conversation_id).is_some(),
        "a failed load must not permanently consume the restore"
    );
    assert!(store.take_conversation(&conversation_id).is_none());
}

#[test]
fn taken_and_unknown_conversations_are_not_restored_into_a_pane() {
    let conversation_id = AIConversationId::new();
    let mut store =
        RestoredAgentConversations::new_seeded(vec![persisted_conversation(conversation_id)]);

    assert!(store.should_restore_into_pane(&conversation_id));
    assert!(store.take_conversation(&conversation_id).is_some());
    assert!(
        !store.should_restore_into_pane(&conversation_id),
        "a conversation already handed out must not be restored again"
    );

    // Neither cached nor loadable: nothing to restore, and nothing cached.
    let unknown_id = AIConversationId::new();
    assert!(!store.should_restore_into_pane(&unknown_id));
    assert_eq!(store.cached_conversation_count(), 0);
}

#[cfg(feature = "local_fs")]
mod db_backed {
    use diesel::Connection as _;
    use diesel_migrations::MigrationHarness as _;
    use warp_multi_agent_api as api;

    use super::*;
    use crate::persistence::agent::upsert_agent_conversation_for_test;

    fn test_connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:")
            .expect("in-memory sqlite connection should open");
        conn.run_pending_migrations(::persistence::MIGRATIONS)
            .expect("migrations should run");
        conn
    }

    fn message(task_id: &str, id: &str, message: api::message::Message) -> api::Message {
        api::Message {
            id: id.to_string(),
            task_id: task_id.to_string(),
            message: Some(message),
            ..Default::default()
        }
    }

    fn user_query_message(task_id: &str) -> api::Message {
        message(
            task_id,
            &format!("{task_id}-user-query"),
            api::message::Message::UserQuery(api::message::UserQuery {
                query: "Initial query".to_string(),
                ..Default::default()
            }),
        )
    }

    fn auto_code_diff_message(task_id: &str) -> api::Message {
        message(
            task_id,
            &format!("{task_id}-auto-code-diff"),
            api::message::Message::SystemQuery(api::message::SystemQuery {
                context: None,
                r#type: Some(api::message::system_query::Type::AutoCodeDiff(
                    api::message::AutoCodeDiff {
                        query: "diff".to_string(),
                    },
                )),
            }),
        )
    }

    /// A `SystemQuery` message wrapping one of the `system_query::Type` arms.
    fn system_query_message(
        task_id: &str,
        id_suffix: &str,
        r#type: api::message::system_query::Type,
    ) -> api::Message {
        message(
            task_id,
            &format!("{task_id}-{id_suffix}"),
            api::message::Message::SystemQuery(api::message::SystemQuery {
                context: None,
                r#type: Some(r#type),
            }),
        )
    }

    fn clone_repository_message(task_id: &str) -> api::Message {
        system_query_message(
            task_id,
            "clone-repository",
            api::message::system_query::Type::CloneRepository(api::message::CloneRepository {
                url: "https://github.com/warpdotdev/warp".to_string(),
            }),
        )
    }

    fn create_new_project_message(task_id: &str) -> api::Message {
        system_query_message(
            task_id,
            "create-new-project",
            api::message::system_query::Type::CreateNewProject(api::message::CreateNewProject {
                query: "Start a new project".to_string(),
            }),
        )
    }

    fn generate_passive_suggestions_message(task_id: &str) -> api::Message {
        system_query_message(
            task_id,
            "generate-passive-suggestions",
            api::message::system_query::Type::GeneratePassiveSuggestions(Default::default()),
        )
    }

    /// A tool-call result for an accepted `SuggestPrompt`. No matching
    /// `ToolCall` is written, so the restored input carries an empty accepted
    /// query — `display_query()` is still `Some`, which is what decides the
    /// filter.
    fn accepted_suggest_prompt_message(task_id: &str) -> api::Message {
        message(
            task_id,
            &format!("{task_id}-suggest-prompt-result"),
            api::message::Message::ToolCallResult(api::message::ToolCallResult {
                tool_call_id: format!("{task_id}-suggest-prompt"),
                result: Some(api::message::tool_call_result::Result::SuggestPrompt(
                    api::SuggestPromptResult {
                        result: Some(api::suggest_prompt_result::Result::Accepted(())),
                    },
                )),
                ..Default::default()
            }),
        )
    }

    fn passive_suggestion_result_message(
        task_id: &str,
        id_suffix: &str,
        suggestion: api::passive_suggestion_result_type::Suggestion,
    ) -> api::Message {
        message(
            task_id,
            &format!("{task_id}-{id_suffix}"),
            api::message::Message::PassiveSuggestionResult(api::message::PassiveSuggestionResult {
                result: Some(api::PassiveSuggestionResultType {
                    trigger: Some(
                        api::passive_suggestion_result_type::Trigger::AgentResponseCompleted(
                            Default::default(),
                        ),
                    ),
                    suggestion: Some(suggestion),
                }),
                context: None,
            }),
        )
    }

    fn accepted_prompt_suggestion_message(task_id: &str) -> api::Message {
        passive_suggestion_result_message(
            task_id,
            "passive-prompt",
            api::passive_suggestion_result_type::Suggestion::Prompt(
                api::passive_suggestion_result_type::Prompt {
                    prompt: "Try this instead".to_string(),
                },
            ),
        )
    }

    fn passive_code_diff_result_message(task_id: &str) -> api::Message {
        passive_suggestion_result_message(
            task_id,
            "passive-code-diff",
            api::passive_suggestion_result_type::Suggestion::CodeDiff(
                api::passive_suggestion_result_type::CodeDiff {
                    summary: "Tidy imports".to_string(),
                    ..Default::default()
                },
            ),
        )
    }

    /// An `InvokeSkill` whose embedded skill has no descriptor, so
    /// `ParsedSkill::try_from_api_with_origin` rejects it and restore pushes no
    /// input at all. The classifier can't see that outcome, which is exactly
    /// why the arm reports "unknown".
    fn invoke_skill_message(task_id: &str) -> api::Message {
        message(
            task_id,
            &format!("{task_id}-invoke-skill"),
            api::message::Message::InvokeSkill(api::message::InvokeSkill {
                skill: Some(Default::default()),
                ..Default::default()
            }),
        )
    }

    fn root_task(task_id: &str, messages: Vec<api::Message>) -> api::Task {
        api::Task {
            id: task_id.to_string(),
            description: String::new(),
            dependencies: None,
            messages,
            summary: String::new(),
            server_data: String::new(),
        }
    }

    /// Writes a conversation through the normal upsert path and returns the
    /// database it lives in.
    fn connection_with_conversation(
        conversation_id: AIConversationId,
        messages: Vec<api::Message>,
    ) -> SqliteConnection {
        let mut conn = test_connection();
        let task_id = format!("task-{conversation_id}");
        let messages = messages
            .into_iter()
            .map(|mut message| {
                message.task_id = task_id.clone();
                message
            })
            .collect();
        upsert_agent_conversation_for_test(
            &mut conn,
            &conversation_id.to_string(),
            [&root_task(&task_id, messages)],
        );
        conn
    }

    fn store_with_conversation(
        conversation_id: AIConversationId,
        messages: Vec<api::Message>,
    ) -> RestoredAgentConversations {
        RestoredAgentConversations::new_with_db_connection(connection_with_conversation(
            conversation_id,
            messages,
        ))
    }

    fn delete_task_rows(conn: &mut SqliteConnection, conversation: &str) {
        use diesel::prelude::*;

        use crate::persistence::schema::agent_tasks::dsl::*;
        diesel::delete(agent_tasks.filter(conversation_id.eq(conversation)))
            .execute(conn)
            .expect("task rows should delete");
    }

    fn clear_summary(conn: &mut SqliteConnection, conversation: &str) {
        use diesel::prelude::*;

        use crate::persistence::schema::agent_conversations::dsl::*;
        diesel::update(agent_conversations.filter(conversation_id.eq(conversation)))
            .set(summary.eq(None::<String>))
            .execute(conn)
            .expect("summary reset should succeed");
    }

    /// The filter decision must match what evaluating the loaded conversation
    /// would say, and must be reached without retaining anything.
    fn assert_filter_decision(messages: Vec<api::Message>, expected: bool) {
        let conversation_id = AIConversationId::new();

        let mut store = store_with_conversation(conversation_id, messages.clone());
        assert_eq!(
            store.should_restore_into_pane(&conversation_id),
            expected,
            "summary-backed decision disagrees with the expectation"
        );

        // The same decision reached from the fully loaded conversation, i.e.
        // the pre-change behavior this must stay equivalent to.
        let mut loading_store = store_with_conversation(conversation_id, messages);
        let loaded = loading_store
            .get_conversation(&conversation_id)
            .expect("conversation should load");
        let from_loaded = loaded.all_tasks().next().is_some() && !loaded.is_entirely_passive();
        assert_eq!(
            from_loaded, expected,
            "the loaded conversation disagrees with the expectation"
        );
    }

    #[test]
    fn filter_matches_loaded_behavior_for_a_conversation_with_a_user_query() {
        assert_filter_decision(vec![user_query_message("root")], true);
    }

    #[test]
    fn filter_matches_loaded_behavior_for_a_message_less_root_task() {
        assert_filter_decision(vec![], true);
    }

    #[test]
    fn filter_matches_loaded_behavior_for_an_entirely_passive_conversation() {
        assert_filter_decision(vec![auto_code_diff_message("root")], false);
    }

    #[test]
    fn filter_matches_loaded_behavior_for_a_passive_conversation_the_user_continued() {
        assert_filter_decision(
            vec![auto_code_diff_message("root"), user_query_message("root")],
            true,
        );
    }

    // One case per non-trivial `restored_message_kind` arm. Each pairs the arm
    // under test with a passive `AutoCodeDiff`, which is what makes the arm's
    // classification load-bearing: without a passive request in the root the
    // answer is "restore" regardless of how the other message classifies, so
    // such a case would pass even if the arm were wrong. With one present, a
    // `UserQuery` arm flips the decision to restore and a `Neither` arm leaves
    // it rejected — and `assert_filter_decision` checks both the summary-backed
    // decision and the one computed from the fully restored conversation, so a
    // classifier that disagrees with the client's rendering fails here.

    #[test]
    fn filter_matches_loaded_behavior_for_an_accepted_prompt_suggestion_tool_result() {
        assert_filter_decision(
            vec![
                auto_code_diff_message("root"),
                accepted_suggest_prompt_message("root"),
            ],
            true,
        );
    }

    #[test]
    fn filter_matches_loaded_behavior_for_an_accepted_passive_prompt_suggestion() {
        assert_filter_decision(
            vec![
                auto_code_diff_message("root"),
                accepted_prompt_suggestion_message("root"),
            ],
            true,
        );
    }

    /// A passive code-diff suggestion result renders no query and is not itself
    /// a passive request, so the conversation stays entirely passive.
    #[test]
    fn filter_matches_loaded_behavior_for_a_passive_code_diff_result() {
        assert_filter_decision(
            vec![
                auto_code_diff_message("root"),
                passive_code_diff_result_message("root"),
            ],
            false,
        );
    }

    /// `GeneratePassiveSuggestions` is never rendered as user input on restore,
    /// so it leaves an otherwise passive conversation rejected.
    #[test]
    fn filter_matches_loaded_behavior_for_a_generate_passive_suggestions_query() {
        assert_filter_decision(
            vec![
                auto_code_diff_message("root"),
                generate_passive_suggestions_message("root"),
            ],
            false,
        );
    }

    #[test]
    fn filter_matches_loaded_behavior_for_a_clone_repository_query() {
        assert_filter_decision(
            vec![
                auto_code_diff_message("root"),
                clone_repository_message("root"),
            ],
            true,
        );
    }

    #[test]
    fn filter_matches_loaded_behavior_for_a_create_new_project_query() {
        assert_filter_decision(
            vec![
                auto_code_diff_message("root"),
                create_new_project_message("root"),
            ],
            true,
        );
    }

    /// The `InvokeSkill` arm reports "unknown", so this is the one case that
    /// exercises the full-load fallback rather than the summary fast path. That
    /// makes it a value test rather than an equivalence test: with no summary
    /// answer to compare against, both halves of `assert_filter_decision`
    /// necessarily agree. It still pins the end-to-end outcome and the fallback
    /// wiring, and here the skill fails to parse, so restore pushes no input and
    /// the conversation remains entirely passive.
    #[test]
    fn filter_falls_back_to_the_full_load_for_a_skill_invocation() {
        assert_filter_decision(
            vec![auto_code_diff_message("root"), invoke_skill_message("root")],
            false,
        );
    }

    /// The point of the summary-backed path: a conversation the filter rejects
    /// must never be loaded into memory, let alone kept there.
    #[test]
    fn rejected_conversations_are_never_cached() {
        let conversation_id = AIConversationId::new();
        let mut store =
            store_with_conversation(conversation_id, vec![auto_code_diff_message("root")]);

        assert!(!store.should_restore_into_pane(&conversation_id));
        assert_eq!(
            store.cached_conversation_count(),
            0,
            "a conversation that failed the filter must not be retained"
        );
    }

    /// A conversation that passes is cached exactly once so the imminent
    /// `take_conversation` reuses the load, and taking it releases the payload.
    #[test]
    fn accepted_conversations_are_released_once_taken() {
        let conversation_id = AIConversationId::new();
        let mut store = store_with_conversation(conversation_id, vec![user_query_message("root")]);

        assert!(store.should_restore_into_pane(&conversation_id));
        assert!(store.take_conversation(&conversation_id).is_some());
        assert_eq!(store.cached_conversation_count(), 0);
    }

    /// Pins that the decision actually comes out of the `summary` column rather
    /// than out of the task payloads — which is the whole fix, and which every
    /// other test here would keep passing without.
    ///
    /// A passive conversation with no `agent_tasks` rows is the shape where the
    /// two paths disagree: the summary says "entirely passive" and rejects,
    /// while a fallback load finds no tasks, synthesizes an empty root, and
    /// accepts. The second half clears the summary to show the fallback really
    /// does reach the opposite answer, so the first assertion is discriminating
    /// rather than vacuous.
    #[test]
    fn the_filter_answers_from_the_summary_not_from_the_task_rows() {
        let conversation_id = AIConversationId::new();
        let mut conn =
            connection_with_conversation(conversation_id, vec![auto_code_diff_message("root")]);
        delete_task_rows(&mut conn, &conversation_id.to_string());

        let mut store = RestoredAgentConversations::new_with_db_connection(conn);
        assert!(
            !store.should_restore_into_pane(&conversation_id),
            "the filter must be answered from the summary column alone"
        );
        assert_eq!(store.cached_conversation_count(), 0);

        let conversation_id = AIConversationId::new();
        let mut conn =
            connection_with_conversation(conversation_id, vec![auto_code_diff_message("root")]);
        delete_task_rows(&mut conn, &conversation_id.to_string());
        clear_summary(&mut conn, &conversation_id.to_string());

        let mut store = RestoredAgentConversations::new_with_db_connection(conn);
        assert!(
            store.should_restore_into_pane(&conversation_id),
            "with no summary to read, the fallback load must run and reach the other answer"
        );
    }
}
