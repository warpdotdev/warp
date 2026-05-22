use super::{
    settings_page::{
        render_body_item, render_settings_info_banner, LocalOnlyIconState, MatchData, PageType,
        SettingsPageMeta, SettingsPageViewHandle, SettingsWidget,
    },
    SettingsSection, ToggleState,
};
use crate::appearance::Appearance;
use crate::report_if_error;
use crate::settings::{
    AllowInsideWarpControl, AllowInsideWarpReadOnly, AllowInsideWarpReadWrite,
    AllowOutsideWarpControl, AllowOutsideWarpReadOnly, AllowOutsideWarpReadWrite,
    LocalControlInvocationContext, LocalControlSettings,
};
use settings::{Setting as _, ToggleableSetting as _};
use std::cell::RefCell;
use std::collections::HashMap;
use warp_core::settings::SyncToCloud;
use warpui::elements::{Container, Element, MouseStateHandle};
use warpui::ui_components::components::UiComponent;
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

#[derive(Clone, Copy, Debug)]
pub enum ScriptingToggle {
    InsideWarpControl,
    InsideWarpReadOnly,
    InsideWarpReadWrite,
    OutsideWarpControl,
    OutsideWarpReadOnly,
    OutsideWarpReadWrite,
}

impl ScriptingToggle {
    fn label(self) -> &'static str {
        match self {
            Self::InsideWarpControl => "Warp control within Warp",
            Self::OutsideWarpControl => "Warp control outside Warp",
            Self::InsideWarpReadOnly | Self::OutsideWarpReadOnly => "Allow read-only control",
            Self::InsideWarpReadWrite | Self::OutsideWarpReadWrite => "Allow read-write control",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::InsideWarpControl => {
                "Allows control commands launched from verified Warp-managed terminal sessions."
            }
            Self::OutsideWarpControl => {
                "Allows other local apps, terminals, IDEs, launch agents, and scripts to request Warp control."
            }
            Self::InsideWarpReadOnly => {
                "Allows commands inside Warp to query app information such as instances, windows, tabs, and protocol version."
            }
            Self::OutsideWarpReadOnly => {
                "Allows external local clients to query app information after outside-Warp control is enabled."
            }
            Self::InsideWarpReadWrite => {
                "Allows commands inside Warp to change Warp app state, such as creating a tab."
            }
            Self::OutsideWarpReadWrite => {
                "Allows external local clients to change Warp app state after outside-Warp control is enabled."
            }
        }
    }

    fn search_terms(self) -> &'static str {
        match self {
            Self::InsideWarpControl => "inside warp control terminal scripting automation",
            Self::OutsideWarpControl => {
                "outside warp control external scripts automation local cli"
            }
            Self::InsideWarpReadOnly => "inside warp read only query windows tabs panes instances",
            Self::OutsideWarpReadOnly => {
                "outside warp read only query windows tabs panes instances"
            }
            Self::InsideWarpReadWrite => "inside warp read write mutate change tab create",
            Self::OutsideWarpReadWrite => "outside warp read write mutate change tab create",
        }
    }

    fn value(self, settings: &LocalControlSettings) -> bool {
        match self {
            Self::InsideWarpControl => *settings.allow_inside_warp_control,
            Self::OutsideWarpControl => *settings.allow_outside_warp_control,
            Self::InsideWarpReadOnly => *settings.allow_inside_warp_read_only,
            Self::OutsideWarpReadOnly => *settings.allow_outside_warp_read_only,
            Self::InsideWarpReadWrite => *settings.allow_inside_warp_read_write,
            Self::OutsideWarpReadWrite => *settings.allow_outside_warp_read_write,
        }
    }

    fn storage_key(self) -> &'static str {
        match self {
            Self::InsideWarpControl => AllowInsideWarpControl::storage_key(),
            Self::OutsideWarpControl => AllowOutsideWarpControl::storage_key(),
            Self::InsideWarpReadOnly => AllowInsideWarpReadOnly::storage_key(),
            Self::OutsideWarpReadOnly => AllowOutsideWarpReadOnly::storage_key(),
            Self::InsideWarpReadWrite => AllowInsideWarpReadWrite::storage_key(),
            Self::OutsideWarpReadWrite => AllowOutsideWarpReadWrite::storage_key(),
        }
    }

    fn sync_to_cloud(self) -> SyncToCloud {
        match self {
            Self::InsideWarpControl => AllowInsideWarpControl::sync_to_cloud(),
            Self::OutsideWarpControl => AllowOutsideWarpControl::sync_to_cloud(),
            Self::InsideWarpReadOnly => AllowInsideWarpReadOnly::sync_to_cloud(),
            Self::OutsideWarpReadOnly => AllowOutsideWarpReadOnly::sync_to_cloud(),
            Self::InsideWarpReadWrite => AllowInsideWarpReadWrite::sync_to_cloud(),
            Self::OutsideWarpReadWrite => AllowOutsideWarpReadWrite::sync_to_cloud(),
        }
    }

    fn parent_context(self) -> Option<LocalControlInvocationContext> {
        match self {
            Self::InsideWarpReadOnly | Self::InsideWarpReadWrite => {
                Some(LocalControlInvocationContext::InsideWarp)
            }
            Self::OutsideWarpReadOnly | Self::OutsideWarpReadWrite => {
                Some(LocalControlInvocationContext::OutsideWarp)
            }
            Self::InsideWarpControl | Self::OutsideWarpControl => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ScriptingSettingsPageAction {
    Toggle(ScriptingToggle),
}

pub struct ScriptingSettingsPageView {
    page: PageType<Self>,
    local_only_icon_tooltip_states: RefCell<HashMap<String, MouseStateHandle>>,
}

impl ScriptingSettingsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&LocalControlSettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });

        Self {
            page: PageType::new_uncategorized(
                vec![
                    Box::new(ScriptingIntroWidget),
                    Box::new(ScriptingToggleWidget::new(
                        ScriptingToggle::InsideWarpControl,
                    )),
                    Box::new(ScriptingToggleWidget::new(
                        ScriptingToggle::InsideWarpReadOnly,
                    )),
                    Box::new(ScriptingToggleWidget::new(
                        ScriptingToggle::InsideWarpReadWrite,
                    )),
                    Box::new(ScriptingToggleWidget::new(
                        ScriptingToggle::OutsideWarpControl,
                    )),
                    Box::new(ScriptingToggleWidget::new(
                        ScriptingToggle::OutsideWarpReadOnly,
                    )),
                    Box::new(ScriptingToggleWidget::new(
                        ScriptingToggle::OutsideWarpReadWrite,
                    )),
                ],
                Some("Scripting"),
            ),
            local_only_icon_tooltip_states: RefCell::new(HashMap::new()),
        }
    }
}

