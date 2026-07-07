use warp_editor::editor::NavigationKey;
use warpui::elements::{
    Align, ConstrainedBox, Container, Flex, ParentElement, SavePosition, Shrinkable, Stack,
};
use warpui::fonts::FamilyId;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, ViewContext, ViewHandle};

use crate::editor::{
    EditOrigin, EditorOptions, EditorView, Event as EditorEvent, InteractionState,
    PropagateAndNoOpNavigationKeys, SingleLineEditorOptions, TextOptions,
};
use crate::env_vars::active_env_var_collection_data::SavingStatus;
use crate::env_vars::view::env_var_collection::{
    EditorType, EnvVarCollectionView, DESCRIPTION_EDITOR_POSITION, ROW_SPACING,
};
use crate::env_vars::EnvVarValue;
use crate::Appearance;

// Metadata labels (name and description)
const LABEL_FONT_SIZE: f32 = 12.;
const METADATA_SPACING: f32 = 8.;
const LAST_ROW_ELEMENT_SPACING: f32 = 2.;
const TITLE_LABEL_TEXT: &str = "Title";
const DESCRIPTION_LABEL_TEXT: &str = "Description";

const VERTICAL_TEXT_INPUT_PADDING: f32 = 5.;
const HORIZONTAL_TEXT_INPUT_PADDING: f32 = 10.;
const SECRET_ICON_BUTTON_MARGIN: f32 = 2.;

impl EnvVarCollectionView {
    pub(super) fn create_editor_handle(
        ctx: &mut ViewContext<Self>,
        font_size_override: Option<f32>,
        font_family_override: Option<FamilyId>,
        placeholder_text: Option<&str>,
        single_line: bool,
    ) -> ViewHandle<EditorView> {
        let text = TextOptions {
            font_size_override,
            font_family_override,
            ..Default::default()
        };
        ctx.add_typed_action_view(|ctx| {
            let mut editor = if single_line {
                EditorView::single_line(
                    SingleLineEditorOptions {
                        text,
                        propagate_and_no_op_vertical_navigation_keys:
                            PropagateAndNoOpNavigationKeys::Always,
                        ..Default::default()
                    },
                    ctx,
                )
            } else {
                EditorView::new(
                    EditorOptions {
                        text,
                        soft_wrap: true,
                        autogrow: true,
                        propagate_and_no_op_vertical_navigation_keys:
                            PropagateAndNoOpNavigationKeys::Always,
                        supports_vim_mode: false,
                        single_line: false,
                        ..Default::default()
                    },
                    ctx,
                )
            };

            if let Some(text) = placeholder_text {
                editor.set_placeholder_text(text, ctx);
            }

            editor
        })
    }

    pub(super) fn editors_are_empty(&self, app: &AppContext) -> bool {
        self.variable_rows.iter().any(|row| {
            row.variable_name_editor.as_ref(app).is_empty(app)
                || (row.variable_value_editor.as_ref(app).is_empty(app)
                    && matches!(row.value, EnvVarValue::Constant(_)))
        })
    }

