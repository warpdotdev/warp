use std::cell::{Cell, RefCell};
use std::rc::Rc;

use ai::diff_validation::{AIRequestedCodeDiff, DiffType, ParsedDiff};
use async_channel::unbounded;
use futures::FutureExt;
use warpui::{App, AppContext, EntityId};

use super::super::file_revisions::read_local_revisions;
use super::*;
use crate::ai::agent::FileEdit;
use crate::ai::agent::task::TaskId;
use crate::auth::AuthStateProvider;
use crate::terminal::model::session::Sessions;
use crate::terminal::model_events::ModelEventDispatcher;

/// Shared observable state for a [`TestStorage`].
struct TestStorageState {
    diffs: RefCell<Option<(Vec<FileDiff>, DiffSessionType)>>,
    accepted: Cell<bool>,
}

#[test]
fn execute_revalidates_after_preprocessing_before_storage_write() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("preview-race.rs");
        let path = path.to_string_lossy().to_string();
        std::fs::write(&path, "let value = old;\n").unwrap();

        let conversation_id = AIConversationId::new();
        let tracker = FileRevisionTracker::default();
        tracker.record_revisions(conversation_id, read_local_revisions([path.clone()]));
        let executor = add_executor_with_tracker(&mut app, tracker);
        let action_id = AIAgentActionId::from("preview-race".to_owned());
        let storage = register_storage(&mut app, &executor, &action_id);
        let action = edit_action_with_edits(
            &action_id,
            vec![FileEdit::Edit(ParsedDiff::StrReplaceEdit {
                file: Some(path.clone()),
                search: Some("1|let value = old;".to_string()),
                replace: Some("let value = agent;".to_string()),
            })],
        );

        let preprocess = executor.update(&mut app, |executor, ctx| {
            executor.preprocess_action(
                PreprocessActionInput {
                    action: &action,
                    conversation_id,
                },
                ctx,
            )
        });
        preprocess.await;
        assert!(storage.diffs.borrow().is_some());

        let external_content = "// external edit\nlet value = old;\n";
        std::fs::write(&path, external_content).unwrap();
        let execution = execute_action(&mut app, &executor, &action, conversation_id);
        let AnyActionExecution::Async {
            execute_future,
            on_complete,
        } = execution
        else {
            panic!("expected async execution");
        };
        let result = execute_future.await;
        let result = app.update(|ctx| on_complete(result, ctx));

        assert!(!storage.accepted.get());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), external_content);
        assert!(matches!(
            result,
            AIAgentActionResultType::RequestFileEdits(
                RequestFileEditsResult::DiffApplicationFailed { error }
            ) if error.contains("Call read_files")
        ));
    });
}

impl TestStorageState {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            diffs: RefCell::new(None),
            accepted: Cell::new(false),
        })
    }
}

/// A registrable storage double that records seeding and accepts immediately.
struct TestStorage(Rc<TestStorageState>);

impl RegisteredDiffStorage for TestStorage {
    fn set_candidate_diffs(
        &self,
        diffs: Vec<FileDiff>,
        session_type: DiffSessionType,
        _app: &mut AppContext,
    ) {
        *self.0.diffs.borrow_mut() = Some((diffs, session_type));
    }

    fn accept_and_save(&self, _app: &mut AppContext) -> BoxFuture<'static, RequestFileEditsResult> {
        self.0.accepted.set(true);
        futures::future::ready(RequestFileEditsResult::Success {
            diff: String::new(),
            updated_files: Vec::new(),
            deleted_files: Vec::new(),
            lines_added: 0,
            lines_removed: 0,
        })
        .boxed()
    }
}

/// Builds an executor over a minimal test session.
fn add_executor(app: &mut App) -> ModelHandle<RequestFileEditsExecutor> {
    add_executor_with_tracker(app, FileRevisionTracker::default())
}

fn add_executor_with_tracker(
    app: &mut App,
    file_revision_tracker: FileRevisionTracker,
) -> ModelHandle<RequestFileEditsExecutor> {
    app.update(warp_core::telemetry::testing::MockTelemetryContextProvider::register);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    let sessions = app.add_model(|_| Sessions::new_for_test());
    let (_, model_events_rx) = unbounded();
    let dispatcher =
        app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
    let active_session =
        app.add_model(|ctx| ActiveSession::new(sessions.clone(), dispatcher.clone(), ctx));
    app.add_model(|ctx| {
        RequestFileEditsExecutor::new(active_session, EntityId::new(), file_revision_tracker, ctx)
    })
}

/// Registers a `TestStorage` for `action_id` and returns its observable state.
fn register_storage(
    app: &mut App,
    executor: &ModelHandle<RequestFileEditsExecutor>,
    action_id: &AIAgentActionId,
) -> Rc<TestStorageState> {
    let state = TestStorageState::new();
    let storage = Box::new(TestStorage(state.clone()));
    executor.update(app, |executor, _| {
        executor.register_requested_edits(action_id, storage);
    });
    state
}

