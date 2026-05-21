use std::cmp::Ordering;

use itertools::Itertools;
use pathfinder_color::ColorU;
use warp_core::{features::FeatureFlag, ui::appearance::Appearance};
use warp_editor::editor::NavigationKey;
use warpui::{
    elements::{
        ChildView, ConstrainedBox, Container, CrossAxisAlignment, Fill, Flex, MainAxisAlignment,
        MainAxisSize, ParentElement, Shrinkable,
    },
    text_layout::TextStyle,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element, SingletonEntity as _, ViewContext, ViewHandle,
};

use crate::{
    drive::workflows::{
        workflow_arg_selector::{WorkflowArgSelector, WorkflowArgSelectorStyles},
        workflow_arg_type_helpers::{self, ArgumentTypeEditor},
    },
    editor::{
        EditOrigin, EditorView, Event as EditorEvent, InteractionState,
        PlainTextEditorViewAction as EditorAction,
    },
    pane_group::PaneEvent,
    ui_components::{buttons::icon_button, icons::Icon},
    workflows::workflow::Workflow,
    workspace::WorkspaceAction,
};

use super::alias_argument_selector::{AliasArgumentSelector, AliasArgumentSelectorEvent};

use super::{
    WorkflowAction, WorkflowView, WorkflowViewEvent, BUTTON_BORDER_RADIUS, EDITOR_FONT_SIZE,
    HORIZONTAL_TEXT_INPUT_PADDING, SECTION_SPACING, VERTICAL_TEXT_INPUT_PADDING,
    WORKFLOW_PARAMETER_HIGHLIGHT_COLOR,
};

const ARGUMENT_INPUT_HEIGHT: f32 = 30.;
const ARGUMENT_LABEL_TEXT: &str = "Arguments";
const ARGUMENT_LABEL_HEIGHT: f32 = 20.;
const ARGUMENT_LABEL_MARGIN_BOTTOM: f32 = 5.;
const ARGUMENT_DESCRIPTION_PLACEHOLDER_TEXT: &str = "Description";
const ARGUMENT_ALIAS_DESCRIPTION_PLACEHOLDER_TEXT: &str = "Value (optional)";
const ARGUMENT_DEFAULT_VALUE_PLACEHOLDER_TEXT: &str = "Default value (optional)";
pub const DEFAULT_ARGUMENT_PREFIX: &str = "argument";

/// Width of the argument editor in alias mode.
pub const ALIAS_ARGUMENT_EDITOR_WIDTH: f32 = 300.;

/// Which version of the argument-editing section to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgumentEditorMode {
    /// Edit argument definitions, as part of editing the workflow itself.
    WorkflowDefinition,
    /// Edit argument values for an alias.
    Alias,
    /// Edit argument values to fill out and copy.
    Viewer,
}

pub struct ArgumentEditorRow {
    pub(super) name: String,
    pub(super) description_editor: ViewHandle<EditorView>,
    pub(super) default_value_editor: ViewHandle<EditorView>,
    pub(super) argument_editor: ViewHandle<EditorView>,
    pub arg_type_editor: ViewHandle<WorkflowArgSelector>,
    // The editor for alias arguments.  Can be a text editor or a dropdown.
    pub alias_argument_selector: ViewHandle<AliasArgumentSelector>,
}

impl ArgumentTypeEditor for ArgumentEditorRow {
    fn arg_type_editor(&self) -> &ViewHandle<WorkflowArgSelector> {
        &self.arg_type_editor
    }
}

