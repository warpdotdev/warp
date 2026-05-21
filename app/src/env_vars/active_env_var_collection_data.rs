use crate::{
    cloud_object::{
        breadcrumbs::ContainingObject,
        model::{persistence::CloudModelEvent, view::CloudViewModel},
        CloudObject, Owner, Revision, Space,
    },
    drive::sharing::{ContentEditability, SharingAccessLevel},
    env_vars::CloudEnvVarCollection,
    server::{
        cloud_objects::update_manager::{
            ObjectOperation, OperationSuccessType, UpdateManagerEvent,
        },
        ids::{ClientId, ServerId, SyncId},
    },
    AppContext, CloudModel, UpdateManager,
};

use warpui::{Entity, ModelContext, SingletonEntity};

use super::CloudEnvVarCollectionModel;

#[derive(Default, Clone)]
pub enum ActiveEnvVarCollection {
    #[default]
    None,
    // An EnvVarCollection already stored in CloudModel, all relevant data should be queried
    // from CloudModel directly
    CommittedEnvVarCollection(SyncId),
    // An EnvVarCollection that has been created and displayed in the view, but is not yet
    // committed to CloudModel
    NewEnvVarCollection(Box<CloudEnvVarCollection>),
}

#[derive(Default, PartialEq, Debug)]
pub enum SavingStatus {
    #[default]
    Saved,
    Unsaved,
    New,
}

#[derive(Default)]
pub struct ActiveEnvVarCollectionData {
    pub saving_status: SavingStatus,
    pub active_env_var_collection: ActiveEnvVarCollection,
    pub revision_ts: Option<Revision>,
}

impl ActiveEnvVarCollectionData {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let update_manager = UpdateManager::handle(ctx);

        ctx.subscribe_to_model(&update_manager, |me, event, ctx| {
            me.handle_update_manager_event(event, ctx);
        });

        let cloud_model = CloudModel::handle(ctx);

        ctx.subscribe_to_model(&cloud_model, |me, event, ctx| {
            me.handle_cloud_model_event(event, ctx);
        });

