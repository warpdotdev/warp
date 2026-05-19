use itertools::Itertools;
use warp_core::ui::theme::Fill;

use pathfinder_geometry::vector::vec2f;
use serde::Serialize;
use warpui::keymap::FixedBinding;
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};

use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::chip_configurator::{ChipConfigurator, ChipConfiguratorAction, ChipConfiguratorLayout};
use crate::context_chips::prompt::{Prompt, PromptConfiguration, PromptSelection};
use crate::context_chips::renderer::Renderer as ContextChipRenderer;
use crate::context_chips::{
    available_chips, ChipAvailability, ChipRuntimeCapabilities, ContextChipKind,
};

use crate::server::telemetry::{PromptChoice, TelemetryEvent};
use crate::settings::{FontSettings, WarpPromptSeparator};
use crate::terminal::blockgrid_element::BlockGridElement;
use crate::terminal::SizeInfo;
use settings::Setting as _;

use crate::terminal::model::blockgrid::BlockGrid;
use crate::terminal::model::ObfuscateSecrets;
use crate::terminal::session_settings::SessionSettings;
use crate::view_components::{Dropdown, DropdownItem};
use crate::Appearance;
use crate::{report_if_error, send_telemetry_from_ctx};
use warpui::elements::{
    Align, Border, ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Empty, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Stack,
};

const MODAL_WIDTH: f32 = 700.;
const BORDER_WIDTH: f32 = 1.;
const MODAL_TITLE_FONT_SIZE: f32 = 16.;
const MODAL_UNIFORM_PADDING: f32 = 24.;
const CORNER_RADIUS_PIXELS: f32 = 8.;
const PRIMARY_BUTTON_HEIGHT: f32 = 40.;
const SECTION_UNIFORM_PADDING: f32 = 24.;

const MARGIN_BETWEEN_MODAL_SECTIONS: f32 = 16.;
const SLP_ROW_BOTTOM_PADDING: f32 = 8.;
const SLP_ROW_TOP_MARGIN: f32 = 8.;
const CHECKBOX_MARGIN_RIGHT: f32 = 4.;
const DROPDOWN_LABEL_MARGIN_LEFT: f32 = 24.;
const DROPDOWN_LABEL_MARGIN_RIGHT: f32 = 4.;
const DROPDOWN_WIDTH: f32 = 72.;

const MODAL_CONTENT_FONT_SIZE: f32 = 14.;
const CHECKBOX_SIZE: f32 = 16.;

const MODAL_TITLE: &str = "Edit prompt";
const WARP_PROMPT_SECTION_HEADER: &str = "Warp terminal prompt";
const SHELL_PROMPT_SECTION_HEADER: &str = "Shell prompt (PS1)";
const RESTORE_DEFAULT_BUTTON: &str = "Restore default";

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        EditorModalAction::Cancel,
        id!(EditorModal::ui_name()),
    )]);
}

#[derive(Copy, Clone, Debug, Serialize)]
pub enum OpenSource {
    AppearancePage,
    CommandPalette,
    InputContextMenu,
}

pub enum EditorModalEvent {
    Close,
}

#[derive(Default)]
struct MouseStateHandles {
    cancel_button_handle: MouseStateHandle,
    save_button_handle: MouseStateHandle,
    restore_default_warp_prompt_handle: MouseStateHandle,
    warp_prompt_mouse_state_handle: MouseStateHandle,
    ps1_mouse_state_handle: MouseStateHandle,
    same_line_prompt_checkbox_state_handle: MouseStateHandle,
}

pub struct EditorModal {
    mouse_state_handles: MouseStateHandles,

    /// The information we need to render the PS1 as a grid.
    ps1_grid_info: Option<(BlockGrid, SizeInfo)>,

    /// Generalized drag/drop chip configurator (single-zone layout).
    chip_configurator: ChipConfigurator,

