#![allow(warnings)]

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use ai::agent::orchestration_config::{OrchestrationConfig, OrchestrationConfigStatus};
use ai::diff_validation::DiffDelta;
// TODO(vorporeal): Remove this re-export at some point.
pub use ai::document::{AIDocumentId, AIDocumentVersion};
use chrono::{DateTime, Local, Utc};
use itertools::Itertools;
use uuid::Uuid;
use warp_editor::model::RichTextEditorModel;
use warp_editor::render::model::RichTextStyles;
use warp_errors::report_error;
use warp_multi_agent_api as maa_api;
use warpui::color::ColorU;
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity, WindowId};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::AIAgentActionId;
use crate::ai::ai_document_view::DEFAULT_PLANNING_DOCUMENT_TITLE;
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::appearance::Appearance;
use crate::auth::auth_state::AuthStateProvider;
use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::cloud_object::{CloudObject, CloudObjectEventEntrypoint, Owner};
use crate::drive::folders::CloudFolder;
use crate::drive::CloudObjectTypeAndId;
use crate::global_resource_handles::GlobalResourceHandlesProvider;
use crate::notebooks::editor::model::{
    FileLinkResolutionContext, NotebooksEditorModel, RichTextEditorModelEvent,
};
use crate::notebooks::editor::rich_text_styles;
use crate::notebooks::file::MarkdownDisplayMode;
use crate::notebooks::{CloudNotebookModel, NotebookId};
use crate::persistence::ModelEvent;
use crate::server::cloud_objects::update_manager::{
    InitiatedBy, ObjectOperation, OperationSuccessType, UpdateManager, UpdateManagerEvent,
};
use crate::server::ids::{ClientId, ServerId, SyncId};
use crate::settings::FontSettings;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::session::Session;
use crate::terminal::TerminalView;
use crate::throttle::throttle;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// The frequency at which we check for modifications and save the AI document to the server.
/// Uses the same 2-second period as notebooks for consistency.
const SAVE_PERIOD: Duration = Duration::from_secs(2);

struct AIDocumentSaveRequest {
    document_id: AIDocumentId,
}

/// The status of saving an AI Document to Warp Drive
pub enum AIDocumentSaveStatus {
    /// Not being synced with Warp Drive at all
    NotSaved,
    /// Is being saved to Warp Drive, but has not finished yet
    Saving,
    /// Has been saved to Warp Drive
    Saved,
}

impl AIDocumentSaveStatus {
    pub fn is_saved(&self) -> bool {
        matches!(self, AIDocumentSaveStatus::Saved)
    }
}

/// Tracks whether user edits to a planning document are known by the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AIDocumentUserEditStatus {
    /// Document content matches what agent generated; no user edits
    UpToDate,
    /// User has made edits that diverge from agent-generated content.
    /// When dirty, the document is always attached to the next query.
    Dirty,
}

impl AIDocumentUserEditStatus {
    pub fn is_dirty(&self) -> bool {
        matches!(self, AIDocumentUserEditStatus::Dirty)
    }
}

const PLAN_FOLDER_NAME: &str = "Plans";

/// Represents a document queued for creation in Warp Drive.
#[derive(Debug, Clone)]
struct PendingDocument {
    id: AIDocumentId,
    title: String,
    content: String,
}

#[derive(Debug, Clone)]
pub struct AIDocumentEarlierVersion {
    pub title: String,
    pub version: AIDocumentVersion,
    pub editor: ModelHandle<NotebooksEditorModel>,
    pub created_at: DateTime<Local>,
    pub restored_from: Option<AIDocumentVersion>,
}

#[derive(Debug, Clone)]
pub struct AIDocument {
    /// ID to sync with a cloud model with the server.
    /// Set when a document is saved to Warp Drive.
    pub sync_id: Option<SyncId>,
    pub title: String,
    pub version: AIDocumentVersion,
    pub editor: ModelHandle<NotebooksEditorModel>,
    pub user_edit_status: AIDocumentUserEditStatus,
    pub conversation_id: AIConversationId,
    pub created_at: DateTime<Local>,
    pub restored_from: Option<AIDocumentVersion>,
    /// The set of pane group entity IDs in which this document is currently visible.
    pub visible_in_pane_groups: HashSet<EntityId>,
}

pub enum AIDocumentInstance {
    Current(AIDocument),
    Earlier(AIDocumentEarlierVersion),
}

impl AIDocumentInstance {
    pub fn get_version(&self) -> AIDocumentVersion {
        match self {
            AIDocumentInstance::Current(doc) => doc.version,
            AIDocumentInstance::Earlier(doc) => doc.version,
        }
    }

    pub fn get_title(&self) -> String {
        match self {
            AIDocumentInstance::Current(doc) => doc.title.clone(),
            AIDocumentInstance::Earlier(doc) => doc.title.clone(),
        }
    }

    pub fn get_editor(&self) -> ModelHandle<NotebooksEditorModel> {
        match self {
            AIDocumentInstance::Current(doc) => doc.editor.clone(),
            AIDocumentInstance::Earlier(doc) => doc.editor.clone(),
        }
    }
}

/// Source of an update to an AI document.
#[derive(Debug, Clone, Copy)]
pub enum AIDocumentUpdateSource {
    User,
    Agent,
    Restoration,
}

/// Queued plan-card edit; cleared once it piggybacks onto an outbound query.
#[derive(Debug, Clone)]
pub struct DirtyOrchestrationEvent {
    pub plan_id: String,
    pub config: OrchestrationConfig,
    pub status: OrchestrationConfigStatus,
}

#[derive(Debug, Clone)]
pub struct AIDocumentModel {
    documents: HashMap<AIDocumentId, AIDocument>,
    earlier_versions: HashMap<AIDocumentId, Vec<AIDocumentEarlierVersion>>,
    /// The latest document ID for each conversation ID.
    /// Tracking separately to ensure the latest document in a conversation is set whenever we create a document.
    /// Otherwise, we'd need to keep track of timestamps to determine the latest document in a conversation.
    latest_document_id_by_conversation_id: HashMap<AIConversationId, AIDocumentId>,
    content_dirty_flags: HashMap<AIDocumentId, bool>,
    save_tx: async_channel::Sender<AIDocumentSaveRequest>,
    /// Queue of documents wait to be saved.
    /// Documents saves are buffered if the Plan folder is still being created.
    pending_document_queue: Vec<PendingDocument>,
    /// Mapping from (conversation_id, action_id, document_index) for streaming CreateDocuments
    /// tool calls to the corresponding AI document ID.
    streaming_create_documents: HashMap<(AIConversationId, AIAgentActionId, usize), AIDocumentId>,