        Self {
            ..Default::default()
        }
    }

    fn handle_cloud_model_event(&mut self, event: &CloudModelEvent, ctx: &mut ModelContext<Self>) {
        if let CloudModelEvent::ObjectMoved { type_and_id, .. } = event {
            if let Some(env_var_collection_id) = type_and_id.as_generic_string_object_id() {
                if self.is_active_env_var_collection(env_var_collection_id) {
                    ctx.emit(ActiveEnvVarCollectionDataEvent::BreadcrumbsChanged)
                }
            }
        }
    }

    fn handle_update_manager_event(
        &mut self,
        event: &UpdateManagerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let cloud_model = CloudModel::as_ref(ctx);

        let UpdateManagerEvent::ObjectOperationComplete { result } = event else {
            return;
        };

        match (&result.operation, &result.success_type) {
            (ObjectOperation::Create { .. }, OperationSuccessType::Success) => {
                if let Some(current_id) = self.id() {
                    if current_id.into_client() == result.client_id {
                        let server_id = result.server_id.expect("Expect server id on success");
                        let env_var_collection_id = SyncId::ServerId(server_id);

                        if let Some(env_var_collection) =
                            cloud_model.get_env_var_collection(&env_var_collection_id)
                        {
                            self.saving_status = SavingStatus::Saved;
                            self.active_env_var_collection =
                                ActiveEnvVarCollection::CommittedEnvVarCollection(
                                    env_var_collection_id,
                                );
                            self.revision_ts
                                .clone_from(&env_var_collection.metadata.revision);
                            ctx.emit(ActiveEnvVarCollectionDataEvent::CreatedOnServer(server_id));
                            ctx.notify();
                        }
                    }
                }
            }
            (ObjectOperation::Update, OperationSuccessType::Success) => {
                if let Some(current_id) = self.id() {
                    // If we match on a non-None client id or a non-None server id then
                    // update the data
                    if (current_id.into_client().is_some()
                        && current_id.into_client() == result.client_id)
                        || (current_id.into_server().is_some()
                            && current_id.into_server() == result.server_id)
                    {
                        let server_id = result.server_id.expect("Expect server id on success");
                        let env_var_collection_id = SyncId::ServerId(server_id);
                        if let Some(env_var_collection) =
                            cloud_model.get_env_var_collection(&env_var_collection_id)
                        {
                            self.saving_status = SavingStatus::Saved;
                            self.active_env_var_collection =
                                ActiveEnvVarCollection::CommittedEnvVarCollection(
                                    env_var_collection_id,
                                );

                            self.revision_ts
                                .clone_from(&env_var_collection.metadata.revision);

                            ctx.notify();
                        }
                    }
                }
            }
            (ObjectOperation::Trash, OperationSuccessType::Success)
            | (ObjectOperation::Untrash, OperationSuccessType::Success) => {
                let server_id = result.server_id.expect("Expect server id on success");
                if let Some(current_id) = self.id() {
                    if current_id.into_client() == result.client_id
                        && cloud_model
                            .get_env_var_collection(&SyncId::ServerId(server_id))
                            .is_some()
                    {
                        ctx.emit(ActiveEnvVarCollectionDataEvent::TrashStatusChanged);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn reset(&mut self) {
        self.active_env_var_collection = ActiveEnvVarCollection::None;
    }

    pub fn open_new(
        &mut self,
        owner: Owner,
        initial_folder_id: Option<SyncId>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.reset();

        let new_id = ClientId::default();

        // Set the active env var collection to be an uncommitted collection
        self.active_env_var_collection = ActiveEnvVarCollection::NewEnvVarCollection(Box::new(
            CloudEnvVarCollection::new_local(
                CloudEnvVarCollectionModel::default(),
                owner,
                initial_folder_id,
                new_id,
            ),
        ));

        ctx.emit(ActiveEnvVarCollectionDataEvent::BreadcrumbsChanged);
        ctx.notify();
    }

    pub fn open_existing(&mut self, env_var_collection_id: SyncId, ctx: &mut ModelContext<Self>) {
        self.reset();
        self.saving_status = SavingStatus::Saved;
        self.active_env_var_collection =
            ActiveEnvVarCollection::CommittedEnvVarCollection(env_var_collection_id);

        ctx.emit(ActiveEnvVarCollectionDataEvent::BreadcrumbsChanged);
        ctx.notify();
    }

    pub fn id(&self) -> Option<SyncId> {
        match &self.active_env_var_collection {
            ActiveEnvVarCollection::None => None,
            ActiveEnvVarCollection::CommittedEnvVarCollection(id) => Some(*id),
            ActiveEnvVarCollection::NewEnvVarCollection(env_var_collection) => {
                Some(env_var_collection.id)
            }
        }
    }

    /// The current user's access level on this env var collection.
    pub fn access_level(&self, app: &AppContext) -> SharingAccessLevel {
        match &self.active_env_var_collection {
            ActiveEnvVarCollection::CommittedEnvVarCollection(sync_id) => {
                CloudViewModel::as_ref(app).access_level(&sync_id.uid(), app)
            }
            ActiveEnvVarCollection::None | ActiveEnvVarCollection::NewEnvVarCollection(_) => {
                SharingAccessLevel::Full
            }
        }
    }

    pub fn editability(&self, app: &AppContext) -> ContentEditability {
        match &self.active_env_var_collection {
            ActiveEnvVarCollection::CommittedEnvVarCollection(sync_id) => {
                CloudViewModel::as_ref(app).object_editability(&sync_id.uid(), app)
            }
            ActiveEnvVarCollection::None | ActiveEnvVarCollection::NewEnvVarCollection(_) => {
                ContentEditability::Editable
            }
        }
    }

    /// The space that this env var collection is in.
    pub fn space(&self, app: &AppContext) -> Option<Space> {
        match &self.active_env_var_collection {
            ActiveEnvVarCollection::None => None,
            ActiveEnvVarCollection::CommittedEnvVarCollection(sync_id) => {
                CloudViewModel::as_ref(app).object_space(&sync_id.uid(), app)
            }
            ActiveEnvVarCollection::NewEnvVarCollection(env_var_collection) => {
                Some(env_var_collection.space(app))
            }
        }
    }

    pub fn active_env_var_collection(&self) -> ActiveEnvVarCollection {
        self.active_env_var_collection.clone()
    }

    /// Whether or not the EVC has been synced to the server.
    pub fn is_on_server(&self) -> bool {
        matches!(
            &self.active_env_var_collection,
            ActiveEnvVarCollection::CommittedEnvVarCollection(SyncId::ServerId(_))
        )
    }

    pub fn is_active_env_var_collection(&self, env_var_collection_id: SyncId) -> bool {
        self.id() == Some(env_var_collection_id)
    }

    pub fn breadcrumbs(&self, ctx: &AppContext) -> Option<Vec<ContainingObject>> {
        let cloud_env_var_collection = match &self.active_env_var_collection {
            ActiveEnvVarCollection::None => None,
            ActiveEnvVarCollection::CommittedEnvVarCollection(id) => {
                CloudModel::as_ref(ctx).get_env_var_collection(id)
            }
            ActiveEnvVarCollection::NewEnvVarCollection(env_var_collection) => {
                Some(env_var_collection.as_ref())
            }
        };

        cloud_env_var_collection
            .map(|env_var_collection| env_var_collection.containing_objects_path(ctx))
    }

    pub fn trash_status(&self, ctx: &AppContext) -> TrashStatus {
        match &self.active_env_var_collection {
            ActiveEnvVarCollection::None | ActiveEnvVarCollection::NewEnvVarCollection(_) => {
                TrashStatus::Active
            }
            ActiveEnvVarCollection::CommittedEnvVarCollection(id) => {
                let cloud_model = CloudModel::as_ref(ctx);
                match cloud_model.get_env_var_collection(id) {
                    Some(env_var_collection) => {
                        if env_var_collection.is_trashed(cloud_model) {
                            TrashStatus::Trashed
                        } else {
                            TrashStatus::Active
                        }
                    }
                    None => TrashStatus::Deleted,
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrashStatus {
    Active,
    Trashed,
    Deleted,
}

pub enum ActiveEnvVarCollectionDataEvent {
    /// The EVC's breadcrumbs were updated.
    BreadcrumbsChanged,
    /// The EVC was synced to the server for the first time.
    CreatedOnServer(ServerId),
    /// The EVC was trashed or untrashed
    /// (used for refreshing the pane overflow items)
    TrashStatusChanged,
}

impl Entity for ActiveEnvVarCollectionData {
    type Event = ActiveEnvVarCollectionDataEvent;
}
