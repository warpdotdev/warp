use warpui::{AppContext, ModelHandle, ViewContext, ViewHandle};

use crate::{
    ai::ambient_agents::github_auth_url::GithubAuthRedirectTarget,
    app_state::{EnvironmentManagementPaneSnapshot, LeafContents},
    pane_group::focus_state::PaneFocusHandle,
    settings_view::{
        environments_page::{EnvironmentsPage, EnvironmentsPageView},
        settings_page::{PaneEventWrapper, SettingsPageEvent},
    },
};

use super::{
    view::PaneView, DetachType, PaneConfiguration, PaneContent, PaneEvent, PaneGroup, PaneId,
    ShareableLink, ShareableLinkError,
};

pub struct EnvironmentManagementPane {
    view: ViewHandle<PaneView<EnvironmentsPageView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl EnvironmentManagementPane {
    pub fn new(ctx: &mut ViewContext<PaneGroup>) -> Self {
        // Create the EnvironmentsPageView
        let environments_page_view = ctx.add_typed_action_view(|ctx| {
            let mut view = EnvironmentsPageView::new(ctx);
            view.set_github_auth_redirect_target(GithubAuthRedirectTarget::FocusCloudMode, ctx);
            view
        });

        Self::from_view(environments_page_view, ctx)
    }

    pub fn from_view(
        environments_page_view: ViewHandle<EnvironmentsPageView>,
        ctx: &mut AppContext,
    ) -> Self {
        let pane_configuration = environments_page_view.as_ref(ctx).pane_configuration();
        let window_id = environments_page_view.window_id(ctx);

        let view = ctx.add_typed_action_view(window_id, |ctx| {
            let pane_id = PaneId::from_environment_management_pane_ctx(ctx);
            PaneView::new(
                pane_id,
                environments_page_view,
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

    pub fn environments_page_view(&self, ctx: &AppContext) -> ViewHandle<EnvironmentsPageView> {
        self.view.as_ref(ctx).child(ctx)
    }

    /// Returns the current mode of the environment management pane.
    pub fn current_mode(&self, ctx: &AppContext) -> EnvironmentsPage {
        self.environments_page_view(ctx)
            .as_ref(ctx)
            .current_page()
            .clone()
    }
}

impl PaneContent for EnvironmentManagementPane {
    fn id(&self) -> PaneId {
        PaneId::from_environment_management_pane_view(&self.view)
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
            &self.environments_page_view(ctx),
            move |pane_group, _, event, ctx| match event {
                SettingsPageEvent::Pane(pane_event_wrapper) => {
                    let pane_event = match pane_event_wrapper {
                        PaneEventWrapper::Close => PaneEvent::Close,
                    };
                    pane_group.handle_pane_event(pane_id, &pane_event, ctx);
                }
                SettingsPageEvent::EnvironmentSetupModeSelectorToggled { is_open } => {
                    pane_group.pane_with_open_environment_setup_mode_selector =
                        is_open.then_some(pane_id);
                    ctx.notify();
                }
                SettingsPageEvent::AgentAssistedEnvironmentModalToggled { is_open } => {
                    pane_group.pane_with_open_agent_assisted_environment_modal =
                        is_open.then_some(pane_id);
                    ctx.notify();
                }
                SettingsPageEvent::FocusModal => {
                    // Not applicable when hosted in a pane.
                }
            },
        );

        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });
    }

    fn detach(
        &self,
        _group: &PaneGroup,
        _detach_type: DetachType,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        let environments_page_view = self.environments_page_view(ctx);
        ctx.unsubscribe_to_view(&environments_page_view);
        ctx.unsubscribe_to_view(&self.view);
    }

    fn snapshot(&self, ctx: &AppContext) -> LeafContents {
        LeafContents::EnvironmentManagement(EnvironmentManagementPaneSnapshot {
            mode: self.current_mode(ctx),
        })
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.environments_page_view(ctx)
            .update(ctx, |view, ctx| view.focus(ctx));
    }

    fn shareable_link(
        &self,
        _ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<ShareableLink, ShareableLinkError> {
        Ok(ShareableLink::Base)
    }

    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool {
        self.view.as_ref(ctx).is_being_dragged()
    }
}