/// Builds a `RequestFileEdits` action with the given id.
fn edit_action(id: &AIAgentActionId) -> AIAgentAction {
    edit_action_with_edits(id, Vec::new())
}

fn edit_action_with_edits(id: &AIAgentActionId, file_edits: Vec<FileEdit>) -> AIAgentAction {
    AIAgentAction {
        id: id.clone(),
        task_id: TaskId::new("task".to_owned()),
        action: AIAgentActionType::RequestFileEdits {
            file_edits,
            title: None,
        },
        requires_result: true,
    }
}

/// Runs `execute` for the given action.
fn execute(
    app: &mut App,
    executor: &ModelHandle<RequestFileEditsExecutor>,
    action_id: &AIAgentActionId,
) -> AnyActionExecution {
    let action = edit_action(action_id);
    let conversation_id = AIConversationId::new();
    execute_action(app, executor, &action, conversation_id)
}

fn execute_action(
    app: &mut App,
    executor: &ModelHandle<RequestFileEditsExecutor>,
    action: &AIAgentAction,
    conversation_id: AIConversationId,
) -> AnyActionExecution {
    executor.update(app, |executor, ctx| {
        executor
            .execute(
                ExecuteActionInput {
                    action,
                    conversation_id,
                },
                ctx,
            )
            .into()
    })
}

#[test]
fn on_diffs_applied_seeds_registered_storage() {
    App::test((), |mut app| async move {
        let executor = add_executor(&mut app);
        let action_id = AIAgentActionId::from("edit-1".to_owned());
        let storage = register_storage(&mut app, &executor, &action_id);

        let (tx, _rx) = oneshot::channel();
        executor.update(&mut app, |executor, ctx| {
            executor.on_diffs_applied(
                Ok(vec![AIRequestedCodeDiff {
                    file_name: "/tmp/x.rs".to_owned(),
                    diff_type: DiffType::creation("fn main() {}\n".to_owned()),
                    failures: None,
                    original_content: String::new(),
                }]),
                action_id.clone(),
                tx,
                ctx,
            );
        });

        let seeded = storage.diffs.borrow_mut().take();
        let (diffs, session_type) = seeded.expect("registered storage should be seeded");
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_path(), "/tmp/x.rs");
        assert!(matches!(session_type, DiffSessionType::Local));
    });
}

#[test]
fn execute_accepts_through_registered_storage() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let executor = add_executor(&mut app);
        let action_id = AIAgentActionId::from("edit-1".to_owned());
        let storage = register_storage(&mut app, &executor, &action_id);

        let execution = execute(&mut app, &executor, &action_id);
        let AnyActionExecution::Async {
            execute_future,
            on_complete,
        } = execution
        else {
            panic!("expected async execution");
        };
        let result = execute_future.await;
        app.update(|ctx| on_complete(result, ctx));
        assert!(storage.accepted.get());
        // The entry stays registered until the action's terminal result
        // funnels through `discard_pending`.
        executor.update(&mut app, |executor, _| {
            assert!(executor.diff_storages.contains_key(&action_id));
        });
    });
}

#[test]
fn execute_reports_preprocess_failure() {
    App::test((), |mut app| async move {
        let executor = add_executor(&mut app);
        let action_id = AIAgentActionId::from("edit-failed".to_owned());
        executor.update(&mut app, |executor, _| {
            executor
                .diff_application_failures
                .insert(action_id.clone(), vec1![DiffApplicationError::EmptyDiff]);
        });

        let execution = execute(&mut app, &executor, &action_id);

        assert!(matches!(
            execution,
            AnyActionExecution::Sync(AIAgentActionResultType::RequestFileEdits(
                RequestFileEditsResult::DiffApplicationFailed { .. }
            ))
        ));
    });
}

#[test]
fn execute_without_prepared_diffs_is_not_ready() {
    App::test((), |mut app| async move {
        let executor = add_executor(&mut app);
        let action_id = AIAgentActionId::from("edit-1".to_owned());

        let execution = execute(&mut app, &executor, &action_id);

        assert!(matches!(execution, AnyActionExecution::NotReady));
    });
}

#[test]
fn discard_pending_drops_state_in_any_state() {
    App::test((), |mut app| async move {
        let executor = add_executor(&mut app);

        // Registered storage entry (e.g. rejected during review).
        let storage_id = AIAgentActionId::from("edit-storage".to_owned());
        register_storage(&mut app, &executor, &storage_id);
        executor.update(&mut app, |executor, _| {
            executor.discard_pending(&storage_id);
            assert!(!executor.diff_storages.contains_key(&storage_id));
        });

        // Failed entry (diff application failed during preprocess).
        let failed_id = AIAgentActionId::from("edit-failed".to_owned());
        executor.update(&mut app, |executor, _| {
            executor
                .diff_application_failures
                .insert(failed_id.clone(), vec1![DiffApplicationError::EmptyDiff]);
            executor.discard_pending(&failed_id);
            assert!(!executor.diff_application_failures.contains_key(&failed_id));
        });
    });
}
