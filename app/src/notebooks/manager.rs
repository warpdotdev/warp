use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::Arc;

use futures_util::stream::AbortHandle;
use markdown_parser::markdown_parser::parse_markdown_to_raw_text;
use warpui::r#async::SpawnedFutureHandle;
use warpui::{
    Entity, EntityId, ModelContext, ModelHandle, SingletonEntity, WeakViewHandle, WindowId,
};

use super::notebook::NotebookView;
use super::CloudNotebook;
use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::cloud_object::Owner;
use crate::drive::OpenWarpDriveObjectSettings;
use crate::pane_group::{NotebookPane, PaneContent};
use crate::server::cloud_objects::update_manager::{
    ObjectOperation, OperationSuccessType, UpdateManager, UpdateManagerEvent,
};
use crate::server::ids::SyncId;
use crate::workspace::PaneViewLocator;
use crate::{safe_debug, safe_warn};

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;

/// A singleton model tracking open notebooks.
///
/// This is tightly tied to the [workspace](crate::workspace::Workspace) and
/// [pane group](crate::pane_group::PaneGroup) views, as they contain all open notebook panes.
///
/// The overall flow is:
/// 1. A `Workspace` is asked to open a notebook (from the Warp Drive index, universal search, etc.).
/// 2. It checks the `NotebookManager` to see if the notebook is already open.
/// 3. If it is, the existing notebook pane is focused (this may be in another window).
/// 4. If not, the `Workspace` uses the `NotebookManager` to create a new notebook pane and
///    attaches it to the active tab.
/// 5. When the new pane is attached to a pane group, it registers itself with the `NotebookManager`.
///    This is because we need the pane group's ID in order to re-focus the pane.
/// 6. When the pane is closed, it de-registers itself from the `NotebookManager`.
///
/// During session restoration, notebook panes are created and attached by the `PaneGroup`.
///
/// NotebookManager also manages a cache of the raw, unformatted text of notebooks
/// which is needed for notebook search.
pub struct NotebookManager {
    panes_by_hashed_id: HashMap<String, NotebookPaneData>,
    // Cache
    raw_text_by_hashed_id: HashMap<String, NotebookRawTextStatus>,
}

#[derive(Debug)]
pub enum NotebookRawTextStatus {
    NotParsed,
    ParseInFlight(AbortHandle),
    // We store this as an arc so it can be used in fuzzy searches
    // without cloning the notebook's entire parsed contents.
    Parsed(Arc<str>),
    ParseError,
}

/// Source for a new notebook pane.
#[derive(Debug, Clone)]
pub enum NotebookSource {
    Existing(SyncId),
    New {
        title: Option<String>,
        owner: Owner,
        initial_folder_id: Option<SyncId>,
    },
}

impl NotebookManager {
    /// Create a new [`NotebookManager`] singleton.
    pub fn new(cached_notebooks: Vec<CloudNotebook>, ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(
            &UpdateManager::handle(ctx),
            Self::handle_update_manager_event,
        );

        ctx.subscribe_to_model(&CloudModel::handle(ctx), Self::handle_cloud_model_event);

        let mut raw_text_by_hashed_id: HashMap<String, NotebookRawTextStatus> = HashMap::new();
        // Parse all the cached notebook raw text

        cached_notebooks.into_iter().for_each(|notebook| {
            let hashed_id = notebook.id.uid();
            let handle = Self::spawn_raw_text_parse_for_notebook(notebook, ctx);
            raw_text_by_hashed_id.insert(
                hashed_id,
                NotebookRawTextStatus::ParseInFlight(handle.abort_handle()),
            );
        });

        Self {
            panes_by_hashed_id: HashMap::new(),
            raw_text_by_hashed_id,
        }
    }

    fn spawn_raw_text_parse_for_notebook(
        notebook: CloudNotebook,
        ctx: &mut ModelContext<Self>,
    ) -> SpawnedFutureHandle {
        let hashed_id = notebook.id.uid();
        ctx.spawn(
            async move { parse_markdown_to_raw_text(&notebook.model().data) },
            move |manager, response, _ctx| match response {
                Ok(parsed_text) => {
                    manager.raw_text_by_hashed_id.insert(
                        hashed_id,
                        NotebookRawTextStatus::Parsed(Arc::from(parsed_text)),
                    );
                }
                Err(err) => {
                    manager
                        .raw_text_by_hashed_id
                        .insert(hashed_id, NotebookRawTextStatus::ParseError);
                    log::error!("Cached Notebook raw text failed to parse: {err}.");
                }
            },
        )
    }