    /// The currently selected prompt type. Whenever the modal is opened,
    /// this value reflects the most recently saved prompt type. It can
    /// be changed while the modal is opened, and will be used when
    /// saving changes.
    prompt_type: PromptType,

    /// Whether same line prompt is currently enabled or not. Similar
    /// to above, value reflects most recently saved setting when modal
    /// is opened. It can be changed while the modal is opened, and it is
    /// used for saving changes.
    same_line_prompt_enabled: bool,

    /// Dropdown to select the separator for the Warp prompt, in the case of
    /// same line prompt. This separator is added at the end of the Warp prompt.
    warp_prompt_separator_dropdown: ViewHandle<Dropdown<EditorModalAction>>,
    /// The separator currently selected for the Warp prompt.
    warp_prompt_separator: WarpPromptSeparator,

    /// True if there was any change while the modal was open.
    is_dirty: bool,

    chip_runtime_capabilities: ChipRuntimeCapabilities,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PromptType {
    PS1,
    Warp,
    WarpDefault,
}

impl PromptType {
    fn warp_prompt_from_settings(app: &AppContext) -> PromptType {
        let session_settings = SessionSettings::as_ref(app);
        if matches!(*session_settings.saved_prompt, PromptSelection::Default) {
            PromptType::WarpDefault
        } else {
            PromptType::Warp
        }
    }

    fn from_settings(app: &AppContext) -> PromptType {
        let session_settings = SessionSettings::as_ref(app);
        if *session_settings.honor_ps1 {
            PromptType::PS1
        } else {
            Self::warp_prompt_from_settings(app)
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum EditorModalAction {
    Cancel,
    Save,
    Chip(ChipConfiguratorAction),
    UsePS1,
    UseWarpPrompt,
    ResetWarpPrompt,
    ToggleSameLinePrompt,
    SetWarpPromptSeparator { separator: WarpPromptSeparator },
}

impl EditorModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let prompt_type = PromptType::from_settings(ctx);

        let same_line_prompt_enabled = SessionSettings::as_ref(ctx)
            .saved_prompt
            .value()
            .same_line_prompt_enabled();

        let warp_prompt_separator = match SessionSettings::as_ref(ctx).saved_prompt.value() {
            PromptSelection::CustomChipSelection(config) => config.separator(),
            // If the "default Warp prompt" i.e. no context chips, is selected, then default to no Warp prompt separator.
            _ => WarpPromptSeparator::None,
        };
        let warp_prompt_separator_label = warp_prompt_separator.dropdown_item_label().to_owned();

        let warp_prompt_separator_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(DROPDOWN_WIDTH);
            dropdown.set_menu_width(DROPDOWN_WIDTH, ctx);
            let items = vec![
                DropdownItem::new(
                    WarpPromptSeparator::None.dropdown_item_label(),
                    EditorModalAction::SetWarpPromptSeparator {
                        separator: WarpPromptSeparator::None,
                    },
                ),
                DropdownItem::new(
                    WarpPromptSeparator::PercentSign.dropdown_item_label(),
                    EditorModalAction::SetWarpPromptSeparator {
                        separator: WarpPromptSeparator::PercentSign,
                    },
                ),
                DropdownItem::new(
                    WarpPromptSeparator::DollarSign.dropdown_item_label(),
                    EditorModalAction::SetWarpPromptSeparator {
                        separator: WarpPromptSeparator::DollarSign,
                    },
                ),
                DropdownItem::new(
                    WarpPromptSeparator::ChevronSymbol.dropdown_item_label(),
                    EditorModalAction::SetWarpPromptSeparator {
                        separator: WarpPromptSeparator::ChevronSymbol,
                    },
                ),
            ];

            if prompt_type != PromptType::PS1 && same_line_prompt_enabled {
                dropdown.set_enabled(ctx);
            } else {
                dropdown.set_disabled(ctx);
            }

            dropdown.set_items(items, ctx);
            dropdown.set_selected_by_name(warp_prompt_separator_label, ctx);
            dropdown
        });

