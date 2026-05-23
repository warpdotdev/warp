use settings::Setting as _;
use warpui::keymap::FixedBinding;
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::chip_configurator::{
    render_chip_editor_modal, render_chip_editor_sections, ChipConfigurator,
    ChipConfiguratorAction, ChipConfiguratorLayout, ChipEditorModalConfig, ChipEditorMouseHandles,
    ChipEditorSectionsConfig, ConfigurableItem, ControlItemRenderer,
};
use crate::settings::AISettings;
use crate::workspace::header_toolbar_item::HeaderToolbarItemKind;
use crate::workspace::tab_settings::{
    HeaderToolbarChipSelection, TabSettings, TabSettingsChangedEvent,
};
use crate::{report_if_error, Appearance};

const MODAL_TITLE: &str = "Edit toolbar";

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        HeaderToolbarEditorAction::Cancel,
        id!(HeaderToolbarEditorModal::ui_name()),
    )]);
}

pub enum HeaderToolbarEditorEvent {
    Close,
}

pub struct HeaderToolbarEditorModal {
    mouse_handles: ChipEditorMouseHandles,
    chip_configurator: ChipConfigurator,
    is_dirty: bool,
}
pub struct HeaderToolbarInlineEditor {
    mouse_handles: ChipEditorMouseHandles,
    chip_configurator: ChipConfigurator,
}

#[derive(Clone, Copy, Debug)]
pub enum HeaderToolbarEditorAction {
    Cancel,
    Save,
    Chip(ChipConfiguratorAction),
    ResetDefault,
    Activate,
}
#[derive(Clone, Copy, Debug)]
pub enum HeaderToolbarInlineEditorAction {
    Chip(ChipConfiguratorAction),
    ResetDefault,
    Activate,
}

fn open_toolbar_items_from_settings<V: View>(
    chip_configurator: &mut ChipConfigurator,
    ctx: &mut ViewContext<V>,
) {
    let selection = TabSettings::as_ref(ctx)
        .header_toolbar_chip_selection
        .clone();

    open_toolbar_items(
        chip_configurator,
        selection.left_items(),
        selection.right_items(),
        ctx,
    );
}

fn open_toolbar_items<V: View>(
    chip_configurator: &mut ChipConfigurator,
    current_left: Vec<HeaderToolbarItemKind>,
    current_right: Vec<HeaderToolbarItemKind>,
    ctx: &mut ViewContext<V>,
) {
    let used_set: Vec<HeaderToolbarItemKind> = current_left
        .iter()
        .chain(current_right.iter())
        .cloned()
        .collect();

    chip_configurator.reset();
    chip_configurator.left_chips = current_left
        .into_iter()
        .filter(|kind| kind.is_supported(ctx))
        .map(|kind| build_configurable_item(&kind))
        .collect();
    chip_configurator.right_chips = current_right
        .into_iter()
        .filter(|kind| kind.is_supported(ctx))
        .map(|kind| build_configurable_item(&kind))
        .collect();
    chip_configurator.unused_chips = HeaderToolbarItemKind::all_items()
        .into_iter()
        .filter(|kind| !used_set.contains(kind) && kind.is_supported(ctx))
        .map(|kind| build_configurable_item(&kind))
        .collect();
}

fn open_default_toolbar_items<V: View>(
    chip_configurator: &mut ChipConfigurator,
    ctx: &mut ViewContext<V>,
) {
    open_toolbar_items(
        chip_configurator,
        HeaderToolbarItemKind::default_left(),
        HeaderToolbarItemKind::default_right(),
        ctx,
    );
}

fn current_toolbar_items(
    chip_configurator: &ChipConfigurator,
) -> (Vec<HeaderToolbarItemKind>, Vec<HeaderToolbarItemKind>) {
    let left = chip_configurator
        .left_chips
        .iter()
        .filter_map(header_toolbar_item_kind)
        .collect();
    let right = chip_configurator
        .right_chips
        .iter()
        .filter_map(header_toolbar_item_kind)
        .collect();
    (left, right)
}

fn toolbar_items_match_defaults(
    left: &[HeaderToolbarItemKind],
    right: &[HeaderToolbarItemKind],
) -> bool {
    left == HeaderToolbarItemKind::default_left() && right == HeaderToolbarItemKind::default_right()
}

fn is_toolbar_editor_at_defaults(chip_configurator: &ChipConfigurator) -> bool {
    let (left, right) = current_toolbar_items(chip_configurator);
    toolbar_items_match_defaults(&left, &right)
}

fn save_toolbar_selection<V: View>(
    left: Vec<HeaderToolbarItemKind>,
    right: Vec<HeaderToolbarItemKind>,
    ctx: &mut ViewContext<V>,
) {
    sync_show_hide_settings(&left, &right, ctx);

    let selection = if toolbar_items_match_defaults(&left, &right) {
        HeaderToolbarChipSelection::Default
    } else {
        HeaderToolbarChipSelection::Custom { left, right }
    };

    TabSettings::handle(ctx).update(ctx, |settings, ctx| {
        report_if_error!(settings
            .header_toolbar_chip_selection
            .set_value(selection, ctx));
    });
}

fn sync_show_hide_settings<V: View>(
    left: &[HeaderToolbarItemKind],
    right: &[HeaderToolbarItemKind],
    ctx: &mut ViewContext<V>,
) {
    let placed: Vec<&HeaderToolbarItemKind> = left.iter().chain(right.iter()).collect();

    let code_review_placed = placed.contains(&&HeaderToolbarItemKind::CodeReview);
    if *TabSettings::as_ref(ctx).show_code_review_button.value() != code_review_placed {
        TabSettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings
                .show_code_review_button
                .set_value(code_review_placed, ctx));
        });
    }

    let notifications_placed = placed.contains(&&HeaderToolbarItemKind::NotificationsMailbox);
    if *AISettings::as_ref(ctx).show_agent_notifications != notifications_placed {
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings
                .show_agent_notifications
                .set_value(notifications_placed, ctx));
        });
    }
}

