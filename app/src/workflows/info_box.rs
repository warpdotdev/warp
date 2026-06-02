use std::collections::HashMap;
use std::ops::Range;

use string_offset::CharOffset;
use warp_core::features::FeatureFlag;
use warp_core::settings::Setting;
use warpui::color::ColorU;
use warpui::elements::{
    self, Align, Border, Clipped, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, DropShadow, Flex, Highlight, Icon,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Rect, Shrinkable,
    Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::geometry::vector::Vector2F;
use warpui::keymap::Keystroke;
use warpui::presenter::ChildView;
use warpui::text_layout::{ClipConfig, TextStyle};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, EventContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::command_parser::{
    compute_workflow_display_data, WorkflowArgumentIndex, WorkflowDisplayData,
};
use super::workflow::Argument;
use super::workflow_view::env_var_selector::{EnvVarSelector, EnvVarSelectorEvent};
use super::{AIWorkflowOrigin, CloudWorkflow};
use crate::ai::blocklist::ai_brand_color;
use crate::appearance::Appearance;
use crate::cloud_object::model::actions::{ObjectActionType, ObjectActions};
use crate::cloud_object::CloudObjectMetadataExt;
use crate::server::ids::SyncId;
use crate::settings::InputModeSettings;
use crate::terminal::block_list_viewport::InputMode;
use crate::terminal::input::InputAction;
use crate::terminal::view::TerminalAction;
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons;
use crate::util::color::coloru_with_opacity;
use crate::view_components::FilterableDropdownOrientation;
use crate::workflows::WorkflowType;
use crate::workspace::WorkspaceAction;

const INFO_BOX_PADDING: f32 = 20.;
const ARGUMENT_PADDING: f32 = 10.;
const KEYBOARD_SHORTCUT_PADDING: f32 = 15.;

const COLLAPSED_BUTTON_VERTICAL_PADDING: f32 = 5.;
const COLLAPSED_BUTTON_HORIZONTAL_PADDING: f32 = 9.;

/// Environment variables row
const ENV_VAR_SPAN_FONT_SIZE: f32 = 14.;
const ENV_VAR_ROW_HEIGHT: f32 = 50.;
const ENV_VAR_DROPDOWN_WIDTH: f32 = 225.;
const ENV_VAR_HORIZONTAL_MARGIN: f32 = 20.;
const ENV_VAR_RIGHT_ELEMENT_VERTICAL_MARGIN: f32 = 5.;
const ENV_VAR_SPAN_VERTICAL_MARGIN: f32 = 15.;
const ENV_VAR_BUTTON_HEIGHT: f32 = 30.;
const ENV_VAR_SPAN: &str = "Environment variables";
const NEW_ENV_VAR_BUTTON_LABEL: &str = "New environment variables";

/// Scale factor the title should be from the user's current font size.
const TITLE_FONT_SIZE_SCALE_FACTOR: f32 = 1.12;

const VERTICAL_DIVIDER_THICKNESS: f32 = 2.;

pub const WORKFLOW_PARAMETER_HIGHLIGHT_COLOR: u32 = 0x42C0FA4D;

/// State necessary for rendering workflows and their arguments when a workflow is selected.
pub struct SelectedWorkflowState {
    currently_selected_argument: WorkflowArgumentIndex,
    num_arguments: WorkflowArgumentIndex,
    argument_cycling_enabled: bool,
}

enum WrapText {
    Yes,
    No,
}

impl SelectedWorkflowState {
    pub fn increment_argument_index(&mut self) {
        if *self.num_arguments > 0 && self.argument_cycling_enabled {
            self.currently_selected_argument =
                ((*self.currently_selected_argument + 1) % *self.num_arguments).into()
        }
    }

    pub fn set_argument_index(&mut self, index: WorkflowArgumentIndex) {
        if *index < *self.num_arguments {
            self.currently_selected_argument = index;
        } else {
            log::error!(
                "Tried to set the argument index to {:?} but the len is {:?}",
                *index,
                *self.num_arguments
            );
        }
    }

    pub fn currently_selected_argument(&self) -> WorkflowArgumentIndex {
        self.currently_selected_argument
    }

    pub fn set_argument_cycling_enabled(&mut self, new_val: bool) {
        self.argument_cycling_enabled = new_val;
    }
}

pub struct WorkflowsMoreInfoView {
    workflow: WorkflowType,
    /// The workflow command with the argument identifiers replaced with the actual argument.
    command_with_replaced_arguments: String,
    /// Map from workflow argument index to the character indices within the command after the arguments
    /// identifiers have been replaced with the actual arguments.
    argument_index_to_char_range_map: HashMap<WorkflowArgumentIndex, Vec<Range<CharOffset>>>,
    button_mouse_states: ButtonMouseStates,
    pub info_box_expanded: bool,
    pub selected_workflow_state: SelectedWorkflowState,

    /// When false, we want to remove the subpanel that explains the shift-tab UX for moving between arguments.
    pub show_shift_tab_treatment: bool,

    /// View for selecting environment variables to apply to the workflow.
    ///
    /// This is `None` for AI workflows.
    environment_variables_dropdown: Option<ViewHandle<EnvVarSelector>>,

    scroll_state: ClippedScrollStateHandle,
}

#[derive(Default)]
struct ButtonMouseStates {
    close: MouseStateHandle,
    collapse: MouseStateHandle,
    view_context: MouseStateHandle,
    save_as_workflow: MouseStateHandle,
    edit_cloud_workflow: MouseStateHandle,
    reset_command: MouseStateHandle,
    add_env_var_collection: MouseStateHandle,
}

impl WorkflowsMoreInfoView {
    pub fn new(
        info_box_expanded: bool,
        workflow: WorkflowType,
        show_shift_tab_treatment: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let num_arguments = workflow.as_workflow().arguments().len();

        let WorkflowDisplayData {
            command_with_replaced_arguments,
            argument_index_to_char_range_map,
            ..
        } = compute_workflow_display_data(workflow.as_workflow());

        let environment_variables_dropdown = (!workflow.as_workflow().is_agent_mode_workflow())
            .then(|| {
                let dropdown = ctx.add_typed_action_view(|ctx| {
                    let mut dropdown = EnvVarSelector::new(ctx);
                    dropdown.set_orientation(FilterableDropdownOrientation::Up, ctx);
                    dropdown.set_width(ENV_VAR_DROPDOWN_WIDTH, ctx);
                    dropdown
                });
                ctx.subscribe_to_view(&dropdown, |me, _, event, ctx| {
                    me.handle_env_var_selector_event(event, ctx);
                });
                dropdown
            });

        Self {
            workflow,
            command_with_replaced_arguments,
            argument_index_to_char_range_map,
            button_mouse_states: Default::default(),
            info_box_expanded,
            selected_workflow_state: SelectedWorkflowState {
                currently_selected_argument: 0.into(),
                num_arguments: num_arguments.into(),
                argument_cycling_enabled: true,
            },
            show_shift_tab_treatment,
            environment_variables_dropdown,
            scroll_state: Default::default(),
        }
    }

    /// Gets the currently selected argument if it exists and the parameter
    /// tabbing has been enabled. Returns none if the user has deleted a parameter
    /// highlight and therefore disabled argument cycling.
    pub fn selected_argument(&self) -> Option<&Argument> {
        if !self.selected_workflow_state.argument_cycling_enabled {
            return None;
        }
        let workflow = self.workflow.as_workflow();
        workflow
            .arguments()
            .get(*self.selected_workflow_state.currently_selected_argument)
    }

    pub fn set_environment_variables_selection(
        &mut self,
        env_vars_id: Option<SyncId>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(dropdown) = self.environment_variables_dropdown.as_ref() {
            dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_selected_env_vars(env_vars_id, ctx)
            });
        }
    }

    fn handle_env_var_selector_event(
        &mut self,
        event: &EnvVarSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EnvVarSelectorEvent::SelectionChanged(id) => {
                ctx.emit(WorkflowsInfoBoxViewEvent::PrefixCommandWithEnvironmentVariables(*id));
            }
            EnvVarSelectorEvent::Refreshed => ctx.notify(),
        }
    }

    fn render_collapse_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let icon = if self.info_box_expanded {
            icons::Icon::ChevronDown
        } else {
            icons::Icon::ChevronUp
        };

        render_hoverable_card_button(
            icon,
            None,
            self.button_mouse_states.collapse.clone(),
            |ctx, _, _| {
                ctx.dispatch_typed_action(WorkflowsInfoBoxViewAction::CollapseOrExpand);
            },
            appearance,
        )
    }

    fn render_edit_button(
        &self,
        cloud_workflow: &CloudWorkflow,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let label = if cloud_workflow.model().data.is_agent_mode_workflow() {
            "Edit prompt"
        } else {
            "Edit workflow"
        };
        let workflow = cloud_workflow.clone();
        render_hoverable_card_button(
            icons::Icon::Rename,
            Some(label.to_owned()),
            self.button_mouse_states.edit_cloud_workflow.clone(),
            move |ctx: &mut warpui::EventContext<'_>, _, _| {
                ctx.dispatch_typed_action(TerminalAction::OpenWorkflowModalWithCloudWorkflow(
                    workflow.id,
                ))
            },
            appearance,
        )
    }

    fn render_collapsed_info_box(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut title_and_arg_container = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.,
                    Container::new(Self::render_workflow_title(self, WrapText::No, appearance))
                        .with_padding_left(INFO_BOX_PADDING)
                        .finish(),
                )
                .finish(),
            );

        if let Some(arg) = self.selected_argument() {
            title_and_arg_container.add_child(
                Shrinkable::new(
                    1.,
                    Container::new(self.render_argument_and_description(
                        arg,
                        false,
                        WrapText::No,
                        appearance,
                    ))
                    .with_padding_left(INFO_BOX_PADDING)
                    .finish(),
                )
                .finish(),
            );
        }

        let collapsed_info_box = Flex::row()
            .with_children([
                Shrinkable::new(1., title_and_arg_container.finish()).finish(),
                Container::new(self.render_collapse_button(appearance))
                    .with_padding_left(COLLAPSED_BUTTON_HORIZONTAL_PADDING)
                    .finish(),
                Container::new(self.render_close_workflow_button(appearance))
                    .with_padding_right(COLLAPSED_BUTTON_HORIZONTAL_PADDING)
                    .finish(),
            ])
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish();

        Container::new(collapsed_info_box)
            .with_padding_top(COLLAPSED_BUTTON_VERTICAL_PADDING)
            .with_padding_bottom(COLLAPSED_BUTTON_VERTICAL_PADDING)
            .with_background(appearance.theme().surface_2())
            .finish()
    }

    fn render_argument_and_description(
        &self,
        arg: &Argument,
        is_selected: bool,
        wrap_text: WrapText,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let title_string = match (arg.name(), arg.description()) {
            (name, Some(description)) => format!("{name}: {description}"),
            (name, None) => name.to_string(),
        };

        let mut highlight =
            Highlight::new().with_properties(Properties::default().weight(Weight::Bold));
        if is_selected {
            highlight = highlight.with_text_style(
                TextStyle::new()
                    .with_background_color(ColorU::from_u32(WORKFLOW_PARAMETER_HIGHLIGHT_COLOR)),
            )
        }
        appearance
            .ui_builder()
            .wrappable_text(title_string, matches!(wrap_text, WrapText::Yes))
            .with_style(UiComponentStyles {
                font_family_id: Some(if self.workflow.as_workflow().is_agent_mode_workflow() {
                    appearance.ui_font_family()
                } else {
                    appearance.monospace_font_family()
                }),
                font_color: Some(
                    appearance
                        .theme()
                        .sub_text_color(appearance.theme().surface_2())
                        .into(),
                ),
                font_size: Some(appearance.monospace_font_size()),
                ..Default::default()
            })
            .with_highlights((0..arg.name.len()).collect::<Vec<_>>(), highlight)
            .build()
            .finish()
    }

    fn render_workflow_description_and_arguments(
        &self,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut flex_column = Flex::column().with_main_axis_alignment(MainAxisAlignment::Center);
        let workflow = self.workflow.as_workflow();

        if let Some(description) = workflow.description() {
            flex_column.add_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .paragraph(description.clone())
                        .with_style(UiComponentStyles {
                            font_family_id: Some(appearance.ui_font_family()),
                            font_color: Some(
                                appearance
                                    .theme()
                                    .sub_text_color(appearance.theme().surface_2())
                                    .into(),
                            ),
                            font_size: Some(appearance.monospace_font_size()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_padding_bottom(ARGUMENT_PADDING)
                .finish(),
            )
        }

        flex_column.add_children(workflow.arguments().iter().enumerate().map(|(index, arg)| {
            let is_selected = WorkflowArgumentIndex::from(index)
                == self.selected_workflow_state.currently_selected_argument
                && self.selected_workflow_state.argument_cycling_enabled
                && self.show_shift_tab_treatment;
            Container::new(self.render_argument_and_description(
                arg,
                is_selected,
                WrapText::Yes,
                appearance,
            ))
            .with_padding_bottom(ARGUMENT_PADDING)
            .finish()
        }));

        flex_column.finish()
    }

    fn render_command_edited_menu(&self, appearance: &Appearance) -> Box<dyn Element> {
        let contents = Flex::row()
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        icons::Icon::AlertCircle
                            .to_warpui_icon(
                                appearance
                                    .theme()
                                    .sub_text_color(appearance.theme().surface_2()),
                            )
                            .finish(),
                    )
                    .with_height(16.)
                    .with_width(16.)
                    .finish(),
                )
                .with_margin_right(8.)
                .finish(),
            )
            .with_child(
                Container::new(
                    Text::new_inline(
                        "Command edited.",
                        appearance.ui_font_family(),
                        appearance.monospace_font_size(),
                    )
                    .with_color(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().surface_2())
                            .into(),
                    )
                    .finish(),
                )
                .with_margin_right(16.)
                .finish(),
            )
            .with_child(
                appearance
                    .ui_builder()
                    .button(
                        ButtonVariant::Text,
                        self.button_mouse_states.reset_command.clone(),
                    )
                    .with_centered_text_label(String::from("Reset"))
                    .with_style(UiComponentStyles {
                        font_family_id: Some(appearance.ui_font_family()),
                        font_size: Some(appearance.monospace_font_size()),
                        font_weight: Some(Weight::Bold),
                        ..Default::default()
                    })
                    .build()
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(InputAction::ResetWorkflowState)
                    })
                    .finish(),
            )
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);

        Container::new(Shrinkable::new(1., contents.finish()).finish())
            .with_background(appearance.theme().dark_overlay())
            .with_padding_left(INFO_BOX_PADDING)
            .with_padding_right(INFO_BOX_PADDING)
            .with_padding_top(ARGUMENT_PADDING)
            .with_padding_bottom(ARGUMENT_PADDING)
            .finish()
    }

    fn render_keyboard_shortcut_menu(&self, appearance: &Appearance) -> Box<dyn Element> {
        let cycle_parameter_text = Flex::row()
            .with_child(
                appearance
                    .ui_builder()
                    .keyboard_shortcut(&Keystroke {
                        key: "Tab".to_string(),
                        shift: true,
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_child(
                Shrinkable::new(
                    1.,
                    Container::new(
                        Text::new_inline(
                            "to cycle parameters",
                            appearance.ui_font_family(),
                            appearance.monospace_font_size(),
                        )
                        .with_color(
                            appearance
                                .theme()
                                .main_text_color(appearance.theme().surface_2())
                                .into(),
                        )
                        .finish(),
                    )
                    .with_padding_left(10.)
                    .finish(),
                )
                .finish(),
            )
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .finish();

        Container::new(Shrinkable::new(1., cycle_parameter_text).finish())
            .with_background(appearance.theme().surface_2())
            .with_padding_left(KEYBOARD_SHORTCUT_PADDING)
            .with_padding_right(KEYBOARD_SHORTCUT_PADDING)
            .with_padding_top(ARGUMENT_PADDING)
            .with_padding_bottom(ARGUMENT_PADDING)
            .finish()
    }

    fn render_save_workflow_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let workflow = self.workflow.as_workflow().to_owned();
        render_hoverable_card_button(
            icons::Icon::Workflow,
            Some("Save as workflow".to_string()),
            self.button_mouse_states.save_as_workflow.clone(),
            move |ctx, _, _| {
                ctx.dispatch_typed_action(TerminalAction::OpenWorkflowModalForAIWorkflow(
                    workflow.clone(),
                ));
            },
            appearance,
        )
    }

    fn render_close_workflow_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        render_hoverable_card_button(
            icons::Icon::X,
            None,
            self.button_mouse_states.close.clone(),
            |ctx, _, _| {
                ctx.dispatch_typed_action(InputAction::HideWorkflowInfoCard);
            },
            appearance,
        )
    }

    fn render_environment_variables_selection(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let span = Container::new(
            Align::new(
                appearance
                    .ui_builder()
                    .span(ENV_VAR_SPAN.to_string())
                    .with_style(UiComponentStyles {
                        font_size: Some(ENV_VAR_SPAN_FONT_SIZE),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .left()
            .finish(),
        )
        .with_vertical_margin(ENV_VAR_SPAN_VERTICAL_MARGIN)
        .with_margin_right(ENV_VAR_HORIZONTAL_MARGIN)
        .finish();

        let environment_variables_dropdown = self.environment_variables_dropdown.as_ref()?;
        let dropdown_element = if environment_variables_dropdown.as_ref(app).has_env_vars(app) {
            ChildView::new(environment_variables_dropdown).finish()
        } else {
            Align::new(
                ConstrainedBox::new(
                    appearance
                        .ui_builder()
                        .button(
                            ButtonVariant::Secondary,
                            self.button_mouse_states.add_env_var_collection.clone(),
                        )
                        .with_centered_text_label(NEW_ENV_VAR_BUTTON_LABEL.to_owned())
                        .build()
                        .on_click(|ctx, _, _| {
                            // Create envvars in personal drive for max extensibility (can be moved
                            // to any team/workspace)
                            ctx.dispatch_typed_action(
                                WorkspaceAction::CreatePersonalEnvVarCollection,
                            )
                        })
                        .finish(),
                )
                .with_height(ENV_VAR_BUTTON_HEIGHT)
                .finish(),
            )
            .finish()
        };

        let env_var_dropdown = Container::new(dropdown_element)
            .with_vertical_margin(ENV_VAR_RIGHT_ELEMENT_VERTICAL_MARGIN)
            .finish();

        Some(
            ConstrainedBox::new(
                Stack::new()
                    .with_child(
                        Rect::new()
                            .with_background_color(appearance.theme().surface_1().into())
                            .finish(),
                    )
                    .with_child(
                        Container::new(
                            Flex::row()
                                .with_main_axis_size(MainAxisSize::Max)
                                .with_child(span)
                                .with_child(env_var_dropdown)
                                .finish(),
                        )
                        .with_horizontal_margin(ENV_VAR_HORIZONTAL_MARGIN)
                        .finish(),
                    )
                    .finish(),
            )
            .with_height(ENV_VAR_ROW_HEIGHT)
            .finish(),
        )
    }

    fn render_info_box(
        &self,
        appearance: &Appearance,
        input_mode: &InputMode,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let content_and_args = self.render_content_and_arguments(appearance);

        let workflow = self.workflow.as_workflow();
        let mut title_line = vec![WorkflowsMoreInfoView::render_workflow_title(
            self,
            WrapText::Yes,
            appearance,
        )];

        if let Some(workflow_source) = workflow.source_url() {
            if !workflow_source.is_empty() {
                title_line.push(WorkflowsMoreInfoView::render_workflow_source(
                    self,
                    workflow_source.to_string(),
                    appearance,
                ));
            }
        }

        let collapse_button = self.render_collapse_button(appearance);

        let close_button = self.render_close_workflow_button(appearance);

        let mut row_content = Flex::row();

        match &self.workflow {
            WorkflowType::Cloud(cloud_workflow) => {
                let editing_history = cloud_workflow.metadata.semantic_editing_history(app);

                let action_history = ObjectActions::as_ref(app)
                    .get_action_history_summary_for_action_type(
                        &cloud_workflow.id.uid(),
                        ObjectActionType::Execute,
                    );

                let full_object_history_text = match (editing_history, action_history) {
                    (Some(edits), Some(actions)) => Some(format!("{edits}  |  {actions}")),
                    (Some(edits), None) => Some(edits),
                    _ => None,
                };

                let metadata_history = full_object_history_text.map(|str| {
                    Container::new(
                        Text::new_inline(str, appearance.ui_font_family(), 12.)
                            .with_color(
                                appearance
                                    .theme()
                                    .sub_text_color(appearance.theme().surface_2())
                                    .into(),
                            )
                            .with_clip(ClipConfig::end())
                            .finish(),
                    )
                    .with_uniform_padding(5.)
                    .finish()
                });

                if let Some(metadata_history_element) = metadata_history {
                    row_content.add_child(Shrinkable::new(1., metadata_history_element).finish());
                }

                let edit_button = self.render_edit_button(cloud_workflow, appearance);
                row_content.add_children([edit_button, collapse_button, close_button]);
            }
            WorkflowType::AIGenerated { .. } => {
                let save_as_workflow_button = self.render_save_workflow_button(appearance);
                row_content.add_children([save_as_workflow_button, collapse_button, close_button]);
            }
            _ => row_content.add_children([collapse_button, close_button]),
        };

        let workflow_info = Flex::column()
            .with_children([
                Container::new(
                    Clipped::new(
                        Flex::row()
                            .with_children([
                                Container::new(
                                    Flex::row()
                                        .with_children(title_line)
                                        .with_cross_axis_alignment(CrossAxisAlignment::End)
                                        .finish(),
                                )
                                .with_padding_top(INFO_BOX_PADDING)
                                .finish(),
                                Shrinkable::new(
                                    1.,
                                    Container::new(row_content.finish())
                                        .with_padding_top(COLLAPSED_BUTTON_VERTICAL_PADDING)
                                        .with_padding_right(COLLAPSED_BUTTON_HORIZONTAL_PADDING)
                                        .with_padding_left(COLLAPSED_BUTTON_HORIZONTAL_PADDING)
                                        .finish(),
                                )
                                .finish(),
                            ])
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                            .finish(),
                    )
                    .finish(),
                )
                .finish(),
                Container::new(content_and_args)
                    .with_padding_top(5.)
                    .finish(),
            ])
            .finish();

        let workflow_container = Flex::row()
            .with_child(
                Shrinkable::new(
                    1.,
                    Container::new(workflow_info)
                        .with_padding_left(INFO_BOX_PADDING)
                        .with_padding_bottom(INFO_BOX_PADDING)
                        .finish(),
                )
                .finish(),
            )
            .finish();

        let mut children = vec![workflow_container];

        if self.workflow.should_show_env_var_selection() {
            if let Some(environment_variables_selection) =
                self.render_environment_variables_selection(appearance, app)
            {
                children.push(Clipped::new(environment_variables_selection).finish());
            }
        }

        if !self.show_shift_tab_treatment {
            children.push(self.render_command_edited_menu(appearance));
        } else if !workflow.arguments().is_empty() {
            children.push(self.render_keyboard_shortcut_menu(appearance));
        }

        match input_mode {
            InputMode::PinnedToBottom => Flex::column().with_children(children).finish(),
            InputMode::PinnedToTop | InputMode::Waterfall => Flex::column()
                .with_children(children.into_iter().rev())
                .finish(),
        }
    }

    fn render_content(&self, appearance: &Appearance) -> Box<dyn Element> {
        let selected_argument = self.selected_workflow_state.currently_selected_argument;

        let (font_family, font_size) = if self.workflow.as_workflow().is_agent_mode_workflow() {
            (
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
        } else {
            (
                appearance.monospace_font_family(),
                appearance.monospace_font_size() * 0.85,
            )
        };

        let mut content_text = appearance
            .ui_builder()
            .paragraph(self.command_with_replaced_arguments.clone())
            .with_style(UiComponentStyles {
                font_family_id: Some(font_family),
                font_color: Some(
                    appearance
                        .theme()
                        .sub_text_color(appearance.theme().surface_2())
                        .into(),
                ),
                font_size: Some(font_size),
                ..Default::default()
            });

        // Don't highlight arguments if the shift-tab UX is unavailable
        if self.show_shift_tab_treatment {
            self.argument_index_to_char_range_map
                .iter()
                .for_each(|(argument_index, ranges)| {
                    ranges.iter().for_each(|range| {
                        let highlight_color = ColorU::from_u32(WORKFLOW_PARAMETER_HIGHLIGHT_COLOR);
                        let background_color = if selected_argument == *argument_index
                            && self.selected_workflow_state.argument_cycling_enabled
                        {
                            highlight_color
                        } else {
                            coloru_with_opacity(highlight_color, 40)
                        };

                        content_text.add_highlight(
                            (range.start.as_usize()..range.end.as_usize()).collect(),
                            Highlight::new().with_text_style(
                                TextStyle::new().with_background_color(background_color),
                            ),
                        );
                    });
                });
        }

        content_text.build().finish()
    }

    /// Renders the "content" of the workflow (command for Workflow::Command, query for Workflow::AgentMode)
    /// with its interleaved arguments.
    fn render_content_and_arguments(&self, appearance: &Appearance) -> Box<dyn Element> {
        let content = self.render_content(appearance);

        let vertical_divider = WorkflowsMoreInfoView::render_vertical_divider(appearance);

        let mut workflow_info = Flex::row()
            .with_child(Shrinkable::new(1., Align::new(content).left().finish()).finish())
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        let workflow = self.workflow.as_workflow();
        if !workflow.arguments().is_empty() || workflow.description().is_some() {
            workflow_info.add_children([
                Shrinkable::new(1., vertical_divider).finish(),
                Shrinkable::new(
                    1.,
                    self.render_workflow_description_and_arguments(appearance),
                )
                .finish(),
            ])
        }

        workflow_info.finish()
    }

    fn render_vertical_divider(appearance: &Appearance) -> Box<dyn Element> {
        // Create a box with a fixed size width. Within that box, create a center-aligned divider
        // with a width of `VERTICAL_DIVIDER_THICKNESS`. This creates a divider that has equal
        // padding based on the overall width of the divider.
        ConstrainedBox::new(
            Align::new(
                ConstrainedBox::new(
                    Rect::new()
                        .with_border(
                            Border::left(VERTICAL_DIVIDER_THICKNESS)
                                .with_border_fill(appearance.theme().accent()),
                        )
                        .finish(),
                )
                .with_max_height(100.)
                .with_width(VERTICAL_DIVIDER_THICKNESS)
                .finish(),
            )
            .finish(),
        )
        .with_width(40.)
        .finish()
    }

    fn render_workflow_title(
        &self,
        wrap_text: WrapText,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        match &self.workflow {
            WorkflowType::AIGenerated {
                workflow,
                origin: source,
            } => {
                let icon = if FeatureFlag::AgentMode.is_enabled() {
                    match source {
                        AIWorkflowOrigin::AgentMode => {
                            Icon::new(icons::Icon::Prompt.into(), appearance.theme().accent())
                                .finish()
                        }
                        _ => Icon::new(
                            icons::Icon::Prompt.into(),
                            ai_brand_color(appearance.theme()),
                        )
                        .finish(),
                    }
                } else {
                    Icon::new(
                        icons::Icon::AiAssistant.into(),
                        appearance
                            .theme()
                            .main_text_color(appearance.theme().background()),
                    )
                    .finish()
                };

                let ai_icon = Container::new(
                    ConstrainedBox::new(icon)
                        .with_width(16.)
                        .with_height(16.)
                        .finish(),
                )
                .with_margin_right(8.)
                .finish();

                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_children([
                        ai_icon,
                        appearance
                            .ui_builder()
                            .wrappable_text(
                                workflow.name().to_owned(),
                                matches!(wrap_text, WrapText::Yes),
                            )
                            .with_style(UiComponentStyles {
                                font_family_id: Some(appearance.ui_font_family()),
                                font_color: Some(
                                    appearance
                                        .theme()
                                        .main_text_color(appearance.theme().background())
                                        .into(),
                                ),
                                font_size: Some(
                                    appearance.monospace_font_size() * TITLE_FONT_SIZE_SCALE_FACTOR,
                                ),
                                font_weight: Some(Weight::Bold),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    ])
                    .finish()
            }
            _ => appearance
                .ui_builder()
                .wrappable_text(
                    self.workflow.as_workflow().name().to_owned(),
                    matches!(wrap_text, WrapText::Yes),
                )
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_color: Some(
                        appearance
                            .theme()
                            .main_text_color(appearance.theme().background())
                            .into(),
                    ),
                    font_size: Some(
                        appearance.monospace_font_size() * TITLE_FONT_SIZE_SCALE_FACTOR,
                    ),
                    font_weight: Some(Weight::Bold),
                    ..Default::default()
                })
                .build()
                .finish(),
        }
    }

    fn render_workflow_source(
        &self,
        workflow_source: String,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            appearance
                .ui_builder()
                .link(
                    "View Context".into(),
                    Some(workflow_source),
                    None,
                    self.button_mouse_states.view_context.clone(),
                )
                .soft_wrap(false)
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_color: Some(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().background())
                            .into(),
                    ),
                    font_size: Some(appearance.monospace_font_size()),
                    font_weight: Some(Weight::Normal),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_padding_left(10.)
        .finish()
    }
}

fn render_hoverable_card_button<F>(
    icon_type: icons::Icon,
    tool_tip_text: Option<String>,
    mouse_state_handle: MouseStateHandle,
    on_click: F,
    appearance: &Appearance,
) -> Box<dyn Element>
where
    F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
{
    let ui_builder = appearance.ui_builder().clone();
    let mut button = icon_button(appearance, icon_type, false, mouse_state_handle.clone());

    if let Some(tool_tip_text) = tool_tip_text {
        button = button.with_tooltip(move || ui_builder.tool_tip(tool_tip_text).build().finish());
    }

    Container::new(button.build().on_click(on_click).finish())
        .with_margin_left(2.)
        .with_margin_right(2.)
        .finish()
}

#[derive(Debug)]
pub enum WorkflowsInfoBoxViewEvent {
    PrefixCommandWithEnvironmentVariables(Option<SyncId>),
}

#[derive(Debug, Clone)]
pub enum WorkflowsInfoBoxViewAction {
    CollapseOrExpand,
    SelectEnvironmentVariables(Option<SyncId>),
}

impl Entity for WorkflowsMoreInfoView {
    type Event = WorkflowsInfoBoxViewEvent;
}

impl TypedActionView for WorkflowsMoreInfoView {
    type Action = WorkflowsInfoBoxViewAction;

    fn handle_action(&mut self, action: &WorkflowsInfoBoxViewAction, ctx: &mut ViewContext<Self>) {
        match action {
            WorkflowsInfoBoxViewAction::CollapseOrExpand => {
                self.info_box_expanded = !self.info_box_expanded
            }
            WorkflowsInfoBoxViewAction::SelectEnvironmentVariables(env_vars) => ctx
                .emit(WorkflowsInfoBoxViewEvent::PrefixCommandWithEnvironmentVariables(*env_vars)),
        }
    }
}

impl View for WorkflowsMoreInfoView {
    fn ui_name() -> &'static str {
        "WorkflowsInfoBoxView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let input_mode = InputModeSettings::as_ref(app).input_mode.value();

        let info_box_element = if self.info_box_expanded {
            self.render_info_box(appearance, input_mode, app)
        } else {
            self.render_collapsed_info_box(appearance)
        };

        Container::new(
            ClippedScrollable::vertical(
                self.scroll_state.clone(),
                Container::new(info_box_element)
                    .with_border(Border::all(1.).with_border_fill(appearance.theme().surface_2()))
                    .with_background(appearance.theme().surface_2())
                    .with_corner_radius(match input_mode {
                        InputMode::PinnedToBottom => CornerRadius::with_top(Radius::Pixels(6.)),
                        InputMode::PinnedToTop | InputMode::Waterfall => {
                            CornerRadius::with_bottom(Radius::Pixels(6.))
                        }
                    })
                    .finish(),
                Default::default(),
                theme.disabled_text_color(theme.background()).into(),
                theme.main_text_color(theme.background()).into(),
                elements::Fill::None,
            )
            .finish(),
        )
        .with_drop_shadow(DropShadow::new_with_standard_offset_and_spread(
            ColorU::new(0, 0, 0, 48),
        ))
        .finish()
    }
}
