//! Settings UI for local scripting and Warp control permissions.
use super::{
    settings_page::{
        render_body_item, render_settings_info_banner, LocalOnlyIconState, MatchData, PageType,
        SettingsPageMeta, SettingsPageViewHandle, SettingsWidget,
    },
    SettingsSection, ToggleState,
};
use crate::appearance::Appearance;
use crate::features::FeatureFlag;
use crate::report_if_error;
use crate::settings::{
    AllowOutsideWarpAppStateMutations, AllowOutsideWarpControl,
    AllowOutsideWarpMetadataConfigurationMutations, AllowOutsideWarpMetadataReads,
    AllowOutsideWarpUnderlyingDataMutations, AllowOutsideWarpUnderlyingDataReads,
    LocalControlSettings,
};
use settings::{Setting as _, ToggleableSetting as _};
use std::cell::RefCell;
use std::collections::HashMap;
use warp_core::settings::SyncToCloud;
use warpui::elements::{Container, Element, MouseStateHandle};
use warpui::ui_components::components::UiComponent;
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

/// Toggle rows shown on the Settings > Scripting page for outside-Warp local-control gates.
#[derive(Clone, Copy, Debug)]
pub enum ScriptingToggle {
    OutsideWarpControl,
    OutsideWarpMetadataReads,
    OutsideWarpUnderlyingDataReads,
    OutsideWarpAppStateMutations,
    OutsideWarpMetadataConfigurationMutations,
    OutsideWarpUnderlyingDataMutations,
}