        Self {
            mouse_state_handles: Default::default(),
            ps1_grid_info: None,
            chip_configurator: ChipConfigurator::new(ChipConfiguratorLayout::SingleZone),
            is_dirty: false,
            prompt_type,
            chip_runtime_capabilities: Default::default(),
            same_line_prompt_enabled,
            warp_prompt_separator_dropdown,
            warp_prompt_separator,
        }
    }

    fn chip_renderer_for_kind(
        &self,
        kind: ContextChipKind,
        appearance: &Appearance,
    ) -> Option<ContextChipRenderer> {
        let availability = kind
            .to_chip()
            .map(|chip| chip.availability(&self.chip_runtime_capabilities))
            .unwrap_or(ChipAvailability::Enabled);
        if matches!(availability, ChipAvailability::Hidden) {
            return None;
        }

        ContextChipRenderer::default_from_kind(kind, availability, appearance)
    }

    fn update_used_chips(&mut self, used_chips: Vec<ContextChipKind>, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let unused_chips = available_chips()
            .into_iter()
            .filter(|kind| !used_chips.contains(kind))
            .filter_map(|kind| self.chip_renderer_for_kind(kind, appearance))
            .collect::<Vec<_>>();

        let used_chips = used_chips
            .into_iter()
            .filter_map(|kind| self.chip_renderer_for_kind(kind, appearance))
            .collect::<Vec<_>>();
        self.chip_configurator
            .open_single_zone_with_renderers(used_chips, unused_chips);
    }

    /// Resets the state to match the most recent saved prompt states.
    /// This API must be used when opening the modal.
    pub fn open(
        &mut self,
        ps1_grid_info: Option<(BlockGrid, SizeInfo)>,
        chip_runtime_capabilities: ChipRuntimeCapabilities,
        ctx: &mut ViewContext<Self>,
    ) {
        // The first thing we should do is just reset the state. We'll populate it accordingly below.
        self.reset();
        self.ps1_grid_info = ps1_grid_info;
        self.chip_runtime_capabilities = chip_runtime_capabilities;

        let used_chips = Prompt::as_ref(ctx).chip_kinds();
        self.update_used_chips(used_chips, ctx);

        self.prompt_type = PromptType::from_settings(ctx);
        self.same_line_prompt_enabled = SessionSettings::as_ref(ctx)
            .saved_prompt
            .value()
            .same_line_prompt_enabled();

        ctx.notify();
    }

    /// Updates the state of the Warp prompt separator dropdown to be enabled/disabled based on the current state of the modal.
    fn update_warp_separator_dropdown_state(&mut self, ctx: &mut ViewContext<Self>) {
        // If we are using the Warp prompt and SLP is enabled, then we enable the dropdown. Otherwise, disable it.
        if self.prompt_type != PromptType::PS1 && self.same_line_prompt_enabled {
            self.warp_prompt_separator_dropdown
                .update(ctx, |dropdown, ctx| {
                    dropdown.set_enabled(ctx);
                });
        } else {
            self.warp_prompt_separator_dropdown
                .update(ctx, |dropdown, ctx| {
                    dropdown.set_disabled(ctx);
                });
        }
    }