    /// Create a mock [`NotebookManager`] for use in tests.
    #[cfg(test)]
    pub fn mock(ctx: &mut ModelContext<Self>) -> Self {
        Self::new(Vec::new(), ctx)
    }

    /// If the notebook is already open in a pane, finds the location of that pane.
    pub fn find_pane(&self, source: &NotebookSource) -> Option<(WindowId, PaneViewLocator)> {
        match source {
            NotebookSource::Existing(notebook_id) => {
                let pane_data = self.panes_by_hashed_id.get(&notebook_id.uid())?;
                Some((pane_data.window_id, pane_data.locator))
            }
            NotebookSource::New { .. } => None,
        }
    }

    fn handle_cloud_model_event(
        &mut self,
        _: ModelHandle<CloudModel>,
        event: &CloudModelEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if let CloudModelEvent::ObjectUpdated { type_and_id, .. } = event {
            if let Some(notebook_id) = type_and_id.as_notebook_id() {
                self.update_raw_text_for_notebook(notebook_id, ctx);
            }
        }
    }

    /// Returns the raw text of a given notebook id - if it exists in the cache.
    pub fn notebook_raw_text(&self, notebook_id: SyncId) -> Option<&str> {
        match self
            .raw_text_by_hashed_id
            .get(&notebook_id.uid())
            .unwrap_or(&NotebookRawTextStatus::NotParsed)
        {
            NotebookRawTextStatus::Parsed(text) => Some(text),
            _ => None,
        }
    }

    /// Returns a shared handle to the parsed raw text.
    pub fn notebook_raw_text_shared(&self, notebook_id: SyncId) -> Option<Arc<str>> {
        match self
            .raw_text_by_hashed_id
            .get(&notebook_id.uid())
            .unwrap_or(&NotebookRawTextStatus::NotParsed)
        {
            NotebookRawTextStatus::Parsed(text) => Some(text.clone()),
            _ => None,
        }
    }

    /// Unconditionally create a new notebook pane.
    pub fn create_pane(
        &mut self,
        source: &NotebookSource,
        settings: &OpenWarpDriveObjectSettings,
        window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) -> NotebookPane {
        let view = ctx.add_typed_action_view(window_id, NotebookView::new);

        match source {
            NotebookSource::Existing(notebook_id) => {
                let notebook = CloudModel::as_ref(ctx).get_notebook(notebook_id).cloned();
                if let Some(notebook) = notebook {
                    view.update(ctx, |view, ctx| view.load(notebook, settings, ctx));
                } else {
                    // If the notebook doesn't exist yet, try waiting for initial load and check again
                    view.update(ctx, |view, ctx| {
                        view.wait_for_initial_load_then_load(*notebook_id, settings, window_id, ctx)
                    });
                }
            }
            NotebookSource::New {
                title,
                owner,
                initial_folder_id,
            } => view.update(ctx, |view, ctx| {
                view.open_new_notebook(title.clone(), *owner, *initial_folder_id, ctx);
            }),
        }

        NotebookPane::new(view, ctx)
    }

    /// Register an open notebook pane once it's bound to a pane group.
    pub fn register_pane(
        &mut self,
        pane: &NotebookPane,
        pane_group_id: EntityId,
        window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(notebook_id) = pane.notebook_view(ctx).as_ref(ctx).notebook_id(ctx) else {
            log::warn!("Notebook pane has no notebook ID");
            return;
        };

        let entry = self.panes_by_hashed_id.entry(notebook_id.uid());
        if let Entry::Vacant(entry) = entry {
            entry.insert(NotebookPaneData {
                notebook_id,
                window_id,
                locator: PaneViewLocator {
                    pane_group_id,
                    pane_id: pane.id(),
                },
                handle: pane.notebook_view(ctx).downgrade(),
            });
        } else {
            safe_warn!(
                safe: ("Ignoring duplicate notebook pane registration"),
                full: ("Ignoring duplicate notebook pane registration for {notebook_id}")
            );
        }
    }

    // De-register an open notebook pane when it's removed from a pane group.
    pub fn deregister_pane(&mut self, pane: &NotebookPane, ctx: &mut ModelContext<Self>) {
        let Some(notebook_id) = pane.notebook_view(ctx).as_ref(ctx).notebook_id(ctx) else {
            log::warn!("Notebook pane has no notebook ID");
            return;
        };

        // If a notebook pane is restored, the notebook may have been reopened in the meantime. In
        // that case, don't let closing the original pane clear out the new pane.
        if let Entry::Occupied(entry) = self.panes_by_hashed_id.entry(notebook_id.uid()) {
            if entry.get().locator.pane_id == pane.id() {
                entry.remove();
            } else {
                log::warn!(
                    "Ignoring duplicate registration of panes for {}",
                    notebook_id.uid()
                );
            }
        }
    }

