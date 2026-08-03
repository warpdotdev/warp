use warpui::{App, EntityId};

use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType,
    ReadDocumentsRequest, ReadDocumentsResult,
};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentModel};
use crate::appearance::Appearance;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{
    CloudObjectMetadata, CloudObjectPermissions, CloudObjectStatuses, CloudObjectSyncStatus, Owner,
};
use crate::notebooks::{CloudNotebook, CloudNotebookModel};
use crate::server::ids::SyncId;
use crate::test_util::settings::initialize_settings_for_tests;

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| CloudModel::new(None, Vec::new(), None));
    app.add_singleton_model(|_| AIDocumentModel::new_for_test());
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
}

fn read_action(document_id: AIDocumentId) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from("read-documents-action".to_string()),
        task_id: TaskId::new("read-documents-task".to_string()),
        requires_result: true,
        action: AIAgentActionType::ReadDocuments(ReadDocumentsRequest {
            document_ids: vec![document_id],
        }),
    }
}

fn add_saved_plan_notebook(app: &mut App, document_id: AIDocumentId, content: &str) {
    let sync_id = SyncId::ServerId(123.into());
    let notebook = CloudNotebook::new(
        sync_id,
        CloudNotebookModel {
            title: "Saved plan".to_string(),
            data: content.to_string(),
            ai_document_id: Some(document_id),
            conversation_id: None,
        },
        CloudObjectMetadata {
            pending_changes_statuses: CloudObjectStatuses {
                content_sync_status: CloudObjectSyncStatus::NoLocalChanges,
                has_pending_metadata_change: false,
                has_pending_permissions_change: false,
                pending_untrash: false,
                pending_delete: false,
            },
            folder_id: None,
            revision: Default::default(),
            metadata_last_updated_ts: Default::default(),
            current_editor_uid: Default::default(),
            trashed_ts: Default::default(),
            is_welcome_object: false,
            creator_uid: None,
            last_editor_uid: None,
            last_task_run_ts: None,
        },
        CloudObjectPermissions {
            owner: Owner::mock_current_user(),
            guests: Vec::new(),
            permissions_last_updated_ts: None,
            anyone_with_link: None,
        },
    );
    CloudModel::handle(app).update(app, |cloud_model, _| {
        cloud_model.add_object(sync_id, notebook);
    });
}

#[test]
fn execute_lazily_hydrates_missing_plan_for_remote_child_without_local_parent() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let executor = app.add_model(|_| ReadDocumentsExecutor::new());
        let document_id = AIDocumentId::new();
        add_saved_plan_notebook(&mut app, document_id, "# Remote child plan");
        let child_conversation_id =
            BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
                let child_conversation_id =
                    history.start_new_conversation(EntityId::new(), false, false, false, ctx);
                history
                    .conversation_mut(&child_conversation_id)
                    .expect("child conversation should exist")
                    .set_parent_agent_id("non-local-parent-run-id".to_string());
                child_conversation_id
            });
        let action = read_action(document_id);

        let execution: AnyActionExecution = executor.update(&mut app, |executor, ctx| {
            executor
                .execute(
                    ExecuteActionInput {
                        action: &action,
                        conversation_id: child_conversation_id,
                    },
                    ctx,
                )
                .into()
        });

        let AnyActionExecution::Sync(AIAgentActionResultType::ReadDocuments(
            ReadDocumentsResult::Success { documents },
        )) = execution
        else {
            panic!("expected read_documents success");
        };
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].document_id, document_id);
        assert_eq!(documents[0].content, "# Remote child plan\n");
    });
}

#[test]
fn execute_returns_error_for_missing_document_id() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let executor = app.add_model(|_| ReadDocumentsExecutor::new());
        let missing_document_id = AIDocumentId::new();
        let action = read_action(missing_document_id);

        let execution: AnyActionExecution = executor.update(&mut app, |executor, ctx| {
            executor
                .execute(
                    ExecuteActionInput {
                        action: &action,
                        conversation_id: AIConversationId::new(),
                    },
                    ctx,
                )
                .into()
        });

        let AnyActionExecution::Sync(AIAgentActionResultType::ReadDocuments(
            ReadDocumentsResult::Error(error),
        )) = execution
        else {
            panic!("expected read_documents error");
        };
        assert!(error.contains(&missing_document_id.to_string()));
    });
}
