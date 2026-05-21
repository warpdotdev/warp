//! Modal for customizing the agent input footer chip layout.
//!
//! Uses the shared [`ChipConfigurator`] with `LeftRightZones` layout to let users
//! drag/drop chips between left, right, and unused banks.

use warpui::keymap::FixedBinding;

use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::chip_configurator::{
    render_chip_editor_modal, render_chip_editor_sections, ChipConfigurator,
    ChipConfiguratorAction, ChipConfiguratorLayout, ChipEditorModalConfig, ChipEditorMouseHandles,
    ChipEditorSectionsConfig,
};
use crate::report_if_error;
use crate::terminal::session_settings::{
    AgentToolbarChipSelection, CLIAgentToolbarChipSelection, SessionSettings,
    SessionSettingsChangedEvent, ToolbarChipSelection,
};
use crate::Appearance;

use settings::Setting as _;

use super::toolbar_item::AgentToolbarItemKind;

const AGENT_MODAL_TITLE: &str = "Edit agent toolbelt";
const CLI_MODAL_TITLE: &str = "Edit CLI agent toolbelt";

/// Controls which set of items and settings the editor modal operates on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentToolbarEditorMode {
    #[default]
    AgentView,
    CLIAgent,
}
pub enum AgentToolbarEditorEvent {
    Close,
}

pub struct AgentToolbarEditorModal {
    mouse_handles: ChipEditorMouseHandles,
    chip_configurator: ChipConfigurator,
    mode: AgentToolbarEditorMode,
    is_dirty: bool,
}

pub struct AgentToolbarInlineEditor {
    mouse_handles: ChipEditorMouseHandles,
    chip_configurator: ChipConfigurator,
    mode: AgentToolbarEditorMode,
}

#[derive(Clone, Copy, Debug)]
pub enum AgentToolbarEditorAction {
    Cancel,
    Save,
    Chip(ChipConfiguratorAction),
    ResetDefault,
    /// Dummy action used as on_click for chip bank clicks (no-op).
    Activate,
}

#[derive(Clone, Copy, Debug)]
pub enum AgentToolbarInlineEditorAction {
    Chip(ChipConfiguratorAction),
    ResetDefault,
    /// Dummy action used as on_click for chip bank clicks (no-op).
    Activate,
}

fn open_toolbar_items_from_settings<V: View>(
    chip_configurator: &mut ChipConfigurator,
    mode: AgentToolbarEditorMode,
    ctx: &mut ViewContext<V>,
) {
    let appearance = Appearance::as_ref(ctx);
    let session_settings = SessionSettings::as_ref(ctx);
    let (current_left, current_right, available) = match mode {
        AgentToolbarEditorMode::AgentView => {
            let selection = session_settings.agent_footer_chip_selection.clone();
            (
                selection.left_items(),
                selection.right_items(),
                AgentToolbarItemKind::all_available(),
            )
        }
        AgentToolbarEditorMode::CLIAgent => {
            let selection = session_settings.cli_agent_footer_chip_selection.clone();
            (
                selection.left_items(),
                selection.right_items(),
                AgentToolbarItemKind::all_available_for_cli_input(),
            )
        }
    };

    // Filter out items that are unavailable due to runtime state (user settings,
    // workspace config, etc.) on top of the feature-flag checks in all_available().
    let available: Vec<AgentToolbarItemKind> = available
        .into_iter()
        .filter(|item| item.is_available(ctx))
        .collect();

    // Drop saved items that are no longer available (e.g. their feature flag was disabled
    // or a setting was turned off).
    let filter_unavailable = |items: Vec<AgentToolbarItemKind>| -> Vec<AgentToolbarItemKind> {
        items
            .into_iter()
            .filter(|item| available.contains(item))
            .collect()
    };
    let current_left = filter_unavailable(current_left);
    let current_right = filter_unavailable(current_right);

    chip_configurator.open_left_right_zones_with_items(
        current_left,
        current_right,
        available,
        appearance,
    );
}