    fn save_prompt_to_settings(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_dirty {
            match self.prompt_type {
                PromptType::PS1 => {
                    // TODO: we need to stop the Warp prompt generators from running at this point
                    SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                        report_if_error!(settings.honor_ps1.set_value(true, ctx));
                    });
                }
                PromptType::WarpDefault => {
                    Prompt::handle(ctx).update(ctx, |prompt, ctx| {
                        report_if_error!(prompt.reset(ctx));
                    });
                }
                PromptType::Warp => {
                    let new_setup = self
                        .chip_configurator
                        .used_chips
                        .iter()
                        .filter_map(|r| r.chip_kind().cloned());

                    let session_settings = SessionSettings::as_ref(ctx);
                    let current_same_line_prompt_enabled =
                        session_settings.saved_prompt.same_line_prompt_enabled();
                    if self.same_line_prompt_enabled != current_same_line_prompt_enabled {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::ToggleSameLinePrompt {
                                enabled: self.same_line_prompt_enabled,
                            },
                            ctx
                        );
                    }

                    // Updating the `Prompt` handles turning off PS1.
                    Prompt::handle(ctx).update(ctx, |prompt, ctx| {
                        report_if_error!(prompt.update(
                            new_setup,
                            self.same_line_prompt_enabled,
                            self.warp_prompt_separator,
                            ctx
                        ));
                    });
                }
            }

            let prompt_info = match self.prompt_type {
                PromptType::PS1 => PromptChoice::PS1,
                PromptType::WarpDefault => PromptChoice::Default,
                PromptType::Warp => PromptChoice::Custom {
                    builtin_chips: self
                        .chip_configurator
                        .used_chips
                        .iter()
                        .filter_map(|r| r.chip_kind().and_then(|k| k.telemetry_name()))
                        .collect_vec(),
                },
            };
            send_telemetry_from_ctx!(
                TelemetryEvent::PromptEdited {
                    prompt: prompt_info,
                    entrypoint: "prompt_editor".to_string()
                },
                ctx
            );
        }
    }

    fn reset(&mut self) {
        self.chip_configurator.reset();
        self.ps1_grid_info = None;
        self.is_dirty = false;
    }
}

impl Entity for EditorModal {
    type Event = EditorModalEvent;
}

impl TypedActionView for EditorModal {
    type Action = EditorModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            Self::Action::Cancel => {
                self.reset();
                ctx.emit(EditorModalEvent::Close);
            }
            Self::Action::Save => {
                self.save_prompt_to_settings(ctx);
                ctx.emit(EditorModalEvent::Close);
            }
            Self::Action::Chip(chip_action) => {
                let mutated = self.chip_configurator.handle_action(chip_action, ctx);
                if mutated {
                    self.is_dirty = true;
                    self.prompt_type = PromptType::Warp;
                }
                ctx.notify();
            }
            Self::Action::UsePS1 => {
                self.is_dirty = true;
                self.prompt_type = PromptType::PS1;
                // Disable the Warp separator dropdown (only applies to Warp prompt).
                self.update_warp_separator_dropdown_state(ctx);
                ctx.notify();
            }
            Self::Action::UseWarpPrompt => {
                self.is_dirty = true;
                self.prompt_type = PromptType::warp_prompt_from_settings(ctx);
                // Enable the Warp separator dropdown, if SLP is on.
                self.update_warp_separator_dropdown_state(ctx);
                ctx.notify();
            }
            Self::Action::ResetWarpPrompt => {
                self.is_dirty = true;
                self.prompt_type = PromptType::WarpDefault;

                let default_prompt = PromptConfiguration::default_prompt();
                self.same_line_prompt_enabled = default_prompt.same_line_prompt_enabled();
                self.warp_prompt_separator = default_prompt.separator();
                // Disable the Warp separator dropdown, since SLP is off for the default Warp prompt.
                self.update_warp_separator_dropdown_state(ctx);
                let restored_chips = default_prompt.chip_kinds();
                self.update_used_chips(restored_chips, ctx);
                ctx.notify();
            }
            Self::Action::ToggleSameLinePrompt => {
                self.is_dirty = true;
                self.same_line_prompt_enabled = !self.same_line_prompt_enabled;

                // In case we had previously picked default Warp prompt, but now the user toggled
                // same line prompt - it's no longer the default prompt.
                self.prompt_type = PromptType::Warp;

                self.update_warp_separator_dropdown_state(ctx);
                ctx.notify();
            }
            Self::Action::SetWarpPromptSeparator { separator } => {
                self.is_dirty = true;
                self.warp_prompt_separator = *separator;
                ctx.notify();
            }
        }
        ctx.notify();
    }
}

