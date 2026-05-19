pub mod active_notebook_data;
mod context_menu;
pub mod editor;
pub mod file;
pub mod link;
pub mod manager;
pub mod notebook;
mod styles;
pub mod telemetry;

use std::sync::Arc;

use async_trait::async_trait;

use anyhow::Result;
pub use cloud_object_models::{CloudNotebook, CloudNotebookModel, NotebookId, SerializedNotebook};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use warpui::AppContext;

use crate::server::cloud_objects::update_manager::InitiatedBy;
use crate::{
    appearance::Appearance,
    cloud_object::{
        CloudModelType, CloudObjectEventEntrypoint, CloudObjectUpsertParams,
        CreateCloudObjectResult, CreateObjectRequest, GenericServerObject, ObjectType, Owner,
        Revision, UpdateCloudObjectResult,
    },
    drive::{
        items::{notebook::WarpDriveNotebook, WarpDriveItem},
        CloudObjectTypeAndId,
    },
    persistence::ModelEvent,
    server::{
        ids::{ServerId, SyncId},
        server_api::object::ObjectClient,
        sync_queue::{QueueItem, SerializedModel},
    },
};

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl CloudModelType for CloudNotebookModel {
    type CloudObjectType = CloudNotebook;
    type IdType = NotebookId;

    fn model_type_name(&self) -> &'static str {
        if self.ai_document_id.is_some() {
            "Plan"
        } else {
            "Notebook"
        }
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Notebook
    }

    fn cloud_object_type_and_id(&self, id: SyncId) -> CloudObjectTypeAndId {
        CloudObjectTypeAndId::Notebook(id)
    }

    fn display_name(&self) -> String {
        self.title.clone()
    }

    fn set_display_name(&mut self, name: &str) {
        name.clone_into(&mut self.title);
    }

    fn upsert_event(params: CloudObjectUpsertParams<Self>) -> ModelEvent {
        ModelEvent::UpsertNotebook {
            notebook: CloudNotebook::from(params),
        }
    }

    fn bulk_upsert_event(objects: Vec<CloudObjectUpsertParams<Self>>) -> ModelEvent {
        ModelEvent::UpsertNotebooks(objects.into_iter().map(CloudNotebook::from).collect())
    }

    fn create_object_queue_item(
        &self,
        notebook: &CloudNotebook,
        entrypoint: CloudObjectEventEntrypoint,
        initiated_by: InitiatedBy,
    ) -> Option<QueueItem> {
        if let SyncId::ClientId(client_id) = notebook.id {
            let title = Some(notebook.model().display_name())
                .filter(|name| !name.is_empty())
                .map(Arc::new);

            let serialized_model = Some(Arc::new(notebook.model().serialized()));

            return Some(QueueItem::CreateObject {
                object_type: self.object_type(),
                owner: notebook.permissions.owner,
                id: client_id,
                title,
                serialized_model,
                initial_folder_id: notebook.metadata.folder_id,
                entrypoint,
                initiated_by,
            });
        }
        None
    }

    fn update_object_queue_item(
        &self,
        revision_ts: Option<Revision>,
        notebook: &CloudNotebook,
    ) -> QueueItem {
        QueueItem::UpdateNotebook {
            // Note that this is intentionally a deep clone of the model because we are grabbing
            // a snapshot to update at a moment in time.
            model: notebook.model().clone().into(),
            id: notebook.id,
            revision: revision_ts.or_else(|| notebook.metadata.revision.clone()),
        }
    }

    fn should_update_after_server_conflict(&self) -> bool {
        true
    }

    fn serialized(&self) -> SerializedModel {
        let serialized = SerializedNotebook {
            data: self.data.clone(),
            ai_document_id: self.ai_document_id.as_ref().map(|id| id.to_string()),
            conversation_id: self.conversation_id.clone(),
        };
        let json = serde_json::to_string(&serialized).expect("Failed to serialize notebook");
        SerializedModel::new(json)
    }

    async fn send_create_request(
        object_client: Arc<dyn ObjectClient>,
        request: CreateObjectRequest,
    ) -> Result<CreateCloudObjectResult> {
        object_client.create_notebook(request).await
    }

    async fn send_update_request(
        &self,
        object_client: Arc<dyn ObjectClient>,
        server_id: ServerId,
        revision: Option<Revision>,
    ) -> Result<UpdateCloudObjectResult<GenericServerObject<NotebookId, Self>>> {
        object_client
            .update_notebook(
                server_id.into(),
                Some(self.title.clone()),
                Some(self.data.clone().into()),
                revision,
            )
            .await
    }

    fn renders_in_warp_drive(&self) -> bool {
        true
    }

    fn can_export(&self) -> bool {
        true
    }

    fn to_warp_drive_item(
        &self,
        id: SyncId,
        _appearance: &Appearance,
        notebook: &CloudNotebook,
    ) -> Option<Box<dyn WarpDriveItem>> {
        Some(Box::new(WarpDriveNotebook::new(
            self.cloud_object_type_and_id(id),
            notebook.clone(),
            notebook.model().ai_document_id.is_some(),
        )))
    }
}

/// A notebook location. Mainly, this lets us distinguish between cloud and file-based notebooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum NotebookLocation {
    /// A cloud notebook in the user's personal space.
    PersonalCloud,
    /// A cloud notebook in a team space.
    Team,
    /// A notebook backed by a local file.
    LocalFile,
    /// A notebook backed by a remote file.
    RemoteFile,
}

impl From<Owner> for NotebookLocation {
    fn from(owner: Owner) -> Self {
        // TODO(ben): Account for shared objects in notebook telemetry.
        match owner {
            Owner::User { .. } => NotebookLocation::PersonalCloud,
            Owner::Team { .. } => NotebookLocation::Team,
        }
    }
}

/// Initialize notebooks-related keybindings.
pub fn init(app: &mut AppContext) {
    self::notebook::init(app);
    self::file::init(app);
    self::editor::view::init(app);
}

/// Post process a notebook's content read from an external system. This cleans up extra
/// whitespace, and, in the future, may filter out unsupported syntax extensions.
///
/// See CLD-944.
pub fn post_process_notebook(data: &str) -> String {
    // TODO(kevin): We should not strip out newlines in the code block.
    data.lines().filter(|line| !line.is_empty()).join("\n")
}

/// Translate a notebook's Markdown content into an external Markdown format.
///
/// This:
/// * Normalizes code block languages
/// * Includes extra context for embedded objects.
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub fn export_notebook(data: &str, ctx: &AppContext) -> anyhow::Result<String> {
    use warp_editor::content::{buffer::Buffer, markdown::MarkdownStyle};

    // Parse the Markdown directly rather than using [`Buffer::from_markdown`] so that we can
    // report errors to the exporter.
    let parsed = markdown_parser::parse_markdown(data)?;
    Ok(Buffer::export_to_markdown(
        parsed,
        Some(editor::notebook_embedded_item_conversion),
        MarkdownStyle::Export {
            app_context: Some(ctx),
            should_not_escape_markdown_punctuation: false,
        },
    ))
}