    pub(super) fn handle_title_editor_event(
        &mut self,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                if self.variable_rows.is_empty() {
                    ctx.focus(&self.description_editor);
                } else if let Some(variable_row) = self.variable_rows.last() {
                    ctx.focus(&variable_row.variable_description_editor);
                }
            }
            EditorEvent::Navigate(NavigationKey::Tab) => {
                ctx.focus(&self.description_editor);
            }
            EditorEvent::ClearParentSelections => {
                self.clear_parent_selections(self.title_editor.clone(), ctx)
            }
            EditorEvent::Edited(EditOrigin::UserInitiated)
            | EditorEvent::Edited(EditOrigin::UserTyped) => {
                self.set_saving_status(SavingStatus::Unsaved, ctx);

                let current_text = self.title_editor.as_ref(ctx).buffer_text(ctx);
                self.update_title_validation(&current_text, ctx);
            }
            _ => {}
        }
    }

    pub(super) fn handle_description_editor_event(
        &mut self,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                ctx.focus(&self.title_editor);
            }
            EditorEvent::Navigate(NavigationKey::Tab) => {
                if self.variable_rows.is_empty() {
                    ctx.focus(&self.title_editor);
                } else if let Some(variable_row) = self.variable_rows.first() {
                    ctx.focus(&variable_row.variable_name_editor);
                }
            }
            EditorEvent::ClearParentSelections => {
                self.clear_parent_selections(self.description_editor.clone(), ctx)
            }
            EditorEvent::Edited(EditOrigin::UserInitiated)
            | EditorEvent::Edited(EditOrigin::UserTyped) => {
                self.set_saving_status(SavingStatus::Unsaved, ctx);

                let current_text = self.description_editor.as_ref(ctx).buffer_text(ctx);
                self.update_description_validation(&current_text, ctx);
            }
            _ => {}
        }
    }

    pub(super) fn handle_variable_event(
        &mut self,
        handle: ViewHandle<EditorView>,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                self.focus_prev_variable_editor(handle, ctx)
            }
            EditorEvent::Navigate(NavigationKey::Tab) => {
                self.focus_next_variable_editor(handle, ctx)
            }
            EditorEvent::ClearParentSelections => self.clear_parent_selections(handle.clone(), ctx),
            EditorEvent::Edited(EditOrigin::UserInitiated)
            | EditorEvent::Edited(EditOrigin::UserTyped) => {
                self.set_saving_status(SavingStatus::Unsaved, ctx);

                if let Some((row_index, field_type)) = self.find_editor_info(&handle) {
                    let current_text = handle.as_ref(ctx).buffer_text(ctx);
                    self.update_field_validation(row_index, field_type, &current_text, ctx);
                }

                ctx.notify()
            }
            _ => {}
        }
    }

    fn clear_parent_selections(
        &mut self,
        editor: ViewHandle<EditorView>,
        ctx: &mut ViewContext<Self>,
    ) {
        if editor != self.title_editor {
            self.title_editor.update(ctx, |editor, ctx| {
                editor.clear_selections(ctx);
            })
        }

        if editor != self.description_editor {
            self.description_editor.update(ctx, |editor, ctx| {
                editor.clear_selections(ctx);
            })
        }

        self.variable_rows.iter().for_each(|var_editor| {
            if var_editor.variable_name_editor != editor {
                var_editor
                    .variable_name_editor
                    .update(ctx, |var_editor, ctx| {
                        var_editor.clear_selections(ctx);
                    });
            }

            if var_editor.variable_value_editor != editor {
                var_editor
                    .variable_value_editor
                    .update(ctx, |var_editor, ctx| {
                        var_editor.clear_selections(ctx);
                    });
            }

            if var_editor.variable_description_editor != editor {
                var_editor
                    .variable_description_editor
                    .update(ctx, |var_editor, ctx| {
                        var_editor.clear_selections(ctx);
                    });
            }
        })
    }

    fn focus_next_variable_editor(
        &self,
        handle: ViewHandle<EditorView>,
        ctx: &mut ViewContext<Self>,
    ) {
        let editor = self
            .variable_rows
            .iter()
            .enumerate()
            .find_map(|(index, editor)| {
                if editor.variable_name_editor == handle {
                    Some((index, EditorType::Name))
                } else if editor.variable_value_editor == handle {
                    Some((index, EditorType::Value))
                } else if editor.variable_description_editor == handle {
                    Some((index, EditorType::Description))
                } else {
                    None
                }
            });

        match editor {
            Some((index, EditorType::Name)) => {
                if let Some(variable_row) = self.variable_rows.get(index) {
                    if let EnvVarValue::Constant(_) = variable_row.value {
                        ctx.focus(&variable_row.variable_value_editor);
                    } else {
                        ctx.focus(&variable_row.variable_description_editor);
                    }
                }
            }
            Some((index, EditorType::Value)) => {
                if let Some(variable_row) = self.variable_rows.get(index) {
                    ctx.focus(&variable_row.variable_description_editor);
                }
            }
            Some((index, EditorType::Description)) => {
                if index == self.variable_rows.len() - 1 {
                    ctx.focus(&self.title_editor)
                } else if let Some(next_variable_row) = self.variable_rows.get(index + 1) {
                    ctx.focus(&next_variable_row.variable_name_editor)
                }
            }
            _ => {}
        };
    }

    fn focus_prev_variable_editor(
        &self,
        handle: ViewHandle<EditorView>,
        ctx: &mut ViewContext<Self>,
    ) {
        let editor = self
            .variable_rows
            .iter()
            .enumerate()
            .find_map(|(index, editor)| {
                if editor.variable_name_editor == handle {
                    Some((index, EditorType::Name))
                } else if editor.variable_value_editor == handle {
                    Some((index, EditorType::Value))
                } else if editor.variable_description_editor == handle {
                    Some((index, EditorType::Description))
                } else {
                    None
                }
            });

        match editor {
            Some((index, EditorType::Name)) => {
                if index == 0 {
                    ctx.focus(&self.description_editor)
                } else if let Some(prev_variable_row) = self.variable_rows.get(index - 1) {
                    ctx.focus(&prev_variable_row.variable_description_editor);
                }
            }
            Some((index, EditorType::Value)) => {
                if let Some(variable_row) = self.variable_rows.get(index) {
                    ctx.focus(&variable_row.variable_name_editor);
                }
            }
            Some((index, EditorType::Description)) => {
                if let Some(variable_row) = self.variable_rows.get(index) {
                    if let EnvVarValue::Constant(_) = variable_row.value {
                        ctx.focus(&variable_row.variable_value_editor);
                    } else {
                        ctx.focus(&variable_row.variable_name_editor);
                    }
                }
            }
            _ => {}
        };
    }

    fn render_metadata_label<S>(&self, text: S, appearance: &Appearance) -> Box<dyn Element>
    where
        S: Into<String>,
    {
        appearance
            .ui_builder()
            .span(text.into())
            .with_style(UiComponentStyles {
                font_size: Some(LABEL_FONT_SIZE),
                ..Default::default()
            })
            .build()
            .finish()
    }

    fn render_metadata_editor(
        &self,
        appearance: &Appearance,
        editor: ViewHandle<EditorView>,
        has_error: bool,
    ) -> Box<dyn Element> {
        let mut style = UiComponentStyles {
            padding: Some(Coords {
                left: HORIZONTAL_TEXT_INPUT_PADDING,
                right: HORIZONTAL_TEXT_INPUT_PADDING,
                top: VERTICAL_TEXT_INPUT_PADDING,
                bottom: VERTICAL_TEXT_INPUT_PADDING,
            }),
            ..Default::default()
        };

        if has_error {
            let error_color = appearance.theme().ui_error_color();
            style.border_color = Some(error_color.into());
            style.border_width = Some(super::env_var_collection::ERROR_BORDER_WIDTH);
        }

        appearance
            .ui_builder()
            .text_input(editor.clone())
            .with_style(style)
            .build()
            .finish()
    }

    // "Metadata" references the object level title and description fields
    pub(super) fn render_metadata(&self, appearance: &Appearance) -> Box<dyn Element> {
        let title_has_error = self.form_validation_state.title_error.is_some();
        let description_has_error = self.form_validation_state.description_error.is_some();

        Flex::column()
            .with_child(
                Container::new(self.render_metadata_label(TITLE_LABEL_TEXT, appearance))
                    .with_margin_bottom(METADATA_SPACING)
                    .finish(),
            )
            .with_child(
                Container::new(self.render_metadata_editor(
                    appearance,
                    self.title_editor.clone(),
                    title_has_error,
                ))
                .with_margin_bottom(METADATA_SPACING)
                .finish(),
            )
            .with_child(
                SavePosition::new(
                    Container::new(self.render_metadata_label(DESCRIPTION_LABEL_TEXT, appearance))
                        .with_margin_bottom(METADATA_SPACING)
                        .finish(),
                    DESCRIPTION_EDITOR_POSITION,
                )
                .finish(),
            )
            .with_child(
                Container::new(self.render_metadata_editor(
                    appearance,
                    self.description_editor.clone(),
                    description_has_error,
                ))
                .finish(),
            )
            .finish()
    }

    pub(super) fn render_variable_editor(
        &self,
        appearance: &Appearance,
        editor: ViewHandle<EditorView>,
        editor_type: EditorType,
        inline_secret_button: Option<Box<dyn Element>>,
        row_index: Option<usize>,
    ) -> Box<dyn Element> {
        let margin_right = if editor_type != EditorType::Description {
            ROW_SPACING
        } else {
            LAST_ROW_ELEMENT_SPACING
        };

        let validation_error = if let Some(index) = row_index {
            self.variable_rows
                .get(index)
                .and_then(|row| row.validation_state.get_field_error(editor_type))
        } else {
            None
        };

        let text_input = {
            let mut style = UiComponentStyles {
                padding: Some(Coords {
                    left: HORIZONTAL_TEXT_INPUT_PADDING,
                    right: HORIZONTAL_TEXT_INPUT_PADDING,
                    top: VERTICAL_TEXT_INPUT_PADDING,
                    bottom: VERTICAL_TEXT_INPUT_PADDING,
                }),
                ..Default::default()
            };

            if validation_error.is_some() {
                let error_color = appearance.theme().ui_error_color();
                style.border_color = Some(error_color.into());
                style.border_width = Some(super::env_var_collection::ERROR_BORDER_WIDTH);
            }

            appearance
                .ui_builder()
                .text_input(editor.clone())
                .with_style(style)
                .build()
                .finish()
        };

        let input_container = {
            let mut stack = Stack::new().with_child(text_input);

            if let Some(element) = inline_secret_button {
                stack.add_child(
                    Align::new(
                        Container::new(element)
                            .with_margin_right(SECRET_ICON_BUTTON_MARGIN)
                            .with_margin_top(SECRET_ICON_BUTTON_MARGIN)
                            .finish(),
                    )
                    .right()
                    .finish(),
                );
            }
            stack.finish()
        };

        let editor_column = input_container;

        Shrinkable::new(
            1.,
            Container::new(ConstrainedBox::new(editor_column).finish())
                .with_margin_right(margin_right)
                .finish(),
        )
        .finish()
    }

    /// Sync all editors with the user's access level. If the env var collection is view-only, all
    /// editors are set to selection-only mode. Otherwise, all are enabled.
    pub(super) fn update_editor_interactivity(&mut self, ctx: &mut ViewContext<Self>) {
        let editability = self
            .active_env_var_collection_data
            .as_ref(ctx)
            .editability(ctx);
        let interaction_state = if editability.can_edit() {
            InteractionState::Editable
        } else {
            InteractionState::Selectable
        };

        // Update metadata editors.
        self.title_editor.update(ctx, |editor, ctx| {
            editor.set_interaction_state(interaction_state, ctx)
        });
        self.description_editor.update(ctx, |editor, ctx| {
            editor.set_interaction_state(interaction_state, ctx)
        });

        // Update individual variable editors.
        for variable_row in self.variable_rows.iter() {
            variable_row
                .variable_name_editor
                .update(ctx, |editor, ctx| {
                    editor.set_interaction_state(interaction_state, ctx)
                });
            variable_row
                .variable_description_editor
                .update(ctx, |editor, ctx| {
                    editor.set_interaction_state(interaction_state, ctx)
                });
            variable_row
                .variable_value_editor
                .update(ctx, |editor, ctx| {
                    editor.set_interaction_state(interaction_state, ctx)
                });
        }
    }
}