/// Rendering-specific implementation.
impl EditorModal {
    fn render_ps1_prompt(
        &self,
        prompt_grid: &BlockGrid,
        size_info: &SizeInfo,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let left_padding = size_info.padding_x_px();
        Container::new(
            BlockGridElement::new(
                prompt_grid,
                appearance,
                *FontSettings::as_ref(app).enforce_minimum_contrast,
                ObfuscateSecrets::No,
                *size_info,
                appearance.monospace_font_size(),
            )
            .finish(),
        )
        // Remove the padding that's build into the prompt and then
        // add in a fixed amount of padding (8px).
        .with_padding_left(-left_padding.as_f32() + 8.)
        .finish()
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .span(MODAL_TITLE.to_string())
            .with_style(UiComponentStyles {
                font_size: Some(MODAL_TITLE_FONT_SIZE),
                font_weight: Some(warpui::fonts::Weight::Bold),
                ..Default::default()
            })
            .build()
            .finish()
    }

    fn render_unused_chips(&self, appearance: &Appearance) -> Box<dyn Element> {
        self.chip_configurator.render_unused_chips_bank(
            EditorModalAction::UseWarpPrompt,
            EditorModalAction::Chip,
            appearance,
        )
    }

    fn render_used_chips(&self, appearance: &Appearance) -> Box<dyn Element> {
        self.chip_configurator.render_used_drop_zone(
            EditorModalAction::UseWarpPrompt,
            EditorModalAction::Chip,
            appearance,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_prompt_section(
        &self,
        appearance: &Appearance,
        selected: bool,
        header_row: Box<dyn Element>,
        configuration_row: Option<Box<dyn Element>>,
        body: Box<dyn Element>,
        mouse_state_handle: MouseStateHandle,
        on_click_action: EditorModalAction,
    ) -> Box<dyn Element> {
        let cursor = if selected {
            Cursor::Arrow
        } else {
            Cursor::PointingHand
        };

        Hoverable::new(mouse_state_handle.clone(), |mouse_state| {
            let mut column: Flex = Flex::column().with_child(header_row).with_child(body);
            let mut bottom_padding = SECTION_UNIFORM_PADDING;
            // Any configuration settings specific to the prompt type.
            if let Some(configuration_row) = configuration_row {
                bottom_padding = SLP_ROW_BOTTOM_PADDING;
                column.add_child(
                    Container::new(configuration_row)
                        .with_margin_top(SLP_ROW_TOP_MARGIN)
                        .finish(),
                );
            }
            let background = appearance.theme().surface_2();
            let border_color = if selected {
                appearance.theme().accent()
            } else if mouse_state.is_hovered() {
                appearance.theme().main_text_color(background)
            } else {
                appearance.theme().surface_2()
            };
            let border = Border::all(1.).with_border_fill(border_color);

            Container::new(column.finish())
                .with_border(border)
                .with_padding_top(SECTION_UNIFORM_PADDING)
                .with_padding_right(SECTION_UNIFORM_PADDING)
                .with_padding_bottom(bottom_padding)
                .with_padding_left(SECTION_UNIFORM_PADDING)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS_PIXELS)))
                .with_background(background)
                .finish()
        })
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(on_click_action))
        .with_cursor(cursor)
        .finish()
    }

    fn render_restore_default_warp_prompt_button(
        &self,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let button = Hoverable::new(
            self.mouse_state_handles
                .restore_default_warp_prompt_handle
                .clone(),
            |_state| {
                appearance
                    .ui_builder()
                    .span(RESTORE_DEFAULT_BUTTON.to_string())
                    .with_style(UiComponentStyles {
                        font_size: Some(MODAL_CONTENT_FONT_SIZE),
                        ..Default::default()
                    })
                    .build()
                    .finish()
            },
        )
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(EditorModalAction::ResetWarpPrompt))
        .with_cursor(Cursor::PointingHand);

        if matches!(self.prompt_type, PromptType::WarpDefault) && !self.is_dirty {
            button.disable().finish()
        } else {
            button.finish()
        }
    }

    // TODO: consider supporting SLP with the new Warp prompt.
    #[allow(dead_code)]
    fn render_same_line_prompt_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        let label = appearance
            .ui_builder()
            .span("Same line prompt".to_string())
            .with_style(UiComponentStyles {
                font_size: Some(MODAL_CONTENT_FONT_SIZE),
                ..Default::default()
            })
            .build()
            .finish();

        let mut checkbox = appearance
            .ui_builder()
            .checkbox(
                self.mouse_state_handles
                    .same_line_prompt_checkbox_state_handle
                    .clone(),
                Some(CHECKBOX_SIZE),
            )
            .check(self.same_line_prompt_enabled)
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(EditorModalAction::ToggleSameLinePrompt);
            });

        if self.prompt_type == PromptType::PS1 {
            checkbox = checkbox.disable();
        }

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(label)
                    .with_margin_right(CHECKBOX_MARGIN_RIGHT)
                    .finish(),
            )
            .with_child(checkbox.finish())
            .with_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .span("Separator".to_string())
                        .with_style(UiComponentStyles {
                            font_size: Some(MODAL_CONTENT_FONT_SIZE),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_margin_left(DROPDOWN_LABEL_MARGIN_LEFT)
                .with_margin_right(DROPDOWN_LABEL_MARGIN_RIGHT)
                .finish(),
            )
            .with_child(
                Container::new(ChildView::new(&self.warp_prompt_separator_dropdown).finish())
                    .with_margin_left(DROPDOWN_LABEL_MARGIN_RIGHT)
                    .finish(),
            )
            .finish()
    }

    fn render_warp_prompt_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        let body = Flex::column()
            .with_child(
                Container::new(self.render_unused_chips(appearance))
                    .with_margin_top(10.)
                    .finish(),
            )
            .with_child(
                Container::new(self.render_used_chips(appearance))
                    .with_margin_top(10.)
                    .finish(),
            )
            .finish();

        let header_row = Flex::row()
            .with_child(
                appearance
                    .ui_builder()
                    .span(WARP_PROMPT_SECTION_HEADER.to_string())
                    .with_style(UiComponentStyles {
                        font_size: Some(MODAL_CONTENT_FONT_SIZE),
                        font_weight: Some(warpui::fonts::Weight::Semibold),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_child(self.render_restore_default_warp_prompt_button(appearance))
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .finish();

        self.render_prompt_section(
            appearance,
            matches!(self.prompt_type, PromptType::Warp | PromptType::WarpDefault),
            header_row,
            None,
            body,
            self.mouse_state_handles
                .warp_prompt_mouse_state_handle
                .clone(),
            EditorModalAction::UseWarpPrompt,
        )
    }

    fn render_shell_prompt_section(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        // TODO: we should render something else when the grid info isn't available.
        let body = if let Some((grid, size_info)) = &self.ps1_grid_info {
            let prompt_grid = self.render_ps1_prompt(grid, size_info, appearance, app);
            let clipped: Box<dyn Element> = Clipped::new(prompt_grid).finish();
            Container::new(clipped)
                .with_uniform_padding(5.)
                .with_margin_top(16.)
                .with_background(appearance.theme().surface_3())
                .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS_PIXELS)))
                .finish()
        } else {
            Empty::new().finish()
        };

        let header = appearance
            .ui_builder()
            .span(SHELL_PROMPT_SECTION_HEADER.to_string())
            .with_style(UiComponentStyles {
                font_size: Some(MODAL_CONTENT_FONT_SIZE),
                font_weight: Some(warpui::fonts::Weight::Semibold),
                ..Default::default()
            })
            .build()
            .finish();

        self.render_prompt_section(
            appearance,
            matches!(self.prompt_type, PromptType::PS1),
            header,
            None,
            body,
            self.mouse_state_handles.ps1_mouse_state_handle.clone(),
            EditorModalAction::UsePS1,
        )
    }

    fn render_primary_button(
        &self,
        label: String,
        variant: ButtonVariant,
        disabled: bool,
        mouse_state_handle: MouseStateHandle,
        on_click_action: EditorModalAction,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let padding = Coords {
            top: 10.,
            bottom: 10.,
            right: 140.,
            left: 140.,
        };

        // TODO: the buttons need to resize when the modal is resized.
        let mut button = appearance
            .ui_builder()
            .button(variant, mouse_state_handle)
            .with_text_label(label)
            .with_style(UiComponentStyles {
                padding: Some(padding),
                font_size: Some(MODAL_CONTENT_FONT_SIZE),
                ..Default::default()
            });

        if disabled {
            button = button.disabled();
        }

        button
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(on_click_action))
            .with_cursor(Cursor::PointingHand)
            .finish()
    }

    fn render_buttons(&self, appearance: &Appearance) -> Box<dyn Element> {
        let cancel_button = self.render_primary_button(
            "Cancel".to_string(),
            ButtonVariant::Outlined,
            false,
            self.mouse_state_handles.cancel_button_handle.clone(),
            EditorModalAction::Cancel,
            appearance,
        );

        // We disable the save button in a couple of cases:
        // - there are no changes
        // - the Warp prompt is used but there are no chips selected
        let save_disabled = !self.is_dirty
            || (matches!(self.prompt_type, PromptType::Warp)
                && self.chip_configurator.used_chips.is_empty());
        let save_button = self.render_primary_button(
            "Save changes".to_string(),
            ButtonVariant::Accent,
            save_disabled,
            self.mouse_state_handles.save_button_handle.clone(),
            EditorModalAction::Save,
            appearance,
        );

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(
                ConstrainedBox::new(cancel_button)
                    .with_height(PRIMARY_BUTTON_HEIGHT)
                    .finish(),
            )
            .with_child(
                ConstrainedBox::new(Container::new(save_button).with_margin_left(5.).finish())
                    .with_height(PRIMARY_BUTTON_HEIGHT)
                    .finish(),
            )
            .finish()
    }
}