    /// Pending plan-card edits, drained on the next outbound request.
    dirty_orchestration_events: HashMap<(AIConversationId, String), DirtyOrchestrationEvent>,
}

impl AIDocumentModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&UpdateManager::handle(ctx), |me, _, event, ctx| {
            me.handle_update_manager_event(event, ctx);
        });
        ctx.subscribe_to_model(&CloudModel::handle(ctx), |me, _, event, ctx| {
            me.handle_cloud_model_event(event, ctx);
        });

        // Subscribe to history events so we can hydrate the orchestration
        // config from OrchestrationConfigSnapshot messages that arrive
        // in the conversation's task message list.
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |me, _, event, ctx| {
                me.handle_history_event_for_orchestration_config(event, ctx);
            },
        );

        // Setup throttled save channel
        let (save_tx, save_rx) = async_channel::unbounded();
        ctx.spawn_stream_local(
            throttle(SAVE_PERIOD, save_rx),
            Self::handle_save_request,
            |_, _| {},
        );

        Self {
            documents: HashMap::new(),
            earlier_versions: HashMap::new(),
            latest_document_id_by_conversation_id: HashMap::new(),
            content_dirty_flags: HashMap::new(),
            save_tx,
            pending_document_queue: Vec::new(),
            streaming_create_documents: HashMap::new(),
            dirty_orchestration_events: HashMap::new(),
        }
    }

    #[cfg(test)]
    pub fn new_for_test() -> Self {
        let (save_tx, _save_rx) = async_channel::unbounded();
        Self {
            documents: HashMap::new(),
            earlier_versions: HashMap::new(),
            latest_document_id_by_conversation_id: HashMap::new(),
            content_dirty_flags: HashMap::new(),
            save_tx,
            pending_document_queue: Vec::new(),
            streaming_create_documents: HashMap::new(),
            dirty_orchestration_events: HashMap::new(),
        }
    }

    /// Sends a request to create a new cloud notebook with the document's contents.
    /// Returns true if the create document request was sent successfully (or if there was already a notebook entry).
    /// Actually creating the notebook is done asynchronously in the background.
    pub fn sync_to_warp_drive(&mut self, id: AIDocumentId, ctx: &mut ModelContext<Self>) -> bool {
        if self.reconcile_document_server_backing(&id, ctx) {
            return true;
        }
        let Some(document) = self.documents.get(&id) else {
            return false;
        };
        if document.sync_id.is_some() {
            // Already created. Return early.
            return true;
        }

        let title = document.title.clone();
        let content = document.editor.as_ref(ctx).markdown(ctx);

        let Some(owner) = Self::get_plan_owner(ctx) else {
            log::warn!("Failed to get owner while saving AI Document to Warp Drive. Skipping");
            return false;
        };

        let Some(plan_folder_id) = self.get_or_create_plan_folder(owner, ctx).into_server() else {
            // Plan folder is still being created (has ClientId only).
            // If we save using the ClientId as the parent folder, the document
            // will end up in a broken state once the folder is saved.
            // Queue the document for creation until the folder gets a ServerId.
            self.pending_document_queue
                .push(PendingDocument { id, title, content });

            if let Some(document) = self.documents.get_mut(&id) {
                let client_id = ClientId::new();
                document.sync_id = Some(SyncId::ClientId(client_id));
            }
            return true;
        };

        self.create_notebook_in_plan_folder(id, &title, &content, owner, plan_folder_id, ctx);
        ctx.emit(AIDocumentModelEvent::DocumentSaveStatusUpdated(id));
        true
    }

    pub fn get_document_save_status(&self, id: &AIDocumentId) -> AIDocumentSaveStatus {
        let sync_id = self.documents.get(id).and_then(|doc| doc.sync_id);
        if sync_id.and_then(|id| id.into_server()).is_some() {
            AIDocumentSaveStatus::Saved
        } else if sync_id.and_then(|id| id.into_client()).is_some() {
            AIDocumentSaveStatus::Saving
        } else {
            AIDocumentSaveStatus::NotSaved
        }
    }

    /// Publishes every document owned by a conversation before child-agent launch.
    pub(in crate::ai) fn publish_documents_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> Vec<AIDocumentId> {
        self.reconcile_all_document_server_backing(ctx);
        let document_ids = self
            .documents
            .iter()
            .filter_map(|(document_id, document)| {
                (document.conversation_id == conversation_id).then_some(*document_id)
            })
            .collect::<Vec<_>>();
        let mut awaiting_server_backing = Vec::new();

        for document_id in document_ids {
            match self.get_document_save_status(&document_id) {
                AIDocumentSaveStatus::Saved => {
                    self.maybe_update_cloud_notebook_data(&document_id, ctx);
                }
                AIDocumentSaveStatus::Saving => {
                    self.refresh_saving_document_content(&document_id, ctx);
                    awaiting_server_backing.push(document_id);
                }
                AIDocumentSaveStatus::NotSaved => {
                    if !self.sync_to_warp_drive(document_id, ctx) {
                        report_error!(
                            "Failed to publish plan document to Warp Drive before child-agent launch.",
                            extra: { "document_id" => %document_id }
                        );
                    } else if !self.get_document_save_status(&document_id).is_saved() {
                        awaiting_server_backing.push(document_id);
                    }
                }
            }
        }

        awaiting_server_backing
    }

    /// Reconciles a document with an existing server-backed Warp Drive notebook.
    fn reconcile_document_server_backing(
        &mut self,
        document_id: &AIDocumentId,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let server_sync_id = CloudModel::as_ref(ctx)
            .get_all_active_notebooks()
            .find(|notebook| {
                notebook.id.into_server().is_some()
                    && notebook.model().ai_document_id.as_ref() == Some(document_id)
            })
            .map(|notebook| notebook.id);
        let Some(server_sync_id) = server_sync_id else {
            return false;
        };
        self.set_document_server_backing(*document_id, server_sync_id, ctx);
        true
    }

    /// Reconciles all loaded documents with server-backed Warp Drive notebooks.
    fn reconcile_all_document_server_backing(&mut self, ctx: &mut ModelContext<Self>) {
        let document_ids = self.documents.keys().copied().collect::<Vec<_>>();
        for document_id in document_ids {
            self.reconcile_document_server_backing(&document_id, ctx);
        }
    }

    /// Refreshes the latest content for a plan whose Warp Drive creation is in progress.
    fn refresh_saving_document_content(
        &mut self,
        document_id: &AIDocumentId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(document) = self.documents.get(document_id) else {
            return;
        };
        let title = document.title.clone();
        let content = document.editor.as_ref(ctx).markdown(ctx);
        let sync_id = document.sync_id;

        for pending in self
            .pending_document_queue
            .iter_mut()
            .filter(|pending| pending.id == *document_id)
        {
            pending.title.clone_from(&title);
            pending.content.clone_from(&content);
        }

        if sync_id.is_some_and(|sync_id| CloudModel::as_ref(ctx).get_notebook(&sync_id).is_some()) {
            self.maybe_update_cloud_notebook_data(document_id, ctx);
        }
    }

    fn handle_cloud_model_event(&mut self, event: &CloudModelEvent, ctx: &mut ModelContext<Self>) {
        match event {
            CloudModelEvent::ObjectSynced { server_id, .. } => {
                self.reconcile_server_backed_notebook(SyncId::ServerId(*server_id), ctx);
            }
            CloudModelEvent::ObjectCreated {
                type_and_id: CloudObjectTypeAndId::Notebook(sync_id),
            }
            | CloudModelEvent::ObjectUpdated {
                type_and_id: CloudObjectTypeAndId::Notebook(sync_id),
                ..
            } => {
                self.reconcile_server_backed_notebook(*sync_id, ctx);
            }
            CloudModelEvent::InitialLoadCompleted => {
                self.reconcile_all_document_server_backing(ctx);
            }
            CloudModelEvent::ObjectMoved { .. }
            | CloudModelEvent::ObjectUpdated { .. }
            | CloudModelEvent::ObjectTrashed { .. }
            | CloudModelEvent::ObjectUntrashed { .. }
            | CloudModelEvent::ObjectDeleted { .. }
            | CloudModelEvent::ObjectPermissionsUpdated { .. }
            | CloudModelEvent::ObjectForceExpanded { .. }
            | CloudModelEvent::ObjectCreated { .. }
            | CloudModelEvent::NotebookEditorChangedFromServer { .. } => {}
        }
    }
    /// Reconciles one server-backed notebook with its loaded AI document.
    fn reconcile_server_backed_notebook(&mut self, sync_id: SyncId, ctx: &mut ModelContext<Self>) {
        if sync_id.into_server().is_none() {
            return;
        }
        let document_id = CloudModel::as_ref(ctx)
            .get_notebook(&sync_id)
            .and_then(|notebook| notebook.model().ai_document_id);
        if let Some(document_id) = document_id {
            self.set_document_server_backing(document_id, sync_id, ctx);
        }
    }

    /// Updates a document and its conversation artifact with server backing.
    fn set_document_server_backing(
        &mut self,
        document_id: AIDocumentId,
        sync_id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(document) = self.documents.get_mut(&document_id) else {
            return;
        };
        if document.sync_id == Some(sync_id) {
            return;
        }
        document.sync_id = Some(sync_id);
        let conversation_id = document.conversation_id;
        ctx.emit(AIDocumentModelEvent::DocumentSaveStatusUpdated(document_id));

        let Some(server_id) = sync_id.into_server() else {
            return;
        };
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
            let terminal_view_id =
                history_model.terminal_surface_id_for_conversation(&conversation_id);
            if let Some(conversation) = history_model.conversation_mut(&conversation_id) {
                conversation.update_plan_notebook_uid(
                    document_id,
                    NotebookId::from(server_id),
                    terminal_view_id,
                    ctx,
                );
            }
        });
    }

    fn handle_update_manager_event(
        &mut self,
        event: &UpdateManagerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let UpdateManagerEvent::ObjectOperationComplete { result } = event else {
            return;
        };
        if !matches!(result.operation, ObjectOperation::Create { .. })
            || result.success_type != OperationSuccessType::Success
        {
            return;
        }
        if result.server_id.is_none() {
            return;
        };

        // If we're waiting on a Plans folder to complete creation, ensure the Plans folder exists
        // (creating it if needed) and if it has a ServerId, process the pending document queue.
        //
        // NOTE: this handler runs for *all* Warp Drive object creations, so we must only create the
        // Plans folder when we actually have a plan notebook waiting to be created.
        if !self.pending_document_queue.is_empty() {
            if let Some(owner) = Self::get_plan_owner(ctx) {
                if let Some(folder_id) = self.get_or_create_plan_folder(owner, ctx).into_server() {
                    let queue = std::mem::take(&mut self.pending_document_queue);

                    for pending in queue {
                        self.create_notebook_in_plan_folder(
                            pending.id,
                            &pending.title,
                            &pending.content,
                            owner,
                            folder_id,
                            ctx,
                        );
                        ctx.emit(AIDocumentModelEvent::DocumentSaveStatusUpdated(pending.id));
                    }
                }
            }
        }
    }

    /// Create a new document with default title/content and return its ID.
    pub fn create_document(
        &mut self,
        title: impl Into<String>,
        content: impl Into<String>,
        conversation_id: AIConversationId,
        file_link_resolution_context: Option<FileLinkResolutionContext>,
        ctx: &mut ModelContext<Self>,
    ) -> AIDocumentId {
        let id = AIDocumentId::new();
        self.create_document_internal(
            id,
            title,
            content,
            AIDocumentUpdateSource::Agent,
            conversation_id,
            file_link_resolution_context,
            Local::now(),
            ctx,
        );
        id
    }

    /// Create a document from an existing Warp Drive notebook.
    pub fn create_document_from_notebook(
        &mut self,
        ai_document_id: AIDocumentId,
        sync_id: SyncId,
        title: impl Into<String>,
        content: impl Into<String>,
        conversation_id: AIConversationId,
        file_link_resolution_context: Option<FileLinkResolutionContext>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.create_document_internal(
            ai_document_id,
            title,
            content,
            AIDocumentUpdateSource::Restoration,
            conversation_id,
            file_link_resolution_context,
            Local::now(),
            ctx,
        );

        if let Some(doc) = self.documents.get_mut(&ai_document_id) {
            doc.sync_id = Some(sync_id);
            ctx.emit(AIDocumentModelEvent::DocumentSaveStatusUpdated(
                ai_document_id,
            ));
        }
    }

    /// Hydrates a saved plan notebook into the target conversation.
    pub(in crate::ai) fn hydrate_saved_plan_from_warp_drive(
        &mut self,
        ai_document_id: AIDocumentId,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), String> {
        let notebook = CloudModel::as_ref(ctx)
            .get_all_active_notebooks()
            .find(|notebook| notebook.model().ai_document_id.as_ref() == Some(&ai_document_id))
            .map(|notebook| {
                (
                    notebook.id,
                    notebook.model().title.clone(),
                    notebook.model().data.clone(),
                )
            })
            .ok_or_else(|| {
                format!("Plan document {ai_document_id} was not found in Warp Drive.")
            })?;
        let (sync_id, title, content) = notebook;
        if sync_id.into_server().is_none() {
            return Err(format!(
                "Plan document {ai_document_id} is not backed by a saved Warp Drive notebook."
            ));
        }

        self.latest_document_id_by_conversation_id
            .insert(conversation_id, ai_document_id);

        if let Some(document) = self.documents.get_mut(&ai_document_id) {
            if document.sync_id != Some(sync_id) {
                document.sync_id = Some(sync_id);
                ctx.emit(AIDocumentModelEvent::DocumentSaveStatusUpdated(
                    ai_document_id,
                ));
            }
            return Ok(());
        }

        self.create_document_from_notebook(
            ai_document_id,
            sync_id,
            title,
            content,
            conversation_id,
            None,
            ctx,
        );
        Ok(())
    }

    fn create_document_internal(
        &mut self,
        id: AIDocumentId,
        title: impl Into<String>,
        content: impl Into<String>,
        source: AIDocumentUpdateSource,
        conversation_id: AIConversationId,
        file_link_resolution_context: Option<FileLinkResolutionContext>,
        created_at: DateTime<Local>,
        ctx: &mut ModelContext<Self>,
    ) {
        let editor = Self::create_editor_model(content, file_link_resolution_context, ctx);

        // Subscribe to editor content changes
        ctx.subscribe_to_model(&editor, move |me, _, event, ctx| {
            me.handle_editor_event(&id, event, ctx);
        });

        let doc = AIDocument {
            sync_id: None,
            title: title.into(),
            version: AIDocumentVersion::default(),
            editor,
            user_edit_status: AIDocumentUserEditStatus::UpToDate,
            conversation_id,
            created_at,
            restored_from: None,
            visible_in_pane_groups: HashSet::new(),
        };
        self.latest_document_id_by_conversation_id
            .insert(conversation_id, id);
        self.documents.insert(id, doc);
        // Emit event for document creation
        ctx.emit(AIDocumentModelEvent::DocumentUpdated {
            document_id: id,
            version: AIDocumentVersion::default(),
            source,
        });
        // Enqueue a save so the initial content is persisted to SQLite.
        // The editor subscription misses the first ContentChanged from
        // create_editor_model because it fires before the subscription is
        // wired up.
        self.enqueue_save(&id);
    }

    /// Returns an existing streaming document for a CreateDocuments tool call, or creates one.
    /// Returns true for the boolean return value if a new document was created.
    ///
    /// This is keyed by (conversation_id, action_id, document_index) so that streaming updates
    /// for the same tool call map to the same document.
    pub fn get_or_create_streaming_document_for_create_documents(
        &mut self,
        conversation_id: AIConversationId,
        action_id: &AIAgentActionId,
        document_index: usize,
        title: impl Into<String>,
        initial_content: impl Into<String>,
        file_link_resolution_context: Option<FileLinkResolutionContext>,
        ctx: &mut ModelContext<Self>,
    ) -> (AIDocumentId, bool) {
        let key = (conversation_id, action_id.clone(), document_index);
        if let Some(existing_id) = self.streaming_create_documents.get(&key) {
            return (*existing_id, false);
        }

        let id = AIDocumentId::new();
        self.create_document_internal(
            id,
            title,
            initial_content,
            AIDocumentUpdateSource::Agent,
            conversation_id,
            file_link_resolution_context,
            Local::now(),
            ctx,
        );
        self.streaming_create_documents.insert(key, id);
        (id, true)
    }

    /// Apply a streamed agent-origin update to the given document's content.
    ///
    /// This is used for incremental tool call updates and does not modify the
    /// user edit status.
    pub fn apply_streamed_agent_update(
        &mut self,
        id: &AIDocumentId,
        new_title: &str,
        new_content: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(doc) = self.documents.get_mut(id) else {
            return;
        };

        doc.title = new_title.to_owned();
        let editor_handle = doc.editor.clone();
        editor_handle.update(ctx, |editor, editor_ctx| {
            editor.update_to_new_markdown(new_content, editor_ctx);
        });

        ctx.emit(AIDocumentModelEvent::DocumentUpdated {
            document_id: *id,
            version: doc.version,
            source: AIDocumentUpdateSource::Agent,
        });
    }

    pub fn is_document_creation_streaming(&self, id: &AIDocumentId) -> bool {
        self.streaming_create_documents
            .values()
            .any(|doc_id| *doc_id == *id)
    }

    /// Returns the streaming document ID for a CreateDocuments tool call document, if any.
    pub fn streaming_document_id_for_create_documents(
        &self,
        conversation_id: &AIConversationId,
        action_id: &AIAgentActionId,
        document_index: usize,
    ) -> Option<AIDocumentId> {
        self.streaming_create_documents
            .get(&(*conversation_id, action_id.clone(), document_index))
            .copied()
    }

    /// Clears all streaming document mappings for the given CreateDocuments action.
    pub fn clear_streaming_documents_for_action(
        &mut self,
        conversation_id: &AIConversationId,
        action_id: &AIAgentActionId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.streaming_create_documents
            .retain(|(conv_id, act_id, _), _| conv_id != conversation_id || act_id != action_id);
        ctx.emit(AIDocumentModelEvent::StreamingDocumentsCleared(
            *conversation_id,
        ));
    }

    pub fn clear_streaming_documents_for_conversation(
        &mut self,
        conversation_id: &AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.streaming_create_documents
            .retain(|(conv_id, _, _), _| conv_id != conversation_id);
        ctx.emit(AIDocumentModelEvent::StreamingDocumentsCleared(
            *conversation_id,
        ));
    }

    /// Get a copy of the current document by id.
    pub fn get_current_document(&self, id: &AIDocumentId) -> Option<AIDocument> {
        self.documents.get(id).cloned()
    }

    /// Deletes the given document and its version history.
    pub fn delete_document(&mut self, id: &AIDocumentId) {
        self.documents.remove(id);
        self.earlier_versions.remove(id);
    }

    pub fn get_document_id_by_conversation_id(&self, id: AIConversationId) -> Option<AIDocumentId> {
        self.latest_document_id_by_conversation_id.get(&id).cloned()
    }

    /// Get all documents for a given conversation, sorted by `created_at` ascending
    /// (oldest first, most recent last).
    pub fn get_all_documents_for_conversation(
        &self,
        conversation_id: AIConversationId,
    ) -> Vec<(AIDocumentId, AIDocument)> {
        let mut docs: Vec<_> = self
            .documents
            .iter()
            .filter(|(_, doc)| doc.conversation_id == conversation_id)
            .map(|(id, doc)| (*id, doc.clone()))
            .collect();
        docs.sort_by_key(|(_, doc)| doc.created_at);
        docs
    }

    pub fn get_conversation_id_for_document_id(
        &self,
        id: &AIDocumentId,
    ) -> Option<AIConversationId> {
        self.documents.get(id).map(|doc| doc.conversation_id)
    }

    fn get_current_document_mut(&mut self, id: &AIDocumentId) -> Option<&mut AIDocument> {
        self.documents.get_mut(id)
    }

    /// Set the user edit status for a document.
    /// Returns whether the document was found.
    pub fn set_user_edit_status(
        &mut self,
        id: &AIDocumentId,
        status: AIDocumentUserEditStatus,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if let Some(doc) = self.get_current_document_mut(id) {
            doc.user_edit_status = status;
            ctx.emit(AIDocumentModelEvent::DocumentUserEditStatusUpdated {
                document_id: *id,
                status,
            });
            true
        } else {
            false
        }
    }

    /// Mark a document as visible (or not) in the given pane group.
    pub fn set_document_visible(
        &mut self,
        id: &AIDocumentId,
        pane_group_id: EntityId,
        is_visible: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(doc) = self.get_current_document_mut(id) else {
            return;
        };
        let changed = if is_visible {
            doc.visible_in_pane_groups.insert(pane_group_id)
        } else {
            doc.visible_in_pane_groups.remove(&pane_group_id)
        };
        if changed {
            ctx.emit(AIDocumentModelEvent::DocumentVisibilityChanged(*id));
        }
    }

    /// Check if any document for the given conversation is visible in the specified pane group.
    pub fn is_document_visible_by_conversation_in_pane_group(
        &self,
        conversation_id: &AIConversationId,
        pane_group_id: EntityId,
    ) -> bool {
        self.documents.values().any(|doc| {
            doc.conversation_id == *conversation_id
                && doc.visible_in_pane_groups.contains(&pane_group_id)
        })
    }

    /// Check if any document for the given conversation is visible in any pane group.
    pub fn is_document_visible_by_conversation(&self, conversation_id: &AIConversationId) -> bool {
        self.documents.values().any(|doc| {
            doc.conversation_id == *conversation_id && !doc.visible_in_pane_groups.is_empty()
        })
    }

    /// Check if a specific document is visible in any pane group.
    pub fn is_document_visible(&self, document_id: &AIDocumentId) -> bool {
        self.documents
            .get(document_id)
            .is_some_and(|doc| !doc.visible_in_pane_groups.is_empty())
    }

    /// Get a copy of a document by id and version.
    /// Could be the current document if the version is the latest version, or an earlier version if the version is older.
    pub fn get_document(
        &self,
        id: &AIDocumentId,
        version: AIDocumentVersion,
    ) -> Option<AIDocumentInstance> {
        let current_document = self.get_current_document(id)?;
        if current_document.version == version {
            Some(AIDocumentInstance::Current(current_document))
        } else {
            let earlier_versions = self.earlier_versions.get(id)?;
            Some(AIDocumentInstance::Earlier(
                earlier_versions
                    .iter()
                    .find(|v| v.version == version)?
                    .clone(),
            ))
        }
    }

    pub fn get_document_warp_drive_object_link(
        &self,
        id: &AIDocumentId,
        ctx: &AppContext,
    ) -> Option<String> {
        let document = self.documents.get(id)?;
        if !self.get_document_save_status(id).is_saved() {
            return None;
        }
        let sync_id = document.sync_id?;
        let notebook = CloudModel::as_ref(ctx).get_notebook(&sync_id)?;
        notebook.object_link()
    }

    /// Get the raw markdown content of a document by id.
    pub fn get_document_content(
        &self,
        id: &AIDocumentId,
        ctx: &warpui::AppContext,
    ) -> Option<String> {
        let doc = self.documents.get(id)?;
        Some(doc.editor.as_ref(ctx).markdown_unescaped(ctx))
    }

    /// Apply persisted content from SQLite on top of conversation-restored content.
    /// If the persisted content differs from the current editor content, update the
    /// editor and mark the document as Dirty so it gets attached to the next query.
    /// If the document doesn't exist (conversation wasn't restored), create it with
    /// the persisted content.
    pub fn apply_persisted_content(
        &mut self,
        id: AIDocumentId,
        persisted_content: &str,
        persisted_title: Option<&str>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(doc) = self.documents.get_mut(&id) else {
            // Document doesn't exist from conversation restoration.
            // Create it from the persisted content so the pane has something
            // to display. Use a synthetic conversation ID since the original
            // conversation wasn't restored.
            log::info!(
                "Creating document {id} from persisted SQLite content (conversation not restored)"
            );
            let title = persisted_title.unwrap_or(DEFAULT_PLANNING_DOCUMENT_TITLE);
            self.create_document_internal(
                id,
                title,
                persisted_content,
                AIDocumentUpdateSource::Restoration,
                // We don't have the conversation ID this is for - this is free floating and not connected to any conversation
                // so create a random one.
                AIConversationId::new(),
                None,
                Local::now(),
                ctx,
            );
            return;
        };

        let current_content = doc.editor.as_ref(ctx).markdown_unescaped(ctx);
        if current_content == persisted_content {
            log::info!(
                "Persisted SQLite content for document {id} is the same as the current content"
            );
            return;
        }

        log::info!("Applying persisted SQLite content for document {id} (content differs from conversation restoration)");
        doc.editor.update(ctx, |editor, editor_ctx| {
            editor.reset_with_markdown(persisted_content, editor_ctx);
        });

        // Mark as dirty so the updated plan is attached to the next agent query
        doc.user_edit_status = AIDocumentUserEditStatus::Dirty;
        let version = doc.version;
        ctx.emit(AIDocumentModelEvent::DocumentUpdated {
            document_id: id,
            version,
            source: AIDocumentUpdateSource::Restoration,
        });
        ctx.emit(AIDocumentModelEvent::DocumentUserEditStatusUpdated {
            document_id: id,
            status: AIDocumentUserEditStatus::Dirty,
        });
    }

    /// Update the title of a document.
    pub fn update_title(
        &mut self,
        id: &AIDocumentId,
        new_title: impl Into<String>,
        source: AIDocumentUpdateSource,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(doc) = self.documents.get_mut(id) {
            doc.title = new_title.into();
            ctx.emit(AIDocumentModelEvent::DocumentUpdated {
                document_id: *id,
                version: doc.version,
                source,
            });
        }
    }

    /// Create a new, unbound editor model with the given content.
    fn create_editor_model(
        content: impl Into<String>,
        file_link_resolution_context: Option<FileLinkResolutionContext>,
        ctx: &mut ModelContext<Self>,
    ) -> ModelHandle<NotebooksEditorModel> {
        ctx.add_model(|ctx| {
            // Get appearance and font settings from the app context
            let appearance = Appearance::as_ref(ctx);
            let font_settings = FontSettings::as_ref(ctx);
            // Use the same rich text styles as notebooks for consistency
            let styles = rich_text_styles(appearance, font_settings);

            let mut model = NotebooksEditorModel::new_unbound(styles, ctx);
            model.set_default_mermaid_display_mode(MarkdownDisplayMode::Rendered, ctx);
            model.set_file_link_resolution_context(file_link_resolution_context);

            let content = content.into();
            if !content.is_empty() {
                model.reset_with_markdown(&content, ctx);
            }
            model
        })
    }

    /// Save the current document state as a version and prepare for a new version.
    /// Returns a mutable reference to the document if successful.
    fn create_new_document_version(
        &mut self,
        id: &AIDocumentId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<&mut AIDocument> {
        let doc = self.documents.get_mut(id)?;

        // Create new editor instance to avoid persisting updates to older versions
        // Preserve the file link resolution context from the current editor, if any.
        let file_link_resolution_context = doc
            .editor
            .as_ref(ctx)
            .file_link_resolution_context()
            .cloned();
        let editor = Self::create_editor_model(
            doc.editor.as_ref(ctx).markdown_unescaped(ctx),
            file_link_resolution_context,
            ctx,
        );

        let earlier_version = AIDocumentEarlierVersion {
            title: doc.title.clone(),
            version: doc.version,
            editor,
            created_at: doc.created_at,
            restored_from: doc.restored_from,
        };

        self.earlier_versions
            .entry(*id)
            .or_insert_with(Vec::new)
            .push(earlier_version);

        doc.version = doc.version.next();
        doc.created_at = Local::now();
        doc.restored_from = None;

        Some(doc)
    }

    /// Apply diffs to a document.
    pub fn create_new_version_and_apply_diffs(
        &mut self,
        id: &AIDocumentId,
        diffs: Vec<DiffDelta>,
        source: AIDocumentUpdateSource,
        ctx: &mut ModelContext<Self>,
    ) -> Option<AIDocumentVersion> {
        if let Some(doc) = self.create_new_document_version(id, ctx) {
            doc.editor.update(ctx, |editor, editor_ctx| {
                editor.apply_diffs(diffs, editor_ctx);
            });
            let version = doc.version;
            // Agent edits create a new version, so they will be persisted by the sqlite writer
            ctx.emit(AIDocumentModelEvent::DocumentUpdated {
                document_id: *id,
                version,
                source,
            });
            self.maybe_update_cloud_notebook_data(id, ctx);
            Some(version)
        } else {
            None
        }
    }

    /// Restore the initial version of a document.
    pub fn restore_document(
        &mut self,
        id: AIDocumentId,
        conversation_id: AIConversationId,
        title: impl Into<String>,
        content: impl Into<String>,
        created_at: DateTime<Local>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.create_document_internal(
            id,
            title,
            content,
            AIDocumentUpdateSource::Restoration,
            conversation_id,
            None,
            created_at,
            ctx,
        );

        self.reconcile_document_server_backing(&id, ctx);
    }

    /// This is used for restoring EditDocuments results where we already have the final content.
    /// Creates a new version of the document with directly-provided content.
    /// This expects `restore_document` to have already been called.
    pub fn restore_document_edit(
        &mut self,
        id: &AIDocumentId,
        new_content: impl Into<String>,
        created_at: DateTime<Local>,
        ctx: &mut ModelContext<Self>,
    ) -> Option<AIDocumentVersion> {
        if let Some(doc) = self.create_new_document_version(id, ctx) {
            let content = new_content.into();
            doc.editor.update(ctx, |editor, editor_ctx| {
                editor.reset_with_markdown(&content, editor_ctx);
            });
            doc.created_at = created_at;
            ctx.emit(AIDocumentModelEvent::DocumentUpdated {
                document_id: *id,
                version: doc.version,
                source: AIDocumentUpdateSource::Restoration,
            });
            Some(doc.version)
        } else {
            None
        }
    }

    /// Handle editor model events and enqueue saves for content changes.
    fn handle_editor_event(
        &mut self,
        document_id: &AIDocumentId,
        event: &RichTextEditorModelEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if let RichTextEditorModelEvent::ContentChanged(edit_origin) = event {
            self.enqueue_save(document_id);
            // Mark document as Dirty on user edit
            if edit_origin.from_user() {
                if let Some(doc) = self.documents.get_mut(document_id) {
                    if !doc.user_edit_status.is_dirty() {
                        doc.user_edit_status = AIDocumentUserEditStatus::Dirty;
                        ctx.emit(AIDocumentModelEvent::DocumentUserEditStatusUpdated {
                            document_id: *document_id,
                            status: AIDocumentUserEditStatus::Dirty,
                        });
                    }
                }
            }
        }
    }

    /// Enqueue a save for the given document.
    fn enqueue_save(&mut self, document_id: &AIDocumentId) {
        self.content_dirty_flags.insert(*document_id, true);
        if let Err(e) = self.save_tx.try_send(AIDocumentSaveRequest {
            document_id: *document_id,
        }) {
            report_error!(
                anyhow::Error::new(e).context("Error enqueueing content save"),
                extra: { "document_id" => %document_id }
            );
        }
    }

    /// Handle save requests from the throttled channel.
    fn handle_save_request(
        &mut self,
        request: AIDocumentSaveRequest,
        ctx: &mut ModelContext<Self>,
    ) {
        if self
            .content_dirty_flags
            .get(&request.document_id)
            .copied()
            .unwrap_or(false)
        {
            self.maybe_update_cloud_notebook_data(&request.document_id, ctx);
            self.persist_content_to_sqlite(&request.document_id, ctx);
            self.content_dirty_flags.insert(request.document_id, false);
        }
    }

    /// Persist the current document content to SQLite for session restoration.
    fn persist_content_to_sqlite(&self, id: &AIDocumentId, ctx: &mut ModelContext<Self>) {
        let Some(doc) = self.documents.get(id) else {
            return;
        };
        let Some(sender) = GlobalResourceHandlesProvider::as_ref(ctx)
            .get()
            .model_event_sender
            .clone()
        else {
            return;
        };
        let content = doc.editor.as_ref(ctx).markdown_unescaped(ctx);
        let event = ModelEvent::SaveAIDocumentContent {
            document_id: id.to_string(),
            content,
            version: doc.version.0 as i32,
            title: doc.title.clone(),
        };
        if let Err(err) = sender.try_send(event) {
            report_error!(
                anyhow::Error::new(err).context("Error persisting AI document content"),
                extra: { "id" => %id }
            );
        }
    }

    fn maybe_update_cloud_notebook_data(
        &mut self,
        id: &AIDocumentId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(doc) = self.documents.get(id) else {
            return;
        };
        let Some(sync_id) = doc.sync_id else {
            return;
        };
        let content = doc.editor.as_ref(ctx).markdown(ctx);
        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.update_notebook_data(content.into(), sync_id, ctx);
        });
    }

    /// Get a specific version of a document by version.
    pub fn get_earlier_document_version(
        &self,
        id: &AIDocumentId,
        version: AIDocumentVersion,
    ) -> Option<&AIDocumentEarlierVersion> {
        self.earlier_versions
            .get(id)?
            .iter()
            .find(|v| v.version == version)
    }

    /// Get all earlier versions of a document.
    pub fn get_earlier_document_versions(
        &self,
        id: &AIDocumentId,
    ) -> Option<&Vec<AIDocumentEarlierVersion>> {
        self.earlier_versions.get(id)
    }

    /// Get the appropriate owner for plan documents.
    /// For service accounts, returns the team drive owner.
    /// For regular users, returns the personal drive owner.
    fn get_plan_owner(ctx: &AppContext) -> Option<Owner> {
        let is_service_account = AuthStateProvider::as_ref(ctx).get().is_service_account();

        if is_service_account {
            // If the SA doesn't have a team, we'll skip the plan sync in the caller
            UserWorkspaces::as_ref(ctx)
                .current_team_uid()
                .map(|team_uid| Owner::Team { team_uid })
        } else {
            UserWorkspaces::as_ref(ctx).personal_drive(ctx)
        }
    }

    /// Get or create the Plans folder in the appropriate drive.
    /// Returns the SyncId of the folder (new ClientId if just created).
    fn get_or_create_plan_folder(&mut self, owner: Owner, ctx: &mut ModelContext<Self>) -> SyncId {
        let cloud_model = CloudModel::as_ref(ctx);

        // Search for existing Plans folder at root level in the appropriate drive.
        let existing_folder = cloud_model.get_all_active_folders().find(|folder| {
            folder.model().name == PLAN_FOLDER_NAME
                && folder.metadata.folder_id.is_none()
                && folder.permissions.owner == owner
        });

        if let Some(folder) = existing_folder {
            return folder.id;
        }

        // Folder doesn't exist, create it.
        let client_id = ClientId::new();
        let folder_id = SyncId::ClientId(client_id);

        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.create_folder(
                PLAN_FOLDER_NAME.to_string(),
                owner,
                client_id,
                None,
                false,
                InitiatedBy::System,
                ctx,
            );
        });

        folder_id
    }

    /// Look up server_conversation_token for a document's conversation.
    fn get_server_conversation_id(&self, id: &AIDocumentId, ctx: &AppContext) -> Option<String> {
        self.documents.get(id).and_then(|doc| {
            BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&doc.conversation_id)
                .and_then(|conv| conv.server_conversation_token())
                .map(|token| token.as_str().to_string())
        })
    }

    /// Helper method to create a notebook in the Plan folder.
    fn create_notebook_in_plan_folder(
        &mut self,
        id: AIDocumentId,
        title: &str,
        content: &str,
        owner: Owner,
        plan_folder_id: ServerId,
        ctx: &mut ModelContext<Self>,
    ) {
        let client_id = ClientId::new();
        let server_conversation_id = self.get_server_conversation_id(&id, ctx);
        if let Some(document) = self.documents.get_mut(&id) {
            document.sync_id = Some(SyncId::ClientId(client_id));
        } else {
            log::warn!("Document {} not found when creating notebook", id);
            return;
        }

        let notebook_model = CloudNotebookModel {
            title: title.to_string(),
            data: content.to_string(),
            ai_document_id: Some(id),
            conversation_id: server_conversation_id,
        };

        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.create_notebook(
                client_id,
                owner,
                Some(SyncId::ServerId(plan_folder_id)),
                notebook_model,
                CloudObjectEventEntrypoint::Unknown,
                true,
                ctx,
            );
        });
    }

    // ── Orchestration config: history event → hydration ──────────

    fn handle_history_event_for_orchestration_config(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        // On restore, scan all restored conversations for the last snapshot.
        if let BlocklistAIHistoryEvent::RestoredConversations {
            conversation_ids, ..
        } = event
        {
            for cid in conversation_ids {
                self.scan_conversation_for_orchestration_config(*cid, ctx);
            }
        }
    }

    /// Scans all messages across all tasks in a restored conversation to find
    /// per-plan `OrchestrationConfigSnapshot` messages and hydrate the config map.
    /// Backward scan: for each `plan_id`, the first snapshot found (most recent) wins.
    fn scan_conversation_for_orchestration_config(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        use std::collections::HashMap;
        let configs = {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            let Some(conversation) = history.conversation(&conversation_id) else {
                return;
            };
            let mut configs: HashMap<String, (OrchestrationConfig, OrchestrationConfigStatus)> =
                HashMap::new();
            let messages: Vec<_> = conversation
                .all_tasks()
                .flat_map(|task| task.messages())
                .collect();
            for message in messages.iter().rev() {
                if let Some(maa_api::message::Message::OrchestrationConfigSnapshot(snapshot)) =
                    &message.message
                {
                    if !snapshot.plan_id.is_empty() && !configs.contains_key(&snapshot.plan_id) {
                        if let Some(config) = snapshot
                            .config
                            .as_ref()
                            .map(OrchestrationConfig::from_proto)
                        {
                            let status =
                                OrchestrationConfigStatus::from_proto(snapshot.status.as_ref());
                            configs.insert(snapshot.plan_id.clone(), (config, status));
                        }
                    }
                }
            }
            configs
        };
        if !configs.is_empty() {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, hctx| {
                if let Some(conversation) = history.conversation_mut(&conversation_id) {
                    if conversation.set_orchestration_configs(configs) {
                        hctx.emit(BlocklistAIHistoryEvent::OrchestrationConfigUpdated {
                            conversation_id,
                            from_restore: true,
                        });
                    }
                }
            });
        }
    }

    // ── Orchestration config accessors ────────────────────────────

    /// Takes all dirty orchestration events for the given conversation,
    /// returning one event per plan that was edited.
    pub fn take_dirty_orchestration_events(
        &mut self,
        conversation_id: &AIConversationId,
    ) -> Vec<DirtyOrchestrationEvent> {
        let keys_to_remove: Vec<_> = self
            .dirty_orchestration_events
            .keys()
            .filter(|(cid, _)| cid == conversation_id)
            .cloned()
            .collect();
        keys_to_remove
            .into_iter()
            .filter_map(|key| self.dirty_orchestration_events.remove(&key))
            .collect()
    }

    /// Re-insert dirty events that were taken but not successfully sent.
    pub fn set_dirty_orchestration_events(
        &mut self,
        conversation_id: AIConversationId,
        events: Vec<DirtyOrchestrationEvent>,
    ) {
        for event in events {
            let plan_id = event.plan_id.clone();
            self.dirty_orchestration_events
                .insert((conversation_id, plan_id), event);
        }
    }

    /// Updates the per-plan orchestration config and status; called from
    /// the plan card config block on field edit / approval toggle.
    pub fn set_orchestration_config_for_plan(
        &mut self,
        conversation_id: AIConversationId,
        plan_id: String,
        config: OrchestrationConfig,
        status: OrchestrationConfigStatus,
        ctx: &mut ModelContext<Self>,
    ) {
        self.dirty_orchestration_events.insert(
            (conversation_id, plan_id.clone()),
            DirtyOrchestrationEvent {
                plan_id: plan_id.clone(),
                config: config.clone(),
                status,
            },
        );
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, hctx| {
            if let Some(conversation) = history.conversation_mut(&conversation_id) {
                conversation.set_orchestration_config_for_plan(plan_id, config, status);
            }
            hctx.emit(BlocklistAIHistoryEvent::OrchestrationConfigUpdated {
                conversation_id,
                from_restore: false,
            });
        });
    }

    /// Restore a document to a previous version, creating a new version in the process.
    /// Returns the new version number on success.
    pub fn revert_to_document_version(
        &mut self,
        id: &AIDocumentId,
        target_version: AIDocumentVersion,
        ctx: &mut ModelContext<Self>,
    ) -> Result<AIDocumentVersion, anyhow::Error> {
        // Find the target version
        let target_version_data = self
            .get_earlier_document_version(id, target_version)
            .ok_or_else(|| {
                anyhow::anyhow!("Version {} not found for document {}", target_version, id)
            })?;

        let title = target_version_data.title.clone();
        let content = target_version_data.editor.as_ref(ctx).markdown(ctx);

        // Save current state as a version and prepare for new version
        let doc = self
            .create_new_document_version(id, ctx)
            .ok_or_else(|| anyhow::anyhow!("Document {} not found", id))?;

        // Restore the document to the target version's content and title
        doc.title = title;
        doc.editor.update(ctx, |editor, editor_ctx| {
            editor.reset_with_markdown(&content, editor_ctx);
        });

        // Track which version this was restored from
        doc.restored_from = Some(target_version);

        // Mark document as Dirty so the updated plan will be attached on the next query
        doc.user_edit_status = AIDocumentUserEditStatus::Dirty;

        ctx.emit(AIDocumentModelEvent::DocumentUpdated {
            document_id: *id,
            version: doc.version,
            source: AIDocumentUpdateSource::User,
        });
        ctx.emit(AIDocumentModelEvent::DocumentUserEditStatusUpdated {
            document_id: *id,
            status: AIDocumentUserEditStatus::Dirty,
        });

        Ok(doc.version)
    }
}

