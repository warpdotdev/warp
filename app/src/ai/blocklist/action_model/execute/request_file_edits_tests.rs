use std::cell::{Cell, RefCell};
use std::rc::Rc;

use ai::diff_validation::DiffType;
use async_channel::unbounded;
use futures::FutureExt;
use warpui::{App, AppContext, EntityId};

use super::*;
use crate::ai::agent::task::TaskId;
use crate::terminal::model::session::Sessions;
use crate::terminal::model_events::ModelEventDispatcher;

/// Shared observable state for a [`TestStorage`].
struct TestStorageState {
    diffs: RefCell<Option<(Vec<FileDiff>, DiffSessionType)>>,
    /// Whether `take_candidate_diffs` hands the diffs over (headless-placeholder
    /// behavior) or keeps them (review-surface behavior).
    relinquishes: bool,
    accepted: Cell<bool>,
}

impl TestStorageState {
    fn new(relinquishes: bool) -> Rc<Self> {
        Rc::new(Self {
            diffs: RefCell::new(None),
            relinquishes,
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

    fn take_candidate_diffs(
        &self,
        _app: &mut AppContext,
    ) -> Option<(Vec<FileDiff>, DiffSessionType)> {
        if self.0.relinquishes {
            self.0.diffs.borrow_mut().take()
        } else {
            None
        }
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
    let sessions = app.add_model(|_| Sessions::new_for_test());
    let (_, model_events_rx) = unbounded();
    let dispatcher =
        app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
    let active_session =
        app.add_model(|ctx| ActiveSession::new(sessions.clone(), dispatcher.clone(), ctx));
    app.add_model(|ctx| RequestFileEditsExecutor::new(active_session, EntityId::new(), ctx))
}

/// Builds a prepared diff creating `/tmp/x.rs`.
fn test_diff() -> FileDiff {
    FileDiff::new(
        String::new(),
        "/tmp/x.rs".to_owned(),
        DiffType::creation("fn main() {}\n".to_owned()),
    )
}

/// Registers a `TestStorage` for `action_id` and returns its observable state.
fn register_storage(
    app: &mut App,
    executor: &ModelHandle<RequestFileEditsExecutor>,
    action_id: &AIAgentActionId,
    relinquishes: bool,
) -> Rc<TestStorageState> {
    let state = TestStorageState::new(relinquishes);
    let storage = Box::new(TestStorage(state.clone()));
    executor.update(app, |executor, ctx| {
        executor.register_requested_edits(action_id, storage, ctx);
    });
    state
}

/// Builds a `RequestFileEdits` action with the given id.
fn edit_action(id: &AIAgentActionId) -> AIAgentAction {
    AIAgentAction {
        id: id.clone(),
        task_id: TaskId::new("task".to_owned()),
        action: AIAgentActionType::RequestFileEdits {
            file_edits: Vec::new(),
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
    executor.update(app, |executor, ctx| {
        executor
            .execute(
                ExecuteActionInput {
                    action: &action,
                    conversation_id,
                },
                ctx,
            )
            .into()
    })
}

#[test]
fn register_hands_prepared_diffs_to_new_surface() {
    App::test((), |mut app| async move {
        let executor = add_executor(&mut app);
        let action_id = AIAgentActionId::from("edit-1".to_owned());

        // A placeholder holding prepared diffs relinquishes them to a
        // registering review surface.
        let placeholder = register_storage(&mut app, &executor, &action_id, true);
        *placeholder.diffs.borrow_mut() = Some((vec![test_diff()], DiffSessionType::Local));

        let surface = register_storage(&mut app, &executor, &action_id, false);
        let seeded = surface.diffs.borrow_mut().take();
        let (diffs, session_type) = seeded.expect("surface should be seeded with the diffs");
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].file_path(), "/tmp/x.rs");
        assert!(matches!(session_type, DiffSessionType::Local));
        assert!(placeholder.diffs.borrow().is_none());
    });
}

#[test]
fn register_keeps_owner_that_does_not_relinquish() {
    App::test((), |mut app| async move {
        let executor = add_executor(&mut app);
        let action_id = AIAgentActionId::from("edit-1".to_owned());

        // A review surface owns the entry and never relinquishes.
        let owner = register_storage(&mut app, &executor, &action_id, false);
        *owner.diffs.borrow_mut() = Some((vec![test_diff()], DiffSessionType::Local));

        let late = register_storage(&mut app, &executor, &action_id, false);
        assert!(late.diffs.borrow().is_none());

        // The original owner still serves execution.
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let execution = execute(&mut app, &executor, &action_id);
        assert!(matches!(execution, AnyActionExecution::Async { .. }));
        assert!(owner.accepted.get());
        assert!(!late.accepted.get());
    });
}

#[test]
fn execute_accepts_through_registered_storage() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let executor = add_executor(&mut app);
        let action_id = AIAgentActionId::from("edit-1".to_owned());
        let storage = register_storage(&mut app, &executor, &action_id, false);

        let execution = execute(&mut app, &executor, &action_id);

        assert!(matches!(execution, AnyActionExecution::Async { .. }));
        assert!(storage.accepted.get());
        // The entry stays registered until the action's terminal result
        // funnels through `discard_pending`.
        executor.update(&mut app, |executor, _| {
            assert!(executor.pending_file_edits.contains_key(&action_id));
        });
    });
}

#[test]
fn execute_reports_preprocess_failure() {
    App::test((), |mut app| async move {
        let executor = add_executor(&mut app);
        let action_id = AIAgentActionId::from("edit-failed".to_owned());
        executor.update(&mut app, |executor, _| {
            executor.pending_file_edits.insert(
                action_id.clone(),
                PendingFileEdits::Failed(vec1![DiffApplicationError::EmptyDiff]),
            );
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
        register_storage(&mut app, &executor, &storage_id, false);
        executor.update(&mut app, |executor, _| {
            executor.discard_pending(&storage_id);
            assert!(!executor.pending_file_edits.contains_key(&storage_id));
        });

        // Failed entry (diff application failed during preprocess).
        let failed_id = AIAgentActionId::from("edit-failed".to_owned());
        executor.update(&mut app, |executor, _| {
            executor.pending_file_edits.insert(
                failed_id.clone(),
                PendingFileEdits::Failed(vec1![DiffApplicationError::EmptyDiff]),
            );
            executor.discard_pending(&failed_id);
            assert!(!executor.pending_file_edits.contains_key(&failed_id));
        });
    });
}