impl View for EditorModal {
    fn ui_name() -> &'static str {
        "PromptEditorModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(self.render_header(appearance))
                    .with_margin_bottom(MARGIN_BETWEEN_MODAL_SECTIONS)
                    .finish(),
            );

        let modal = Container::new(
            ConstrainedBox::new(
                column
                    .with_child(
                        Container::new(self.render_warp_prompt_section(appearance))
                            .with_margin_bottom(MARGIN_BETWEEN_MODAL_SECTIONS)
                            .finish(),
                    )
                    .with_child(
                        Container::new(self.render_shell_prompt_section(appearance, app))
                            .with_margin_bottom(MARGIN_BETWEEN_MODAL_SECTIONS)
                            .finish(),
                    )
                    .with_child(self.render_buttons(appearance))
                    .finish(),
            )
            .with_max_width(MODAL_WIDTH)
            .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS_PIXELS)))
        .with_border(Border::all(BORDER_WIDTH).with_border_fill(theme.outline()))
        .with_background(appearance.theme().surface_1())
        .with_uniform_padding(MODAL_UNIFORM_PADDING)
        .with_margin_top(35.)
        .finish();

        let mut stack = Stack::new();
        stack.add_positioned_child(
            modal,
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::Center,
                ChildAnchor::Center,
            ),
        );

        Container::new(Align::new(stack.finish()).finish())
            .with_background_color(Fill::blur().into())
            .finish()
    }
}