impl WorkflowView {
    pub(super) fn update_arguments_rows(&mut self, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let ui_font_family = appearance.ui_font_family();

        match self
            .arguments_rows
            .len()
            .cmp(&self.arguments_state.arguments.len())
        {
            Ordering::Equal => {
                self.arguments_state
                    .arguments
                    .iter()
                    .enumerate()
                    .for_each(|(index, argument)| {
                        self.arguments_rows[index].name.clone_from(&argument.name);
                    });
            }
            Ordering::Less | Ordering::Greater => {
                // first, get rid of all rows that have names not present in the updated args state
                let argument_names = self
                    .arguments_state
                    .arguments
                    .iter()
                    .map(|argument| argument.name.clone())
                    .collect::<Vec<_>>();
                self.arguments_rows
                    .retain(|row| argument_names.contains(&row.name));

                // next, go over each item in the args state, and either add a row at this position,
                // or skip over it if we've found a match
                self.arguments_state
                    .arguments
                    .iter()
                    .enumerate()
                    .for_each(|(index, argument)| {
                        // if we reach the end of the state struct and we still
                        // haven't inserted a row OR we find a mismatched name,
                        // we know to add a row at this particular index
                        if index == self.arguments_rows.len()
                            || !argument.name.eq(&self.arguments_rows[index].name)
                        {
                            let description_editor = Self::create_editor_handle(
                                ctx,
                                Some(EDITOR_FONT_SIZE),
                                Some(ui_font_family),
                                Some(ARGUMENT_DESCRIPTION_PLACEHOLDER_TEXT),
                                false, /* vim_keybindings */
                                true,
                                false,
                            );

                            ctx.subscribe_to_view(
                                &description_editor,
                                |me, emitter, event, ctx| {
                                    me.handle_argument_editor_event(emitter, event, ctx);
                                },
                            );

                            let default_value_editor = Self::create_editor_handle(
                                ctx,
                                Some(EDITOR_FONT_SIZE),
                                Some(ui_font_family),
                                Some(ARGUMENT_DEFAULT_VALUE_PLACEHOLDER_TEXT),
                                false, /* vim_keybindings */
                                true,
                                false,
                            );

                            ctx.subscribe_to_view(
                                &default_value_editor,
                                |me, emitter, event, ctx| {
                                    me.handle_argument_editor_event(emitter, event, ctx);
                                },
                            );

                            let argument_editor = Self::create_editor_handle(
                                ctx,
                                Some(EDITOR_FONT_SIZE),
                                Some(ui_font_family),
                                None, // none at first will be updated later
                                false,
                                true,
                                false,
                            );

                            ctx.subscribe_to_view(&argument_editor, |me, emitter, event, ctx| {
                                me.handle_argument_editor_event(emitter, event, ctx);
                            });

                            let arg_type_editor = ctx.add_typed_action_view(|ctx| {
                                WorkflowArgSelector::new(
                                    WorkflowArgSelectorStyles {
                                        editor_padding: Coords {
                                            left: HORIZONTAL_TEXT_INPUT_PADDING,
                                            right: HORIZONTAL_TEXT_INPUT_PADDING,
                                            top: VERTICAL_TEXT_INPUT_PADDING,
                                            bottom: VERTICAL_TEXT_INPUT_PADDING,
                                        },
                                        height: Some(ARGUMENT_INPUT_HEIGHT),
                                        width: None,
                                        dropdown_background: |appearance| {
                                            appearance.theme().surface_2()
                                        },
                                        border_color: |appearance| {
                                            appearance.theme().foreground().with_opacity(20)
                                        },
                                        border_radius: BUTTON_BORDER_RADIUS,
                                    },
                                    &self.all_workflow_enums,
                                    ctx,
                                )
                            });

                            ctx.subscribe_to_view(&arg_type_editor, |me, emitter, event, ctx| {
                                me.handle_type_selector_event(emitter, event, ctx);
                            });

                            let alias_argument_selector =
                                ctx.add_typed_action_view(AliasArgumentSelector::new);

                            ctx.subscribe_to_view(
                                &alias_argument_selector,
                                |me, emitter, event, ctx| {
                                    me.handle_alias_argument_selector_event(emitter, event, ctx);
                                },
                            );

                            self.arguments_rows.insert(
                                index,
                                ArgumentEditorRow {
                                    name: argument.name.clone(),
                                    description_editor,
                                    default_value_editor,
                                    argument_editor,
                                    arg_type_editor,
                                    alias_argument_selector,
                                },
                            );
                        }
                    });
            }
        }
    }

    /// Copy argument information (defaults, types, etc.) into the editors for each argument.
    ///
    /// This assumes that [`Self::update_arguments_rows`] has been called first.
    pub(super) fn load_argument_data(
        &mut self,
        workflow_data: &Workflow,
        ctx: &mut ViewContext<Self>,
    ) {
        workflow_data
            .arguments()
            .iter()
            .enumerate()
            .for_each(|(index, argument)| {
                if let Some(description) = &argument.description {
                    self.arguments_rows[index]
                        .description_editor
                        .update(ctx, |editor, ctx| {
                            editor.set_buffer_text_with_base_buffer(description.as_str(), ctx);
                        });
                }

                self.arguments_rows[index]
                    .arg_type_editor
                    .update(ctx, |selector, ctx| {
                        selector.set_workflow_enums(&self.all_workflow_enums, ctx);
                        workflow_arg_type_helpers::load_argument_into_selector(
                            selector,
                            argument,
                            &mut self.all_workflow_enums,
                            ctx,
                        );
                    });

                if let Some(default_value) = &argument.default_value {
                    self.arguments_rows[index]
                        .default_value_editor
                        .update(ctx, |editor, ctx| {
                            editor.set_buffer_text_with_base_buffer(default_value.as_str(), ctx);
                        });

                    // Argument editor is used in the view mode only. We're updating the
                    // placeholder to reflect the default value of this argument
                    // (when a user hasn't manually changed the argument in view mode).
                    self.arguments_rows[index]
                        .argument_editor
                        .update(ctx, |editor, ctx| {
                            editor.set_placeholder_text(default_value.as_str(), ctx);
                        });
                } else {
                    // Clear the argument editor if there is no default value.
                    self.arguments_rows[index]
                        .argument_editor
                        .update(ctx, |editor, _| {
                            editor.clear_all_placeholder_text();
                        });
                }
            });
    }