fn open_default_toolbar_items<V: View>(
    chip_configurator: &mut ChipConfigurator,
    mode: AgentToolbarEditorMode,
    ctx: &mut ViewContext<V>,
) {
    let appearance = Appearance::as_ref(ctx);
    let (left, right, available) = AgentToolbarItemKind::defaults_for_mode(mode);
    let filter_runtime = |items: Vec<AgentToolbarItemKind>| -> Vec<AgentToolbarItemKind> {
        items
            .into_iter()
            .filter(|item| item.is_available(ctx))
            .collect()
    };
    let left = filter_runtime(left);
    let right = filter_runtime(right);
    let available = filter_runtime(available);
    chip_configurator.open_left_right_zones_with_items(left, right, available, appearance);
}

fn is_toolbar_editor_at_defaults(
    mode: AgentToolbarEditorMode,
    chip_configurator: &ChipConfigurator,
) -> bool {
    let left = chip_configurator.left_item_kinds();
    let right = chip_configurator.right_item_kinds();
    toolbar_items_match_defaults(mode, &left, &right)
}

fn toolbar_items_match_defaults(
    mode: AgentToolbarEditorMode,
    left: &[AgentToolbarItemKind],
    right: &[AgentToolbarItemKind],
) -> bool {
    let (default_left, default_right, _) = AgentToolbarItemKind::defaults_for_mode(mode);
    default_left.as_slice() == left && default_right.as_slice() == right
}

impl AgentToolbarInlineEditor {
    pub fn new(mode: AgentToolbarEditorMode, ctx: &mut ViewContext<Self>) -> Self {
        let mut editor = Self {
            mouse_handles: Default::default(),
            chip_configurator: ChipConfigurator::new(ChipConfiguratorLayout::LeftRightZones),
            mode,
        };
        editor.reset_from_settings(ctx);

        ctx.subscribe_to_model(&SessionSettings::handle(ctx), |me, _, event, ctx| {
            let should_refresh = matches!(
                (me.mode, event),
                (
                    AgentToolbarEditorMode::AgentView,
                    SessionSettingsChangedEvent::AgentToolbarChipSelectionSetting { .. },
                ) | (
                    AgentToolbarEditorMode::CLIAgent,
                    SessionSettingsChangedEvent::CLIAgentToolbarChipSelectionSetting { .. },
                )
            );

            if should_refresh && me.chip_configurator.current_dragging_state.is_none() {
                me.reset_from_settings(ctx);
                ctx.notify();
            }
        });

        editor
    }

    fn reset_from_settings(&mut self, ctx: &mut ViewContext<Self>) {
        open_toolbar_items_from_settings(&mut self.chip_configurator, self.mode, ctx);
    }

    fn save_current_selection(&self, ctx: &mut ViewContext<Self>) {
        let left = self.chip_configurator.left_item_kinds();
        let right = self.chip_configurator.right_item_kinds();
        save_toolbar_selection(self.mode, left, right, ctx);
    }

    fn is_at_defaults(&self) -> bool {
        is_toolbar_editor_at_defaults(self.mode, &self.chip_configurator)
    }
}

impl Entity for AgentToolbarInlineEditor {
    type Event = ();
}

impl TypedActionView for AgentToolbarInlineEditor {
    type Action = AgentToolbarInlineEditorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            Self::Action::Chip(chip_action) => {
                let should_save = self.chip_configurator.handle_action(chip_action, ctx);
                if should_save {
                    self.save_current_selection(ctx);
                }
                ctx.notify();
            }
            Self::Action::ResetDefault => {
                open_default_toolbar_items(&mut self.chip_configurator, self.mode, ctx);
                self.save_current_selection(ctx);
                ctx.notify();
            }
            Self::Action::Activate => {
                // no-op — used as the on_click for chip bank items
            }
        }
    }
}

impl View for AgentToolbarInlineEditor {
    fn ui_name() -> &'static str {
        "AgentToolbarInlineEditor"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        render_chip_editor_sections(
            &self.chip_configurator,
            ChipEditorSectionsConfig {
                available_section_label: "Available chips",
                is_at_defaults: self.is_at_defaults(),
                reset_action: AgentToolbarInlineEditorAction::ResetDefault,
                activate_action: AgentToolbarInlineEditorAction::Activate,
                chip_action_wrapper: AgentToolbarInlineEditorAction::Chip,
                mouse_handles: &self.mouse_handles,
            },
            appearance,
        )
    }
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        AgentToolbarEditorAction::Cancel,
        id!(AgentToolbarEditorModal::ui_name()),
    )]);
}