impl HeaderToolbarInlineEditor {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let mut editor = Self {
            mouse_handles: Default::default(),
            chip_configurator: ChipConfigurator::new(ChipConfiguratorLayout::LeftRightZones),
        };
        editor.reset_from_settings(ctx);

        ctx.subscribe_to_model(&TabSettings::handle(ctx), |me, _, event, ctx| {
            if matches!(
                event,
                TabSettingsChangedEvent::HeaderToolbarChipSelection { .. }
            ) && me.chip_configurator.current_dragging_state.is_none()
            {
                me.reset_from_settings(ctx);
                ctx.notify();
            }
        });

        editor
    }

    fn reset_from_settings(&mut self, ctx: &mut ViewContext<Self>) {
        open_toolbar_items_from_settings(&mut self.chip_configurator, ctx);
    }

    fn save_current_selection(&self, ctx: &mut ViewContext<Self>) {
        let (left, right) = current_toolbar_items(&self.chip_configurator);
        save_toolbar_selection(left, right, ctx);
    }
}

impl Entity for HeaderToolbarInlineEditor {
    type Event = ();
}

impl TypedActionView for HeaderToolbarInlineEditor {
    type Action = HeaderToolbarInlineEditorAction;

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
                open_default_toolbar_items(&mut self.chip_configurator, ctx);
                self.save_current_selection(ctx);
                ctx.notify();
            }
            Self::Action::Activate => {}
        }
    }
}

impl View for HeaderToolbarInlineEditor {
    fn ui_name() -> &'static str {
        "HeaderToolbarInlineEditor"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        render_chip_editor_sections(
            &self.chip_configurator,
            ChipEditorSectionsConfig {
                available_section_label: "Available items",
                is_at_defaults: is_toolbar_editor_at_defaults(&self.chip_configurator),
                reset_action: HeaderToolbarInlineEditorAction::ResetDefault,
                activate_action: HeaderToolbarInlineEditorAction::Activate,
                chip_action_wrapper: HeaderToolbarInlineEditorAction::Chip,
                mouse_handles: &self.mouse_handles,
            },
            appearance,
        )
    }
}

impl HeaderToolbarEditorModal {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self {
            mouse_handles: Default::default(),
            chip_configurator: ChipConfigurator::new(ChipConfiguratorLayout::LeftRightZones),
            is_dirty: false,
        }
    }

    pub fn open(&mut self, ctx: &mut ViewContext<Self>) {
        self.reset();
        open_toolbar_items_from_settings(&mut self.chip_configurator, ctx);

        ctx.notify();
    }

    fn save_to_settings(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_dirty {
            return;
        }

        let (left, right) = current_toolbar_items(&self.chip_configurator);
        save_toolbar_selection(left, right, ctx);
    }

    fn reset(&mut self) {
        self.chip_configurator.reset();
        self.is_dirty = false;
    }
}

impl Entity for HeaderToolbarEditorModal {
    type Event = HeaderToolbarEditorEvent;
}

impl TypedActionView for HeaderToolbarEditorModal {
    type Action = HeaderToolbarEditorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            Self::Action::Cancel => {
                self.reset();
                ctx.emit(HeaderToolbarEditorEvent::Close);
            }
            Self::Action::Save => {
                self.save_to_settings(ctx);
                ctx.emit(HeaderToolbarEditorEvent::Close);
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
                open_default_toolbar_items(&mut self.chip_configurator, ctx);

                ctx.notify();
            }
            Self::Action::Activate => {}
        }
    }
}

impl HeaderToolbarEditorModal {
    fn is_at_defaults(&self) -> bool {
        is_toolbar_editor_at_defaults(&self.chip_configurator)
    }
}

impl View for HeaderToolbarEditorModal {
    fn ui_name() -> &'static str {
        "HeaderToolbarEditorModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        render_chip_editor_modal(
            &self.chip_configurator,
            ChipEditorModalConfig {
                title: MODAL_TITLE,
                available_section_label: "Available items",
                is_at_defaults: self.is_at_defaults(),
                is_dirty: self.is_dirty,
                cancel_action: HeaderToolbarEditorAction::Cancel,
                save_action: HeaderToolbarEditorAction::Save,
                reset_action: HeaderToolbarEditorAction::ResetDefault,
                activate_action: HeaderToolbarEditorAction::Activate,
                chip_action_wrapper: HeaderToolbarEditorAction::Chip,
                mouse_handles: &self.mouse_handles,
            },
            appearance,
        )
    }
}

fn build_configurable_item(kind: &HeaderToolbarItemKind) -> ConfigurableItem {
    let id = serde_json::to_string(kind).expect("HeaderToolbarItemKind is serializable");
    let renderer =
        ControlItemRenderer::new_with_label_and_icon(kind.display_label().to_string(), kind.icon())
            .with_identifier(id);
    let renderer = match kind {
        HeaderToolbarItemKind::TabsPanel => renderer.non_removable(),
        _ => renderer,
    };
    ConfigurableItem::Control(renderer)
}

fn header_toolbar_item_kind(item: &ConfigurableItem) -> Option<HeaderToolbarItemKind> {
    match item {
        ConfigurableItem::Control(renderer) => {
            let id = renderer.identifier()?;
            serde_json::from_str(id).ok()
        }
        ConfigurableItem::ContextChip(_) => None,
    }
}