    pub(super) fn has_dirty_argument_editor(&self, app: &AppContext) -> bool {
        self.arguments_rows.iter().any(|row| {
            let selector_is_dirty = {
                let editor = row.arg_type_editor.as_ref(app);
                let editor_is_dirty = editor.is_dirty(app);
                let enum_is_dirty = editor
                    .get_selected_enum()
                    .and_then(|id| self.all_workflow_enums.get(&id))
                    .map(|enum_data| enum_data.new_data.is_some())
                    .unwrap_or(false);
                editor_is_dirty || enum_is_dirty
            };

            selector_is_dirty
                || row.default_value_editor.as_ref(app).is_dirty(app)
                || row.description_editor.as_ref(app).is_dirty(app)
        })
    }

    pub(super) fn handle_alias_argument_selector_event(
        &mut self,
        handle: ViewHandle<AliasArgumentSelector>,
        event: &AliasArgumentSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            AliasArgumentSelectorEvent::ValueSet(value) => {
                self.arguments_rows.iter().for_each(|row| {
                    if row.alias_argument_selector == handle
                        && self.alias_bar.as_ref(ctx).has_selected_alias()
                    {
                        self.alias_bar.update(ctx, |bar, ctx| {
                            bar.set_current_argument_value(&row.name, value.clone(), ctx);
                        })
                    }
                });
            }
            AliasArgumentSelectorEvent::Navigate(NavigationKey::Tab) => {
                self.arguments_rows
                    .iter()
                    .enumerate()
                    .for_each(|(index, row)| {
                        if row.alias_argument_selector == handle {
                            // If there's another row, tab to its alias argument selector.
                            if let Some(next_row) = self
                                .arguments_rows
                                .get(index + 1)
                                .or(self.arguments_rows.first())
                            {
                                ctx.focus(&next_row.alias_argument_selector)
                            }
                        }
                    });
            }
            AliasArgumentSelectorEvent::Navigate(NavigationKey::ShiftTab) => {
                self.arguments_rows
                    .iter()
                    .enumerate()
                    .for_each(|(index, row)| {
                        if row.alias_argument_selector == handle {
                            // If there's a previous row, tab to its argument editor.
                            let previous_row = match index {
                                0 => self.arguments_rows.last(),
                                _ => self.arguments_rows.get(index - 1),
                            };
                            if let Some(previous_row) = previous_row {
                                ctx.focus(&previous_row.alias_argument_selector)
                            }
                        }
                    });
            }
            _ => {}
        }
    }

    /// Handle an event from one of the argument definition editors.
    pub(super) fn handle_argument_editor_event(
        &mut self,
        handle: ViewHandle<EditorView>,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            // because the number of editor views we have depends on how many arguments
            // are in the command or query, tabbing/shift-tabbing is slightly complex.
            // `handle_argument_editor_event` is used for all of these views, so broadly
            // speaking there are two steps for each interaction:
            // 1. iterate through every row, looking for which editor fired this event
            // 2. decide what editor to focus next based on what editor's ahead/behind us
            EditorEvent::Navigate(NavigationKey::Tab) => {
                self.arguments_rows
                    .iter()
                    .enumerate()
                    .for_each(|(index, row)| {
                        // tabbing in a description editor just means we focus
                        // the corresponding default value editor
                        if row.description_editor == handle {
                            ctx.focus(&row.arg_type_editor);
                        } else if row.default_value_editor == handle {
                            // if we have another row ahead of us, tabbing in the default
                            // value editor moves to the following row's description editor.
                            // otherwise, it wraps around to the title.
                            match self.arguments_rows.get(index + 1) {
                                Some(next_row) => ctx.focus(&next_row.description_editor),
                                None => ctx.focus(&self.name_editor),
                            }
                        } else if row.argument_editor == handle {
                            // If there's another row, tab to its argument editor.
                            if let Some(next_row) = self
                                .arguments_rows
                                .get(index + 1)
                                .or(self.arguments_rows.first())
                            {
                                ctx.focus(&next_row.argument_editor)
                            }
                        }
                    });
            }
            EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                self.arguments_rows
                    .iter()
                    .enumerate()
                    .for_each(|(index, row)| {
                        // if we have another row behind us, shift-tabbing in the description
                        // editor moves to the previous row's default value editor.
                        // otherwise, it focuses the content editor.
                        if row.description_editor == handle {
                            if index == 0 {
                                ctx.focus(&self.content_editor);
                            } else {
                                ctx.focus(&self.arguments_rows[index - 1].arg_type_editor);
                            }
                        // shift-tabbing in a default value editor just means we
                        // focus the corresponding default value editor
                        } else if row.default_value_editor == handle {
                            ctx.focus(&row.description_editor);
                        } else if row.argument_editor == handle {
                            // If there's a previous row, tab to its argument editor.
                            let previous_row = match index {
                                0 => self.arguments_rows.last(),
                                _ => self.arguments_rows.get(index - 1),
                            };
                            if let Some(previous_row) = previous_row {
                                ctx.focus(&previous_row.argument_editor)
                            }
                        }
                    });
            }
            EditorEvent::Edited(origin) => {
                self.arguments_rows.iter().for_each(|row| {
                    if row.argument_editor == handle {
                        let mut updated_args = handle.as_ref(ctx).buffer_text(ctx);

                        if self.alias_bar.as_ref(ctx).has_selected_alias() {
                            // When switching between aliases, we repopulate all the argument
                            // editors - don't count that as an edit to the alias.
                            if *origin != EditOrigin::SystemEdit {
                                self.alias_bar.update(ctx, |bar, ctx| {
                                    bar.set_current_argument_value(&row.name, updated_args, ctx);
                                })
                            }
                        } else {
                            // if we don't have anything filled use the default arguments
                            if updated_args.is_empty() {
                                updated_args =
                                    row.default_value_editor.as_ref(ctx).buffer_text(ctx);
                            }

                            // if there are no default arguments use the argument name
                            if updated_args.is_empty() {
                                updated_args.clone_from(&row.name);
                            }

                            self.command_display_data
                                .set_argument_value(row.name.clone(), updated_args);

                            let text_style_ranges = self
                                .command_display_data
                                .argument_ranges()
                                .into_iter()
                                .map(|range| {
                                    (
                                        range,
                                        TextStyle::new().with_background_color(ColorU::from_u32(
                                            WORKFLOW_PARAMETER_HIGHLIGHT_COLOR,
                                        )),
                                    )
                                })
                                .collect_vec();

                            self.view_only_content_editor.update(ctx, |editor, ctx| {
                                // first make it editable so we can make changes
                                editor.set_interaction_state(InteractionState::Editable, ctx);
                                editor.clear_buffer(ctx);

                                editor.insert_with_styles(
                                    self.command_display_data.to_command_string().as_str(),
                                    //&updated_ranges,
                                    &text_style_ranges,
                                    EditorAction::SystemInsert,
                                    ctx,
                                );

                                // once done revert to being selectable only
                                editor.set_interaction_state(InteractionState::Selectable, ctx);
                            });

                            if !self.is_for_agent_mode {
                                // debounce the syntax highlighting change to avoid flicker per
                                // keystroke and only do the highlighting when the editing has ended.
                                // The flicker would occur because we replace the buffer above with
                                // insert_with_styles for capturing arguments changes and then perform
                                // the syntax highlighting here.
                                self.view_only_content_editor_highlight_model.update(
                                    ctx,
                                    |model, _ctx| {
                                        model.debounce_highlight();
                                    },
                                );
                            }
                        }
                    }
                });
                ctx.notify();
            }
            EditorEvent::Activate => {
                ctx.emit(WorkflowViewEvent::Pane(PaneEvent::FocusSelf));
            }
            _ => {}
        }
    }

    /// Render the arguments area.
    pub(super) fn render_arguments_section(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let mode = if self.alias_bar.as_ref(app).has_selected_alias() {
            ArgumentEditorMode::Alias
        } else if self.is_editable() {
            ArgumentEditorMode::WorkflowDefinition
        } else {
            ArgumentEditorMode::Viewer
        };

        // If there are no arguments to fill out in view mode, don't show the arguments section.
        if mode == ArgumentEditorMode::Viewer && self.arguments_rows.is_empty() {
            return None;
        }

        let mut arguments_section = Flex::column();
        arguments_section.add_child(self.render_arguments_section_header(appearance, app));

        match mode {
            ArgumentEditorMode::WorkflowDefinition | ArgumentEditorMode::Viewer => {
                arguments_section.add_child(self.render_arguments_editors(appearance))
            }
            ArgumentEditorMode::Alias => {
                arguments_section.add_child(self.render_alias_arguments(appearance, app));
            }
        }

        if FeatureFlag::WorkflowAliases.is_enabled()
            && matches!(
                mode,
                ArgumentEditorMode::WorkflowDefinition | ArgumentEditorMode::Alias
            )
            && !self.is_for_agent_mode
        {
            arguments_section.add_child(self.render_env_vars_selector(appearance, app));
        }

        Some(arguments_section.finish())
    }

    fn render_arguments_section_header(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut arguments_section_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        arguments_section_row.add_child(
            Shrinkable::new(
                2.,
                self.render_section_header(
                    crate::i18n::tr_static(app, ARGUMENT_LABEL_TEXT),
                    appearance,
                ),
            )
            .finish(),
        );

        let theme = appearance.theme();
        let sub_text_color = theme.sub_text_color(theme.background()).into_solid();

        if self.is_editable() {
            let ui_builder = appearance.ui_builder().clone();
            let add_argument_tooltip =
                crate::i18n::tr_static(app, "Add a workflow argument").to_string();
            arguments_section_row.add_child(
                icon_button(
                    appearance,
                    Icon::Plus,
                    false,
                    self.ui_state_handles.add_variable_state.clone(),
                )
                .with_tooltip(move || {
                    ui_builder
                        .tool_tip(add_argument_tooltip.clone())
                        .build()
                        .finish()
                })
                .build()
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(WorkflowAction::AddArgument))
                .finish(),
            )
        } else {
            arguments_section_row.add_child(Shrinkable::new(
                    1.,
                    Container::new(
                        appearance
                        .ui_builder()
                        .span(crate::i18n::tr_static(app, "Fill out the arguments in this workflow and copy it to run in your terminal session"))
                        .with_soft_wrap()
                        .with_style(UiComponentStyles {
                            font_size: Some(EDITOR_FONT_SIZE),
                            font_color: Some(sub_text_color),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                    )
                    .with_margin_left(40.)
                    .finish()
                )
                .finish()
                );
        }

        arguments_section_row.finish()
    }

    fn render_arguments_editors(&self, appearance: &Appearance) -> Box<dyn Element> {
        let children: Vec<Box<dyn Element>> = self
            .arguments_state
            .arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                let description_handle = &self.arguments_rows[index].description_editor;
                let argument_handle = &self.arguments_rows[index].argument_editor;

                let text_span = appearance
                    .ui_builder()
                    .span(argument.name.clone())
                    .with_style(UiComponentStyles {
                        font_family_id: Some(appearance.monospace_font_family()),
                        font_size: Some(14.),
                        ..Default::default()
                    })
                    .build()
                    .finish();

                let bg = Fill::from(ColorU::from(appearance.theme().subshell_background()));
                let mut description_container = appearance
                    .ui_builder()
                    .text_input(description_handle.clone());
                description_container = if self.is_editable() {
                    description_container.with_style(UiComponentStyles {
                        padding: Some(Coords {
                            left: HORIZONTAL_TEXT_INPUT_PADDING,
                            right: HORIZONTAL_TEXT_INPUT_PADDING,
                            top: VERTICAL_TEXT_INPUT_PADDING,
                            bottom: VERTICAL_TEXT_INPUT_PADDING,
                        }),
                        ..Default::default()
                    })
                } else {
                    description_container.with_style(UiComponentStyles {
                        padding: Some(Coords {
                            left: HORIZONTAL_TEXT_INPUT_PADDING,
                            right: HORIZONTAL_TEXT_INPUT_PADDING,
                            top: VERTICAL_TEXT_INPUT_PADDING,
                            bottom: VERTICAL_TEXT_INPUT_PADDING,
                        }),
                        background: Some(bg),
                        ..Default::default()
                    })
                };

                let description_input = ConstrainedBox::new(description_container.build().finish())
                    .with_height(ARGUMENT_INPUT_HEIGHT)
                    .finish();

                let input = if self.is_editable() {
                    let arg_type_selector_handle = &self.arguments_rows[index].arg_type_editor;
                    Container::new(ChildView::new(arg_type_selector_handle).finish()).finish()
                } else {
                    ConstrainedBox::new(
                        appearance
                            .ui_builder()
                            .text_input(argument_handle.clone())
                            .with_style(UiComponentStyles {
                                padding: Some(Coords {
                                    left: HORIZONTAL_TEXT_INPUT_PADDING,
                                    right: HORIZONTAL_TEXT_INPUT_PADDING,
                                    top: VERTICAL_TEXT_INPUT_PADDING,
                                    bottom: VERTICAL_TEXT_INPUT_PADDING,
                                }),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .with_height(ARGUMENT_INPUT_HEIGHT)
                    .finish()
                };

                let argument_inputs = ConstrainedBox::new(
                    Flex::row()
                        .with_child(
                            Shrinkable::new(1., Container::new(description_input).finish())
                                .finish(),
                        )
                        .with_child(
                            Shrinkable::new(
                                1.,
                                Container::new(input).with_margin_left(8.).finish(),
                            )
                            .finish(),
                        )
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .finish(),
                )
                .finish();

                let mut column = Flex::column();

                // only show the argument name above if we are in edit mode
                column.add_child(
                    Container::new(
                        ConstrainedBox::new(text_span)
                            .with_min_height(ARGUMENT_LABEL_HEIGHT)
                            .finish(),
                    )
                    .with_margin_bottom(ARGUMENT_LABEL_MARGIN_BOTTOM)
                    .finish(),
                );
                column.add_child(argument_inputs);

                Container::new(column.finish())
                    .with_margin_bottom(SECTION_SPACING)
                    .finish()
            })
            .collect();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_children(children)
            .finish()
    }

    /// Render editors for filling out arguments in an alias.
    fn render_alias_arguments(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut arguments = Flex::column();
        let theme = appearance.theme();

        for (index, argument) in self.arguments_state.arguments.iter().enumerate() {
            let name = appearance
                .ui_builder()
                .span(argument.name.clone())
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.monospace_font_family()),
                    font_size: Some(14.),
                    ..Default::default()
                })
                .build()
                .with_margin_bottom(8.)
                .finish();
            arguments.add_child(name);

            let mut current_description = self.arguments_rows[index]
                .description_editor
                .as_ref(app)
                .buffer_text(app);

            let mut styles = UiComponentStyles {
                font_size: Some(13.),
                ..Default::default()
            };

            // If the description is empty, show a placeholder text.
            if current_description.is_empty() {
                current_description.push_str(ARGUMENT_ALIAS_DESCRIPTION_PLACEHOLDER_TEXT);
                styles.font_color = Some(theme.sub_text_color(theme.background()).into_solid());
            }

            let description = appearance
                .ui_builder()
                .span(current_description)
                .with_style(styles)
                .build()
                .with_horizontal_padding(12.)
                .with_vertical_padding(5.)
                .finish();

            let value =
                ChildView::new(&self.arguments_rows[index].alias_argument_selector).finish();

            arguments.add_child(
                Container::new(
                    Flex::row()
                        .with_children([description, Shrinkable::new(1., value).finish()])
                        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .finish(),
                )
                .with_margin_bottom(8.)
                .finish(),
            )
        }

        arguments.finish()
    }

    fn render_env_vars_selector(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let action_element = if self.env_vars_selector.as_ref(app).has_env_vars(app) {
            Shrinkable::new(1., ChildView::new(&self.env_vars_selector).finish()).finish()
        } else {
            appearance
                .ui_builder()
                .button(
                    ButtonVariant::Secondary,
                    self.ui_state_handles
                        .add_environment_variables_mouse_state
                        .clone(),
                )
                .with_centered_text_label(
                    crate::i18n::tr_static(app, "Add environment variables").to_string(),
                )
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(WorkspaceAction::CreatePersonalEnvVarCollection);
                })
                .finish()
        };

        Flex::row()
            .with_children([
                appearance
                    .ui_builder()
                    .span(crate::i18n::tr_static(app, "Environment variables"))
                    .with_style(UiComponentStyles {
                        font_size: Some(13.),
                        ..Default::default()
                    })
                    .build()
                    .with_margin_right(8.)
                    .finish(),
                action_element,
            ])
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish()
    }
}