impl ScriptingToggle {
    fn label(self) -> &'static str {
        match self {
            Self::OutsideWarpControl => "Warp control outside Warp",
            Self::OutsideWarpMetadataReads => "Allow metadata reads",
            Self::OutsideWarpUnderlyingDataReads => "Allow underlying data reads",
            Self::OutsideWarpAppStateMutations => "Allow app-state mutations",
            Self::OutsideWarpMetadataConfigurationMutations => {
                "Allow metadata/configuration mutations"
            }
            Self::OutsideWarpUnderlyingDataMutations => "Allow underlying data mutations",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::OutsideWarpControl => {
                "Allows other local apps, terminals, IDEs, launch agents, and scripts to request Warp control."
            }
            Self::OutsideWarpMetadataReads => {
                "Allows external local clients to query app metadata after outside-Warp control is enabled."
            }
            Self::OutsideWarpUnderlyingDataReads => {
                "Allows external local clients to read underlying user data when those commands are implemented."
            }
            Self::OutsideWarpAppStateMutations => {
                "Allows external local clients to mutate Warp app state after outside-Warp control is enabled."
            }
            Self::OutsideWarpMetadataConfigurationMutations => {
                "Allows external local clients to change metadata and configuration when those commands are implemented."
            }
            Self::OutsideWarpUnderlyingDataMutations => {
                "Allows external local clients to mutate underlying user data when those commands are implemented."
            }
        }
    }

    fn search_terms(self) -> &'static str {
        match self {
            Self::OutsideWarpControl => {
                "outside warp control external scripts automation local cli"
            }
            Self::OutsideWarpMetadataReads => {
                "outside warp metadata read query windows tabs panes instances"
            }
            Self::OutsideWarpUnderlyingDataReads => {
                "outside warp underlying data read terminal output input history blocks"
            }
            Self::OutsideWarpAppStateMutations => {
                "outside warp app state mutate change tab create window pane"
            }
            Self::OutsideWarpMetadataConfigurationMutations => {
                "outside warp metadata configuration mutate settings theme labels"
            }
            Self::OutsideWarpUnderlyingDataMutations => {
                "outside warp underlying data mutate input files drive"
            }
        }
    }

    fn value(self, settings: &LocalControlSettings) -> bool {
        match self {
            Self::OutsideWarpControl => *settings.allow_outside_warp_control,
            Self::OutsideWarpMetadataReads => *settings.allow_outside_warp_metadata_reads,
            Self::OutsideWarpUnderlyingDataReads => {
                *settings.allow_outside_warp_underlying_data_reads
            }
            Self::OutsideWarpAppStateMutations => *settings.allow_outside_warp_app_state_mutations,
            Self::OutsideWarpMetadataConfigurationMutations => {
                *settings.allow_outside_warp_metadata_configuration_mutations
            }
            Self::OutsideWarpUnderlyingDataMutations => {
                *settings.allow_outside_warp_underlying_data_mutations
            }
        }
    }

    fn storage_key(self) -> &'static str {
        match self {
            Self::OutsideWarpControl => AllowOutsideWarpControl::storage_key(),
            Self::OutsideWarpMetadataReads => AllowOutsideWarpMetadataReads::storage_key(),
            Self::OutsideWarpUnderlyingDataReads => {
                AllowOutsideWarpUnderlyingDataReads::storage_key()
            }
            Self::OutsideWarpAppStateMutations => AllowOutsideWarpAppStateMutations::storage_key(),
            Self::OutsideWarpMetadataConfigurationMutations => {
                AllowOutsideWarpMetadataConfigurationMutations::storage_key()
            }
            Self::OutsideWarpUnderlyingDataMutations => {
                AllowOutsideWarpUnderlyingDataMutations::storage_key()
            }
        }
    }

    fn sync_to_cloud(self) -> SyncToCloud {
        match self {
            Self::OutsideWarpControl => AllowOutsideWarpControl::sync_to_cloud(),
            Self::OutsideWarpMetadataReads => AllowOutsideWarpMetadataReads::sync_to_cloud(),
            Self::OutsideWarpUnderlyingDataReads => {
                AllowOutsideWarpUnderlyingDataReads::sync_to_cloud()
            }
            Self::OutsideWarpAppStateMutations => {
                AllowOutsideWarpAppStateMutations::sync_to_cloud()
            }
            Self::OutsideWarpMetadataConfigurationMutations => {
                AllowOutsideWarpMetadataConfigurationMutations::sync_to_cloud()
            }
            Self::OutsideWarpUnderlyingDataMutations => {
                AllowOutsideWarpUnderlyingDataMutations::sync_to_cloud()
            }
        }
    }

    fn requires_outside_control(self) -> bool {
        match self {
            Self::OutsideWarpControl => false,
            Self::OutsideWarpMetadataReads
            | Self::OutsideWarpUnderlyingDataReads
            | Self::OutsideWarpAppStateMutations
            | Self::OutsideWarpMetadataConfigurationMutations
            | Self::OutsideWarpUnderlyingDataMutations => true,
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
        if FeatureFlag::WarpControlCli.is_enabled() {
            ctx.subscribe_to_model(&LocalControlSettings::handle(ctx), |_, _, _, ctx| {
                ctx.notify();
            });
        }

        Self {
            page: PageType::new_uncategorized(
                vec![
                    Box::new(ScriptingIntroWidget),
                    Box::new(ScriptingToggleWidget::new(
                        ScriptingToggle::OutsideWarpControl,
                    )),
                    Box::new(ScriptingToggleWidget::new(
                        ScriptingToggle::OutsideWarpMetadataReads,
                    )),
                    Box::new(ScriptingToggleWidget::new(
                        ScriptingToggle::OutsideWarpUnderlyingDataReads,
                    )),
                    Box::new(ScriptingToggleWidget::new(
                        ScriptingToggle::OutsideWarpAppStateMutations,
                    )),
                    Box::new(ScriptingToggleWidget::new(
                        ScriptingToggle::OutsideWarpMetadataConfigurationMutations,
                    )),
                    Box::new(ScriptingToggleWidget::new(
                        ScriptingToggle::OutsideWarpUnderlyingDataMutations,
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
                    ScriptingToggle::OutsideWarpControl => {
                        report_if_error!(settings
                            .allow_outside_warp_control
                            .toggle_and_save_value(ctx));
                    }
                    ScriptingToggle::OutsideWarpMetadataReads => {
                        report_if_error!(settings
                            .allow_outside_warp_metadata_reads
                            .toggle_and_save_value(ctx));
                    }
                    ScriptingToggle::OutsideWarpUnderlyingDataReads => {
                        report_if_error!(settings
                            .allow_outside_warp_underlying_data_reads
                            .toggle_and_save_value(ctx));
                    }
                    ScriptingToggle::OutsideWarpAppStateMutations => {
                        report_if_error!(settings
                            .allow_outside_warp_app_state_mutations
                            .toggle_and_save_value(ctx));
                    }
                    ScriptingToggle::OutsideWarpMetadataConfigurationMutations => {
                        report_if_error!(settings
                            .allow_outside_warp_metadata_configuration_mutations
                            .toggle_and_save_value(ctx));
                    }
                    ScriptingToggle::OutsideWarpUnderlyingDataMutations => {
                        report_if_error!(settings
                            .allow_outside_warp_underlying_data_mutations
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

struct ScriptingIntroWidget;

impl SettingsWidget for ScriptingIntroWidget {
    type View = ScriptingSettingsPageView;

    fn search_terms(&self) -> &str {
        "scripting warp control automation warpctrl local cli outside read only read write"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        render_settings_info_banner(
            "Warp control lets local scripts automate allowlisted actions in a running Warp app.",
            Some("This foundation branch supports outside-Warp local clients only. Verified Warp-managed terminal invocations are planned for a later implementation and are currently rejected by the credential broker."),
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
        !self.toggle.requires_outside_control() || settings.outside_warp_control_enabled()
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
        if self.toggle.requires_outside_control() {
            Container::new(item).with_margin_left(16.).finish()
        } else {
            item
        }
    }
}
