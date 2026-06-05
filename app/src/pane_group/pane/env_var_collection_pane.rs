use anyhow::Context;
use warpui::{AppContext, ModelHandle, SingletonEntity, ViewContext, ViewHandle};

use super::view::PaneView;
use super::{
    DetachType, PaneConfiguration, PaneContent, PaneGroup, PaneId, ShareableLink,
    ShareableLinkError,
};
use crate::app_state::{EnvVarCollectionPaneSnapshot, LeafContents};
use crate::drive::items::WarpDriveItemId;
use crate::env_vars::manager::{EnvVarCollectionManager, EnvVarCollectionSource};
use crate::env_vars::view::env_var_collection::{EnvVarCollectionEvent, EnvVarCollectionView};
use crate::env_vars::EnvVarCollectionType;
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::server::ids::SyncId;
use crate::workspaces::user_workspaces::UserWorkspaces;

pub struct EnvVarCollectionPane {
    view: ViewHandle<PaneView<EnvVarCollectionView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl EnvVarCollectionPane {
    pub fn new(
        env_var_collection_view: ViewHandle<EnvVarCollectionView>,
        ctx: &mut AppContext,
    ) -> Self {
        let pane_configuration = env_var_collection_view
            .as_ref(ctx)
            .pane_configuration()
            .to_owned();
        let view = ctx.add_typed_action_view(env_var_collection_view.window_id(ctx), |ctx| {
            let pane_id = PaneId::from_env_var_collection_pane_ctx(ctx);
            PaneView::new(
                pane_id,
                env_var_collection_view,
                (),
                pane_configuration.clone(),
                ctx,
            )
        });

        Self {
            view,
            pane_configuration,
        }
    }

    pub fn restore(
        env_var_collection_id: Option<SyncId>,
        ctx: &mut ViewContext<PaneGroup>,
    ) -> anyhow::Result<Self> {
        let window_id = ctx.window_id();
        let source = match env_var_collection_id {
            Some(id) => EnvVarCollectionSource::Existing(id),
            None => EnvVarCollectionSource::New {
                title: None,
                owner: UserWorkspaces::as_ref(ctx)
                    .personal_drive(ctx)
                    .context("personal drive unavailable")?,
                initial_folder_id: None,
            },
        };

        Ok(
            EnvVarCollectionManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.create_pane(&source, window_id, ctx)
            }),
        )
    }

    pub fn env_var_collection_view(&self, ctx: &AppContext) -> ViewHandle<EnvVarCollectionView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

impl PaneContent for EnvVarCollectionPane {
    fn id(&self) -> PaneId {
        PaneId::from_env_var_collection_view(&self.view)
    }

    fn snapshot(&self, app: &AppContext) -> LeafContents {
        let env_var_collection_id = self
            .env_var_collection_view(app)
            .as_ref(app)
            .env_var_collection_id(app);
        LeafContents::EnvVarCollection(EnvVarCollectionPaneSnapshot::CloudEnvVarCollection {
            env_var_collection_id,
        })
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        let pane_id = self.id();
        ctx.subscribe_to_view(
            &self.env_var_collection_view(ctx),
            move |group, _, event, ctx| {
                handle_env_var_collection_event(group, pane_id, event, ctx);
            },
        );

        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });

        let pane_group_id = ctx.view_id();
        let window_id = ctx.window_id();
        EnvVarCollectionManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.register_pane(self, pane_group_id, window_id, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        // Always unsubscribe from views
        let env_var_collection_view = self.env_var_collection_view(ctx);
        ctx.unsubscribe_to_view(&env_var_collection_view);
        ctx.unsubscribe_to_view(&self.view);

        // Always deregister from EnvVarCollectionManager - it will be re-registered on attach if restored
        EnvVarCollectionManager::handle(ctx)
            .update(ctx, |manager, ctx| manager.deregister_pane(self, ctx));
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.env_var_collection_view(ctx)
            .update(ctx, |view, ctx| view.focus(ctx));
    }

    fn shareable_link(
        &self,
        _ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<ShareableLink, ShareableLinkError> {
        // TODO: sega
        Ok(ShareableLink::Base)
    }

    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool {
        self.view.as_ref(ctx).is_being_dragged()
    }
}

fn handle_env_var_collection_event(
    group: &mut PaneGroup,
    pane_id: PaneId,
    event: &EnvVarCollectionEvent,
    ctx: &mut ViewContext<PaneGroup>,
) {
    match event {
        EnvVarCollectionEvent::Pane(pane_event) => {
            group.handle_pane_event(pane_id, pane_event, ctx)
        }
        EnvVarCollectionEvent::ViewInWarpDrive(id) => view_in_warp_drive(*id, ctx),
        EnvVarCollectionEvent::Invoke(env_var_collection) => {
            invoke_env_var_collection(env_var_collection.clone(), ctx)
        }
        EnvVarCollectionEvent::UpdatedEnvVarCollection(_) => {
            log::warn!("EVC updates not yet handled by EVC pane")
        }
    }
}

fn invoke_env_var_collection(
    env_var_collection: EnvVarCollectionType,
    ctx: &mut ViewContext<PaneGroup>,
) {
    ctx.emit(crate::pane_group::Event::InvokeEnvVarCollection {
        env_var_collection: env_var_collection.into(),
        in_subshell: false,
    })
}

fn view_in_warp_drive(id: WarpDriveItemId, ctx: &mut ViewContext<PaneGroup>) {
    ctx.emit(crate::pane_group::Event::ViewInWarpDrive(id))
}
