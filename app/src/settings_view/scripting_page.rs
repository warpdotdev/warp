//! Settings UI for local scripting and Warp control permissions.
use std::cell::RefCell;
use std::collections::HashMap;

use settings::Setting as _;
use warpui::elements::{ChildView, Element, MouseStateHandle};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use super::settings_page::{
    render_body_item, LocalOnlyIconState, MatchData, PageType, SettingsPageMeta,
    SettingsPageViewHandle, SettingsWidget,
};
use super::{SettingsSection, ToggleState};
use crate::appearance::Appearance;
use crate::features::FeatureFlag;
use crate::report_if_error;
use crate::settings::{LocalControlMode, LocalControlModeSetting, LocalControlSettings};
use crate::view_components::{Dropdown, DropdownItem};

#[derive(Clone, Debug, PartialEq)]
pub enum ScriptingSettingsPageAction {
    SetLocalControlMode(LocalControlMode),
}

pub struct ScriptingSettingsPageView {
    page: PageType<Self>,
    local_only_icon_tooltip_states: RefCell<HashMap<String, MouseStateHandle>>,
    local_control_mode_dropdown: ViewHandle<Dropdown<ScriptingSettingsPageAction>>,
}

impl ScriptingSettingsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let local_control_mode_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(360.);
            dropdown
        });
        Self::update_local_control_mode_dropdown(local_control_mode_dropdown.clone(), ctx);

        if FeatureFlag::WarpControlCli.is_enabled() {
            ctx.subscribe_to_model(&LocalControlSettings::handle(ctx), |view, _, _, ctx| {
                Self::update_local_control_mode_dropdown(
                    view.local_control_mode_dropdown.clone(),
                    ctx,
                );
                ctx.notify();
            });
        }

        Self {
            page: PageType::new_uncategorized(
                vec![Box::new(LocalControlModeWidget)],
                Some("Scripting"),
            ),
            local_only_icon_tooltip_states: RefCell::new(HashMap::new()),
            local_control_mode_dropdown,
        }
    }

    fn update_local_control_mode_dropdown(
        dropdown: ViewHandle<Dropdown<ScriptingSettingsPageAction>>,
        ctx: &mut ViewContext<Self>,
    ) {
        let current_mode = LocalControlSettings::as_ref(ctx).mode();
        dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(
                LocalControlMode::ALL
                    .into_iter()
                    .map(|mode| {
                        DropdownItem::new(
                            mode.as_dropdown_label(),
                            ScriptingSettingsPageAction::SetLocalControlMode(mode),
                        )
                    })
                    .collect(),
                ctx,
            );
            dropdown.set_selected_by_action(
                ScriptingSettingsPageAction::SetLocalControlMode(current_mode),
                ctx,
            );
        });
    }
}

impl Entity for ScriptingSettingsPageView {
    type Event = ();
}

impl TypedActionView for ScriptingSettingsPageView {
    type Action = ScriptingSettingsPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ScriptingSettingsPageAction::SetLocalControlMode(mode) => {
                LocalControlSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.local_control_mode.set_value(*mode, ctx));
                });
                ctx.notify();
            }
        }
    }
}

impl View for ScriptingSettingsPageView {
    fn ui_name() -> &'static str {
        "ScriptingSettingsPage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl SettingsPageMeta for ScriptingSettingsPageView {
    fn section() -> SettingsSection {
        SettingsSection::Scripting
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        cfg!(not(target_family = "wasm")) && FeatureFlag::WarpControlCli.is_enabled()
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<ScriptingSettingsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<ScriptingSettingsPageView>) -> Self {
        SettingsPageViewHandle::Scripting(view_handle)
    }
}

struct LocalControlModeWidget;

impl SettingsWidget for LocalControlModeWidget {
    type View = ScriptingSettingsPageView;

    fn search_terms(&self) -> &str {
        "scripting warp control automation warpctrl local cli inside warp outside warp external scripts disabled enabled"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        render_body_item::<ScriptingSettingsPageAction>(
            "warpctrl CLI".into(),
            None,
            LocalOnlyIconState::for_setting(
                LocalControlModeSetting::storage_key(),
                LocalControlModeSetting::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            ChildView::new(&view.local_control_mode_dropdown).finish(),
            Some("warpctrl allows for scripting Warp's UI.  Use with care.'".to_owned()),
        )
    }
}