impl Entity for ScriptingSettingsPageView {
    type Event = ();
}

impl TypedActionView for ScriptingSettingsPageView {
    type Action = ScriptingSettingsPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ScriptingSettingsPageAction::Toggle(toggle) => {
                LocalControlSettings::handle(ctx).update(ctx, |settings, ctx| match toggle {
                    ScriptingToggle::InsideWarpControl => {
                        report_if_error!(settings
                            .allow_inside_warp_control
                            .toggle_and_save_value(ctx));
                    }
                    ScriptingToggle::OutsideWarpControl => {
                        report_if_error!(settings
                            .allow_outside_warp_control
                            .toggle_and_save_value(ctx));
                    }
                    ScriptingToggle::InsideWarpReadOnly => {
                        report_if_error!(settings
                            .allow_inside_warp_read_only
                            .toggle_and_save_value(ctx));
                    }
                    ScriptingToggle::OutsideWarpReadOnly => {
                        report_if_error!(settings
                            .allow_outside_warp_read_only
                            .toggle_and_save_value(ctx));
                    }
                    ScriptingToggle::InsideWarpReadWrite => {
                        report_if_error!(settings
                            .allow_inside_warp_read_write
                            .toggle_and_save_value(ctx));
                    }
                    ScriptingToggle::OutsideWarpReadWrite => {
                        report_if_error!(settings
                            .allow_outside_warp_read_write
                            .toggle_and_save_value(ctx));
                    }
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
        cfg!(not(target_family = "wasm"))
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

struct ScriptingIntroWidget;

impl SettingsWidget for ScriptingIntroWidget {
    type View = ScriptingSettingsPageView;

    fn search_terms(&self) -> &str {
        "scripting warp control automation warpctrl local cli inside outside read only read write"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        render_settings_info_banner(
            "Warp control lets local scripts automate allowlisted actions in a running Warp app.",
            Some("Enable Warp control within Warp for commands launched from Warp-managed terminals, or outside Warp for other local apps and scripts. Each scope can allow read-only queries and read-write app changes separately."),
            appearance,
        )
    }
}

struct ScriptingToggleWidget {
    toggle: ScriptingToggle,
    switch_state: SwitchStateHandle,
}

impl ScriptingToggleWidget {
    fn new(toggle: ScriptingToggle) -> Self {
        Self {
            toggle,
            switch_state: SwitchStateHandle::default(),
        }
    }
}

impl SettingsWidget for ScriptingToggleWidget {
    type View = ScriptingSettingsPageView;

    fn search_terms(&self) -> &str {
        self.toggle.search_terms()
    }

    fn should_render(&self, app: &AppContext) -> bool {
        let settings = LocalControlSettings::as_ref(app);
        match self.toggle.parent_context() {
            Some(context) => settings.is_context_enabled(context),
            None => true,
        }
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let settings = LocalControlSettings::as_ref(app);
        let checked = self.toggle.value(settings);
        let toggle = self.toggle;

        let item = render_body_item::<ScriptingSettingsPageAction>(
            self.toggle.label().to_owned(),
            None,
            LocalOnlyIconState::for_setting(
                self.toggle.storage_key(),
                self.toggle.sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(checked)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(ScriptingSettingsPageAction::Toggle(toggle));
                })
                .finish(),
            Some(self.toggle.description().to_owned()),
        );
        if self.toggle.parent_context().is_some() {
            Container::new(item).with_margin_left(16.).finish()
        } else {
            item
        }
    }
}