    /// Spawns an async thread to compute the notebook's raw text, adds this
    /// result to the cache ones the operation has been completed.
    fn update_raw_text_for_notebook(&mut self, notebook_id: SyncId, ctx: &mut ModelContext<Self>) {
        log::debug!("Updating raw text cache for {}", notebook_id.uid());
        let Some(notebook) = CloudModel::handle(ctx).read(ctx, |model, _| {
            Some(model.get_notebook(&notebook_id)?.clone())
        }) else {
            return;
        };

        if let Some(NotebookRawTextStatus::ParseInFlight(abort_handle)) =
            self.raw_text_by_hashed_id.get(&notebook_id.uid())
        {
            // If there's already a parse in flight, abort it
            abort_handle.abort();
        }

        let handle = Self::spawn_raw_text_parse_for_notebook(notebook, ctx);

        self.raw_text_by_hashed_id.insert(
            notebook_id.uid(),
            NotebookRawTextStatus::ParseInFlight(handle.abort_handle()),
        );
    }

    fn handle_update_manager_event(
        &mut self,
        _: ModelHandle<UpdateManager>,
        event: &UpdateManagerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let UpdateManagerEvent::ObjectOperationComplete { result } = event else {
            return;
        };

        if !matches!(&result.success_type, OperationSuccessType::Success) {
            return;
        }
        if let ObjectOperation::Create { .. } = result.operation {
            let server_id = result.server_id.expect("Expect server id on success");
            let Some(server_id) = CloudModel::as_ref(ctx)
                .get_notebook_by_uid(&server_id.uid())
                .and_then(|notebook| notebook.id.into_server())
            else {
                return;
            };
            let Some(client_id) = result.client_id else {
                return;
            };

            if let Some(mut pane) = self.panes_by_hashed_id.remove(&client_id.to_string()) {
                pane.notebook_id = SyncId::ServerId(server_id);
                self.panes_by_hashed_id
                    .insert(server_id.uid().clone(), pane);
            }
            if let Some(parse_status) = self.raw_text_by_hashed_id.remove(&client_id.to_string()) {
                self.raw_text_by_hashed_id
                    .insert(server_id.uid(), parse_status);
            }
        }
    }

    /// Swap the ID of the notebook open in a pane. This assumes the pane location and view are
    /// unchanged.
    pub(super) fn swap_notebook(&mut self, old_id: SyncId, new_id: SyncId) {
        if let Some(mut pane_data) = self.panes_by_hashed_id.remove(&old_id.uid()) {
            debug_assert_eq!(pane_data.notebook_id, old_id);
            pane_data.notebook_id = new_id;
            debug_assert!(
                self.panes_by_hashed_id
                    .insert(new_id.uid(), pane_data)
                    .is_none(),
                "New notebook was already open"
            );
        } else {
            log::warn!("Tried to swap notebooks, but the old one was not open");
        }
    }

    /// Close all open notebooks, saving any changes. This is called before the app terminates to
    /// prevent data loss, since notebooks are not saved immediately after every user edit.
    pub fn close_notebooks(&self, ctx: &mut ModelContext<Self>) {
        for pane in self.panes_by_hashed_id.values() {
            if let Some(notebook_view) = pane.handle.upgrade(ctx) {
                safe_debug!(
                    safe : ("Closing notebook on termination"),
                    full: ("Closing notebook {} on termination", pane.notebook_id)
                );
                notebook_view.update(ctx, |view, ctx| view.on_detach(ctx));
            }
        }
    }

    /// Reset the notebook manager state for logout.
    ///
    /// This _does not_ save any pending notebook changes.
    pub fn reset(&mut self) {
        self.panes_by_hashed_id.clear();
        for (_, status) in self.raw_text_by_hashed_id.drain() {
            if let NotebookRawTextStatus::ParseInFlight(handle) = status {
                handle.abort();
            }
        }
    }
}

struct NotebookPaneData {
    notebook_id: SyncId,
    window_id: WindowId,
    handle: WeakViewHandle<NotebookView>,
    locator: PaneViewLocator,
}

impl Entity for NotebookManager {
    type Event = ();
}

impl SingletonEntity for NotebookManager {}
