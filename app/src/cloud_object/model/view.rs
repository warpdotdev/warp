use std::{cell::RefCell, collections::HashMap};

use chrono::{Duration, Utc};
use warp_graphql::scalars::time::ServerTimestamp;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::{
    auth::{AuthStateProvider, UserUid},
    cloud_object::{CloudObject, CloudObjectLocation, Space},
    drive::{
        folders::CloudFolder,
        sharing::{ContentEditability, SharingAccessLevel},
    },
    safe_info,
    server::{
        cloud_objects::update_manager::{
            ObjectOperation, OperationSuccessType, UpdateManager, UpdateManagerEvent,
        },
        ids::{ObjectUid, SyncId},
    },
    workspaces::user_profiles::UserProfiles,
};

use super::persistence::{CloudModel, CloudModelEvent};

pub const EDITOR_TIMEOUT_DURATION_MINUTES: i64 = 15;

#[derive(Default, Clone, Debug, PartialEq)]
pub enum EditorState {
    #[default]
    None,
    CurrentUser,
    OtherUserActive,
    OtherUserIdle,
}

/// Stores information about the current editor of
/// a particular notebook, for display purposes.
/// For now, this just includes the state and
/// an email, but will eventually hold more information
/// about the user.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct Editor {
    pub state: EditorState,
    pub email: Option<String>,
}

impl Editor {
    pub fn no_editor() -> Self {
        Self {
            state: EditorState::None,
            email: None,
        }
    }
}

/// Singleton model for storing and querying the data and logic logic needed by various view, based on the information
/// stored in [CloudModel]. As a general, rule, any new API that requires logic beyond just retrieving the raw value
/// in [CloudModel], should be stored here. This includes logic such as object trashed status, the object current editor,
/// and object location.
///
/// Any API added to this model should be unit tested in model_test.rs
pub struct CloudViewModel {
    folder_timestamp_cache: FolderTimestampCache,
}

type FolderTimestampCache = RefCell<HashMap<SyncId, ServerTimestamp>>;

pub enum CloudViewModelEvent {
    /// A model change has invalidated object sort timestamps.
    SortTimestampsChanged,
}