fn save_toolbar_selection<V: View>(
    mode: AgentToolbarEditorMode,
    left: Vec<AgentToolbarItemKind>,
    right: Vec<AgentToolbarItemKind>,
    ctx: &mut ViewContext<V>,
) {
    let is_default = toolbar_items_match_defaults(mode, &left, &right);
    match mode {
        AgentToolbarEditorMode::AgentView => {
            let selection = if is_default {
                AgentToolbarChipSelection::Default
            } else {
                AgentToolbarChipSelection::Custom { left, right }
            };
            SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings
                    .agent_footer_chip_selection
                    .set_value(selection, ctx));
            });
        }
        AgentToolbarEditorMode::CLIAgent => {
            let selection = if is_default {
                CLIAgentToolbarChipSelection::Default
            } else {
                CLIAgentToolbarChipSelection::Custom { left, right }
            };
            SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                report_if_error!(settings
                    .cli_agent_footer_chip_selection
                    .set_value(selection, ctx));
            });
        }
    }
}

impl AgentToolbarEditorModal {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self {
            mouse_handles: Default::default(),
            chip_configurator: ChipConfigurator::new(ChipConfiguratorLayout::LeftRightZones),
            mode: AgentToolbarEditorMode::default(),
            is_dirty: false,
        }
    }

    pub fn open(&mut self, mode: AgentToolbarEditorMode, ctx: &mut ViewContext<Self>) {
        self.reset();
        self.mode = mode;
        open_toolbar_items_from_settings(&mut self.chip_configurator, mode, ctx);
        ctx.notify();
    }

    fn save_to_settings(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_dirty {
            return;
        }

        let left = self.chip_configurator.left_item_kinds();
        let right = self.chip_configurator.right_item_kinds();
        save_toolbar_selection(self.mode, left, right, ctx);
    }

    fn reset(&mut self) {
        self.chip_configurator.reset();
        self.is_dirty = false;
    }

    fn modal_title(&self) -> &'static str {
        match self.mode {
            AgentToolbarEditorMode::AgentView => AGENT_MODAL_TITLE,
            AgentToolbarEditorMode::CLIAgent => CLI_MODAL_TITLE,
        }
    }
}

impl Entity for AgentToolbarEditorModal {
    type Event = AgentToolbarEditorEvent;
}

impl TypedActionView for AgentToolbarEditorModal {
    type Action = AgentToolbarEditorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            Self::Action::Cancel => {
                self.reset();
                ctx.emit(AgentToolbarEditorEvent::Close);
            }
            Self::Action::Save => {
                self.save_to_settings(ctx);
                ctx.emit(AgentToolbarEditorEvent::Close);
            }
            Self::Action::Chip(chip_action) => {
                let mutated = self.chip_configurator.handle_action(chip_action, ctx);
                if mutated {
                    self.is_dirty = true;
                }
                ctx.notify();
            }
            Self::Action::ResetDefault => {
                self.is_dirty = true;
                open_default_toolbar_items(&mut self.chip_configurator, self.mode, ctx);
                ctx.notify();
            }
            Self::Action::Activate => {
                // no-op — used as the on_click for chip bank items
            }
        }
    }
}

impl AgentToolbarEditorModal {
    fn is_at_defaults(&self) -> bool {
        is_toolbar_editor_at_defaults(self.mode, &self.chip_configurator)
    }
}

impl View for AgentToolbarEditorModal {
    fn ui_name() -> &'static str {
        "AgentToolbarEditorModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        render_chip_editor_modal(
            &self.chip_configurator,
            ChipEditorModalConfig {
                title: self.modal_title(),
                available_section_label: "Available chips",
                is_at_defaults: self.is_at_defaults(),
                is_dirty: self.is_dirty,
                cancel_action: AgentToolbarEditorAction::Cancel,
                save_action: AgentToolbarEditorAction::Save,
                reset_action: AgentToolbarEditorAction::ResetDefault,
                activate_action: AgentToolbarEditorAction::Activate,
                chip_action_wrapper: AgentToolbarEditorAction::Chip,
                mouse_handles: &self.mouse_handles,
            },
            appearance,
        )
    }
}