impl AIDocumentEarlierVersion {
    pub fn get_content(&self, ctx: &warpui::AppContext) -> String {
        self.editor.as_ref(ctx).markdown_unescaped(ctx)
    }
}

pub enum AIDocumentModelEvent {
    /// Emitted when a document is created or updated.
    /// If the agent made an update that created a new version, this will be emitted
    /// with the new version number.
    /// If the user makes updates to the latest (current) version, this will be emitted
    /// repeatedly with the same latest (current) version.
    DocumentUpdated {
        document_id: AIDocumentId,
        version: AIDocumentVersion,
        source: AIDocumentUpdateSource,
    },
    /// When the AI Document has progressed from NotSaved -> Saving -> Saved
    DocumentSaveStatusUpdated(AIDocumentId),
    /// When the user edit status of a document changes
    DocumentUserEditStatusUpdated {
        document_id: AIDocumentId,
        status: AIDocumentUserEditStatus,
    },
    /// When streaming documents for a conversation are cleared
    StreamingDocumentsCleared(AIConversationId),
    DocumentVisibilityChanged(AIDocumentId),
}

impl Entity for AIDocumentModel {
    type Event = AIDocumentModelEvent;
}

impl SingletonEntity for AIDocumentModel {}

#[cfg(test)]
#[path = "ai_document_model_tests.rs"]
mod tests;