impl CloudViewModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&CloudModel::handle(ctx), Self::handle_cloud_model_event);
        ctx.subscribe_to_model(
            &UpdateManager::handle(ctx),
            Self::handle_update_manager_event,
        );
        Self {
            folder_timestamp_cache: Default::default(),
        }
    }

    #[cfg(test)]
    pub fn mock(ctx: &mut ModelContext<Self>) -> Self {
        Self::new(ctx)
    }

    /// Returns the current editor of the object based on what current exists in CloudModel. If the current editor
    /// matches the logged in user's email, we assume that that user is the current editor.
    /// If the current editor hasn't made an edit in the past 15 minutes, they are considered idle and
    /// we instead just return Editor::OtherUserIdle. This is to prevent introducing friction into the baton grabbing process
    /// when it's not needed. For more info see:
    /// https://docs.google.com/document/d/1KgDFLApPg1uDVP-vOwhZzL1kRIviS8mMECIZg2VCKLY/edit
    pub fn object_current_editor(&self, uid: &ObjectUid, ctx: &AppContext) -> Option<Editor> {
        let cloud_model = CloudModel::as_ref(ctx);
        let object = cloud_model.get_by_uid(uid)?;

        match &object.metadata().current_editor_uid {
            Some(uid) => {
                let auth_state = AuthStateProvider::as_ref(ctx).get();
                let user_uid = auth_state.user_id();

                // If the logged in user matches the current UID, then the editor is the current
                // user.
                if user_uid.is_some_and(|user_uid| user_uid.as_string() == uid.clone()) {
                    return Some(Editor {
                        state: EditorState::CurrentUser,
                        email: auth_state.user_email().clone(),
                    });
                }

                let editor_uid = UserUid::new(uid);
                let editor_email = UserProfiles::as_ref(ctx)
                    .profile_for_uid(editor_uid)
                    .map(|profile| profile.email.clone());

                match &object.metadata().revision {
                    Some(revision) => {
                        let time_since_last_edit = Utc::now() - revision.utc();
                        let time_since_last_metadata_change = Utc::now()
                            - object
                                .metadata()
                                .metadata_last_updated_ts
                                .unwrap_or(Utc::now().into())
                                .utc();
                        if time_since_last_edit > Duration::minutes(EDITOR_TIMEOUT_DURATION_MINUTES)
                            && time_since_last_metadata_change
                                > Duration::minutes(EDITOR_TIMEOUT_DURATION_MINUTES)
                        {
                            safe_info!(
                                safe: ("Current editor idle, eagerly grabbing edit access for notebook"),
                                full: ("Current editor idle, eagerly grabbing edit access for notebook with editor: {}", uid.clone())
                            );
                            Some(Editor {
                                state: EditorState::OtherUserIdle,
                                email: editor_email,
                            })
                        } else {
                            Some(Editor {
                                state: EditorState::OtherUserActive,
                                email: editor_email,
                            })
                        }
                    }
                    None => Some(Editor {
                        state: EditorState::OtherUserActive,
                        email: editor_email,
                    }),
                }
            }
            _ => Some(Editor::no_editor()),
        }
    }

    /// Get the [`Space`] that contains an object.
    pub fn object_space(&self, id: &ObjectUid, app: &AppContext) -> Option<Space> {
        CloudModel::as_ref(app)
            .get_by_uid(id)
            .map(|object| object.space(app))
    }

    /// Get the current user's access level on a Warp Drive object.
    ///
    /// This is based on the client's current view of the object permissions, which may be stale. The
    /// server is the source of truth for all permission data, and it may reject a request that the
    /// client expects is allowed.
    pub fn access_level(&self, object_uid: &ObjectUid, app: &AppContext) -> SharingAccessLevel {
        match CloudModel::as_ref(app).get_by_uid(object_uid) {
            Some(object) => Self::object_access_level(object, app),
            None => SharingAccessLevel::View,
        }
    }

    fn object_access_level(object: &dyn CloudObject, app: &AppContext) -> SharingAccessLevel {
        match object.space(app) {
            // For now, users have full access to all objects in their own drives. We may introduce
            // drive-level ACLs in the future.
            Space::Personal | Space::Team { .. } => SharingAccessLevel::Full,
            Space::Shared => {
                let mut access_level = SharingAccessLevel::View;

                // Check the default link-based access (if set, this is *at least* View).
                if let Some(link_settings) = &object.permissions().anyone_with_link {
                    access_level = link_settings.access_level;
                }

                let user_uid = AuthStateProvider::as_ref(app).get().user_id();
                if let Some(user_uid) = user_uid {
                    for guest in object.permissions().guests.iter() {
                        if guest.subject.is_user(user_uid) {
                            access_level = access_level.max(guest.access_level);
                        }
                    }
                }

                // If the user created an object in a shared space, they will be treated as a guest and not the owner.
                // The guest permissions aren't fetched until the object is re-fetched, and this fixes this behavior
                // by forcing edit access if they created the object.
                if let (Some(creator_uid), Some(user_uid)) =
                    (object.metadata().creator_uid.clone(), user_uid)
                {
                    if creator_uid == user_uid.as_string() {
                        access_level = access_level.max(SharingAccessLevel::Edit);
                    }
                }

                access_level
            }
        }
    }

    /// Get the current user's editability state for a Warp Drive object.
    pub fn object_editability(
        &self,
        object_uid: &ObjectUid,
        app: &AppContext,
    ) -> ContentEditability {
        match CloudModel::as_ref(app).get_by_uid(object_uid) {
            Some(object) => {
                let access_level = Self::object_access_level(object, app);
                if access_level < SharingAccessLevel::Edit {
                    ContentEditability::ReadOnly
                } else if AuthStateProvider::as_ref(app)
                    .get()
                    .is_anonymous_or_logged_out()
                {
                    // The object is editable, but the user is not logged in.
                    if object.space(app) == Space::Personal {
                        ContentEditability::Editable
                    } else {
                        ContentEditability::RequiresLogin
                    }
                } else {
                    ContentEditability::Editable
                }
            }
            // Assume objects not yet in CloudModel are new, and therefore editable.
            None => ContentEditability::Editable,
        }
    }

    /// Get the timestamp to sort `object` according to `timestamp_kind`.
    pub fn object_sorting_timestamp(
        &self,
        object: &dyn CloudObject,
        timestamp_kind: UpdateTimestamp,
        app: &AppContext,
    ) -> Option<ServerTimestamp> {
        match timestamp_kind {
            // When sorting in the trash, we only ever consider the object's own trashed timestamp.
            // For trashed folders, their indirectly-trashed children will not have a trashed_ts,
            // so there's no need to recurse.
            UpdateTimestamp::Trashed => object.metadata().trashed_ts,
            // When sorting in the main index, we consider all of the children of a folder. This
            // can be expensive, so it's cached.
            UpdateTimestamp::Revision => {
                self.sorting_timestamp_rec(object, CloudModel::as_ref(app), app)
            }
        }
    }

    /// Calculate the sorting timestamp for `object`:
    /// * For a folder, this is the max of the folder's timestamp and all of its children's timestamps
    ///   (recursively, for sub-folders).
    /// * For other objects, this is the object's own timestamp.
    fn sorting_timestamp_rec(
        &self,
        object: &dyn CloudObject,
        cloud_model: &CloudModel,
        app: &AppContext,
    ) -> Option<ServerTimestamp> {
        let folder: Option<&CloudFolder> = object.into();
        match folder {
            // For non-folder objects, always use the object's own timestamp.
            None => object.metadata().revision.clone().map(Into::into),
            Some(folder) => self
                .folder_timestamp_cache
                // Skip the cache if it's already mutably borrowed. This should not happen in practice,
                // because the UI framework is single-threaded.
                .try_borrow()
                .ok()
                .and_then(|cache| cache.get(&folder.id).cloned())
                .or_else(|| {
                    let max_child_timestamp = cloud_model
                        .active_cloud_objects_in_location_without_descendents(
                            CloudObjectLocation::Folder(folder.id),
                            app,
                        )
                        // TODO(ben): This check won't be needed soon.
                        .filter(|child| child.permissions().owner == folder.permissions().owner)
                        .filter_map(|child| self.sorting_timestamp_rec(child, cloud_model, app))
                        .max();
                    // The `Ord` implementation of `Option` always considers `None` less than
                    // `Some`.
                    let folder_timestamp = folder.metadata().revision.clone().map(Into::into);
                    let timestamp = max_child_timestamp.max(folder_timestamp);

                    if let Some(timestamp) = timestamp {
                        if let Ok(mut cache) = self.folder_timestamp_cache.try_borrow_mut() {
                            cache.insert(folder.id, timestamp);
                        }
                    }

                    timestamp
                }),
        }
    }

    fn handle_cloud_model_event(&mut self, event: &CloudModelEvent, ctx: &mut ModelContext<Self>) {
        match event {
            CloudModelEvent::ObjectUpdated { type_and_id, .. }
            | CloudModelEvent::ObjectTrashed { type_and_id, .. }
            | CloudModelEvent::ObjectUntrashed { type_and_id, .. }
            | CloudModelEvent::ObjectPermissionsUpdated { type_and_id, .. } => {
                // If an object is updated, we need to recompute the timestamps of its parents.
                if self.invalidate_object_timestamps(&type_and_id.uid(), CloudModel::as_ref(ctx)) {
                    ctx.emit(CloudViewModelEvent::SortTimestampsChanged);
                }
            }
            CloudModelEvent::ObjectMoved {
                from_folder,
                to_folder,
                ..
            } => {
                // Both the old parent and the new parent need to be invalidated, since this object
                // could affect the sort timestamp of both. Even if the moved object were a folder,
                // its own sort timestamp isn't affected.
                let cloud_model = CloudModel::as_ref(ctx);
                let old_parent_changed = from_folder.is_some_and(|folder_id| {
                    self.invalidate_folder_timestamps(&folder_id, cloud_model)
                });
                let new_parent_changed = to_folder.is_some_and(|folder_id| {
                    self.invalidate_folder_timestamps(&folder_id, cloud_model)
                });
                if old_parent_changed || new_parent_changed {
                    ctx.emit(CloudViewModelEvent::SortTimestampsChanged);
                }
            }
            CloudModelEvent::ObjectCreated { type_and_id } => {
                // There are three cases for an ObjectCreated event:
                // 1. We created a new object locally (in which case type_and_id is a client ID)
                // 2. We were notified about a new object from the server.
                // 3. A locally-created object was saved to the server, so we now have a server ID
                //    for it.
                // Because we sort on server timestamps, only the second or third cases can affect
                // sorting.
                if type_and_id.has_server_id()
                    && self
                        .invalidate_object_timestamps(&type_and_id.uid(), CloudModel::as_ref(ctx))
                {
                    ctx.emit(CloudViewModelEvent::SortTimestampsChanged);
                }
            }
            CloudModelEvent::ObjectDeleted { folder_id, .. } => {
                if let Some(folder_id) = folder_id {
                    if self.invalidate_folder_timestamps(folder_id, CloudModel::as_ref(ctx)) {
                        ctx.emit(CloudViewModelEvent::SortTimestampsChanged);
                    }
                }
            }
            CloudModelEvent::NotebookEditorChangedFromServer { .. }
            | CloudModelEvent::ObjectForceExpanded { .. }
            | CloudModelEvent::ObjectSynced { .. }
            | CloudModelEvent::InitialLoadCompleted => (),
        }
    }

    fn handle_update_manager_event(
        &mut self,
        event: &UpdateManagerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let UpdateManagerEvent::ObjectOperationComplete { result } = event else {
            return;
        };

        if result.success_type != OperationSuccessType::Success {
            return;
        }

        let cloud_model = CloudModel::as_ref(ctx);
        if let ObjectOperation::Create { .. } = result.operation {
            // If a folder was created, remove the cache entry tied to its client ID.
            // TODO @ianhodge: Update the way we do this check once we remove the generic
            let server_id = &result.server_id.expect("Expect server id on success");
            if cloud_model.get_folder_by_uid(&server_id.uid()).is_some() {
                if let Some(client_id) = result.client_id {
                    let sync_id = SyncId::ClientId(client_id);
                    self.folder_timestamp_cache.borrow_mut().remove(&sync_id);
                }
            }

            // For any new object, we need to recalculate its ancestors' timestamp with their
            // new child.
            if let Some(parent_id) = cloud_model
                .get_by_uid(&server_id.uid())
                .and_then(|object| object.metadata().folder_id)
            {
                if self.invalidate_folder_timestamps(&parent_id, cloud_model) {
                    ctx.emit(CloudViewModelEvent::SortTimestampsChanged);
                }
            }
        }
    }

    /// Invalidate all cached timestamps for the object with the given ID, and its parents.
    fn invalidate_object_timestamps(&mut self, uid: &ObjectUid, cloud_model: &CloudModel) -> bool {
        let Some(object) = cloud_model.get_by_uid(uid) else {
            return false;
        };
        let folder: Option<&CloudFolder> = object.into();
        match folder {
            Some(folder) => self.invalidate_folder_timestamps(&folder.id, cloud_model),
            None => {
                if let Some(parent_id) = object.metadata().folder_id {
                    self.invalidate_folder_timestamps(&parent_id, cloud_model)
                } else {
                    false
                }
            }
        }
    }

    /// Invalidate all cached timestamps for the given folder and its parents.
    fn invalidate_folder_timestamps(
        &mut self,
        folder_id: &SyncId,
        cloud_model: &CloudModel,
    ) -> bool {
        let had_revision_ts = self
            .folder_timestamp_cache
            .borrow_mut()
            .remove(folder_id)
            .is_some();

        let had_parent_ts = cloud_model
            .get_folder(folder_id)
            .and_then(|folder| folder.metadata().folder_id.as_ref())
            .is_some_and(|parent| self.invalidate_folder_timestamps(parent, cloud_model));
        had_revision_ts || had_parent_ts
    }
}

impl Entity for CloudViewModel {
    type Event = CloudViewModelEvent;
}

/// Mark CloudViewModel as global application state.
impl SingletonEntity for CloudViewModel {}

/// The timestamp to use when sorting objects by their last updated time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpdateTimestamp {
    /// Sort objects by their revision timestamp, when they were last edited.
    #[default]
    Revision,
    /// Sort objects by their trashed timestamp.
    Trashed,
}
