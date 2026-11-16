use warpui::{AppContext, ModelHandle, SingletonEntity, View, ViewContext, ViewHandle, WindowId};

use super::view::PaneView;
use super::{
    DetachType, PaneConfiguration, PaneContent, PaneGroup, PaneId, ShareableLink,
    ShareableLinkError,
};
use crate::app_state::{LeafContents, SettingsPaneSnapshot};
use crate::settings_view::pane_manager::SettingsPaneManager;
use crate::settings_view::{SettingsSection, SettingsView, SettingsViewEvent};

pub struct SettingsPane {
    view: ViewHandle<PaneView<SettingsView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl SettingsPane {
    fn from_view(settings_view: ViewHandle<SettingsView>, ctx: &mut AppContext) -> Self {
        let pane_configuration = settings_view.as_ref(ctx).pane_configuration();
        let view = ctx.add_typed_action_view(settings_view.window_id(ctx), |ctx| {
            let pane_id = PaneId::from_settings_pane_ctx(ctx);
            PaneView::new(pane_id, settings_view, (), pane_configuration.clone(), ctx)
        });

        Self {
            view,
            pane_configuration,
        }
    }

    pub fn new<V: View>(
        page: SettingsSection,
        search_query: Option<&str>,
        window_id: WindowId,
        ctx: &mut ViewContext<V>,
    ) -> Self {
        let view = SettingsPaneManager::handle(ctx)
            .read(ctx, |manager, _| manager.settings_view(window_id));
        view.update(ctx, |view, ctx| {
            view.set_and_refresh_current_page(page, ctx);
            if let Some(search_query) = search_query {
                view.set_search_query(search_query, ctx);
            }
        });
        Self::from_view(view, ctx)
    }

    fn settings_view(&self, ctx: &AppContext) -> ViewHandle<SettingsView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

impl PaneContent for SettingsPane {
    fn id(&self) -> PaneId {
        PaneId::from_settings_pane_view(&self.view)
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: crate::pane_group::focus_state::PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        let pane_id = self.id();
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        ctx.subscribe_to_view(
            &self.settings_view(ctx),
            move |pane_group, _, event, ctx| handle_settings_event(pane_group, pane_id, event, ctx),
        );

        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });

        let pane_group_id = ctx.view_id();
        let window_id = ctx.window_id();
        SettingsPaneManager::handle(ctx).update(ctx, |manager, ctx| {
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
        let settings_view = self.settings_view(ctx);
        ctx.unsubscribe_to_view(&settings_view);
        ctx.unsubscribe_to_view(&self.view);

        // Always deregister from SettingsPaneManager - it will be re-registered on attach if restored.
        // Only clear the locator if this is the currently registered settings pane for the window.
        let window_id = ctx.window_id();
        let pane_group_id = ctx.view_id();
        let pane_id = self.id();
        SettingsPaneManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.deregister_pane(&window_id, pane_group_id, pane_id, ctx);
        });
    }

    fn snapshot(&self, app: &AppContext) -> LeafContents {
        let view = self.settings_view(app);
        let current_page = view.as_ref(app).current_settings_section();
        LeafContents::Settings(SettingsPaneSnapshot::Local {
            current_page,
            search_query: None,
        })
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        self.settings_view(ctx)
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

fn handle_settings_event(
    group: &mut PaneGroup,
    pane_id: PaneId,
    event: &SettingsViewEvent,
    ctx: &mut ViewContext<PaneGroup>,
) {
    if let SettingsViewEvent::Pane(pane_event) = event {
        group.handle_pane_event(pane_id, pane_event, ctx);
    }
}
