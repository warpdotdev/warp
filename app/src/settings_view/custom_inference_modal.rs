use crate::editor::Event as EditorEvent;
use crate::modal::{Modal, ModalViewState};
use crate::{
    appearance::Appearance,
    editor::{EditorView, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions, TextOptions},
    ui_components::icons::Icon,
    view_components::action_button::{ActionButton, DangerSecondaryTheme},
};
use warp_editor::editor::NavigationKey;
use warpui::elements::{ConstrainedBox, CrossAxisAlignment, Expanded, MainAxisSize};
use warpui::{
    elements::{
        Border, ChildView, Container, CornerRadius, Empty, Flex, MouseStateHandle, ParentElement,
        Radius, Text,
    },
    fonts::FamilyId,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use ::ai::api_keys::CustomEndpoint;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use url::Url;

const LABEL_FONT_SIZE: f32 = 12.;
const INPUT_WIDTH: f32 = 480.;

const MODEL_ROW_SPACING: f32 = 16.;
const REMOVE_MODEL_BUTTON_COL_WIDTH: f32 = 32.;
const MODEL_INPUT_WIDTH: f32 = (INPUT_WIDTH - MODEL_ROW_SPACING) / 2.;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomEndpointModalEvent {
    Close,
    AddEndpoint {
        name: String,
        url: String,
        api_key: String,
        models: Vec<(String, Option<String>, Option<String>)>,
    },
    SaveEndpoint {
        index: usize,
        name: String,
        url: String,
        api_key: String,
        models: Vec<(String, Option<String>, Option<String>)>,
    },
    RemoveEndpoint {
        index: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomEndpointModalAction {
    Cancel,
    Save,
    AddModel,
    RemoveModel(usize),
    RemoveEndpoint,
}

struct ModelRow {
    name_editor: ViewHandle<EditorView>,
    alias_editor: ViewHandle<EditorView>,
    remove_mouse_state: MouseStateHandle,
    config_key: Option<String>,
}

pub struct CustomEndpointModal {
    endpoint_name_editor: ViewHandle<EditorView>,
    endpoint_url_editor: ViewHandle<EditorView>,
    api_key_editor: ViewHandle<EditorView>,
    model_rows: Vec<ModelRow>,
    cancel_button_mouse_state: MouseStateHandle,
    save_button_mouse_state: MouseStateHandle,
    add_model_button_mouse_state: MouseStateHandle,
    remove_endpoint_button: ViewHandle<ActionButton>,
    editing_index: Option<usize>,
    url_has_error: bool,
}

impl CustomEndpointModal {
    pub fn new(
        endpoint: Option<&CustomEndpoint>,
        editing_index: Option<usize>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let font_family = Appearance::as_ref(ctx).ui_font_family();
        let text_colors = crate::settings_view::editor_text_colors(Appearance::as_ref(ctx));

        let endpoint_name_text_colors = text_colors.clone();
        let endpoint_name_editor = ctx.add_typed_action_view(move |ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_family_override: Some(font_family),
                    text_colors_override: Some(endpoint_name_text_colors.clone()),
                    ..Default::default()
                },
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("e.g., Zach's external models", ctx);
            if let Some(ep) = endpoint {
                editor.set_buffer_text(&ep.name, ctx);
            }
            editor
        });

        let endpoint_url_text_colors = text_colors.clone();
        let endpoint_url_editor = ctx.add_typed_action_view(move |ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_family_override: Some(font_family),
                    text_colors_override: Some(endpoint_url_text_colors.clone()),
                    ..Default::default()
                },
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("Please include 'https://'", ctx);
            if let Some(ep) = endpoint {
                editor.set_buffer_text(&ep.url, ctx);
            }
            editor
        });

        let api_key_text_colors = text_colors.clone();
        let api_key_editor = ctx.add_typed_action_view(move |ctx| {
            let options = SingleLineEditorOptions {
                is_password: true,
                text: TextOptions {
                    font_family_override: Some(font_family),
                    text_colors_override: Some(api_key_text_colors.clone()),
                    ..Default::default()
                },
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("e.g., sk-...", ctx);
            if let Some(ep) = endpoint {
                editor.set_buffer_text(&ep.api_key, ctx);
            }
            editor
        });

        let mut model_rows = Vec::new();
        if let Some(ep) = endpoint {
            for model in &ep.models {
                model_rows.push(Self::create_model_row(
                    Some(&model.name),
                    model.alias.as_deref(),
                    Some(model.config_key.clone()),
                    font_family,
                    &text_colors,
                    ctx,
                ));
            }
        }
        if model_rows.is_empty() {
            model_rows.push(Self::create_model_row(
                None,
                None,
                None,
                font_family,
                &text_colors,
                ctx,
            ));
        }

        ctx.subscribe_to_view(&endpoint_name_editor, |me, _, event, ctx| {
            me.handle_endpoint_name_event(event, ctx);
        });
        ctx.subscribe_to_view(&endpoint_url_editor, |me, _, event, ctx| {
            me.handle_endpoint_url_event(event, ctx);
        });
        // Validate initial URL (if any) so the error state is accurate on open.
        let initial_url = endpoint_url_editor.as_ref(ctx).buffer_text(ctx);
        let url_has_error = !initial_url.trim().is_empty() && validate_url(&initial_url).is_err();
        ctx.subscribe_to_view(&api_key_editor, |me, _, event, ctx| {
            me.handle_api_key_event(event, ctx);
        });
        for row in &model_rows {
            let name_editor = row.name_editor.clone();
            ctx.subscribe_to_view(&name_editor, |me, editor, event, ctx| {
                me.handle_model_editor_event(&editor, event, ctx);
            });
            let alias_editor = row.alias_editor.clone();
            ctx.subscribe_to_view(&alias_editor, |me, editor, event, ctx| {
                me.handle_model_editor_event(&editor, event, ctx);
            });
        }
        let remove_endpoint_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Remove", DangerSecondaryTheme)
                .with_icon(Icon::Trash)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CustomEndpointModalAction::RemoveEndpoint);
                })
        });

        Self {
            endpoint_name_editor,
            endpoint_url_editor,
            api_key_editor,
            model_rows,
            cancel_button_mouse_state: Default::default(),
            save_button_mouse_state: Default::default(),
            add_model_button_mouse_state: Default::default(),
            remove_endpoint_button,
            editing_index,
            url_has_error,
        }
    }

    fn create_model_row(
        name: Option<&str>,
        alias: Option<&str>,
        config_key: Option<String>,
        font_family: FamilyId,
        text_colors: &crate::editor::TextColors,
        ctx: &mut ViewContext<Self>,
    ) -> ModelRow {
        let tc = text_colors.clone();
        let name_editor = ctx.add_typed_action_view(move |ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_family_override: Some(font_family),
                    text_colors_override: Some(tc.clone()),
                    ..Default::default()
                },
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("e.g., GLM-5-FP8", ctx);
            if let Some(n) = name {
                editor.set_buffer_text(n, ctx);
            }
            editor
        });

        let tc = text_colors.clone();
        let alias_editor = ctx.add_typed_action_view(move |ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_family_override: Some(font_family),
                    text_colors_override: Some(tc.clone()),
                    ..Default::default()
                },
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("e.g., GLM-5", ctx);
            if let Some(a) = alias {
                editor.set_buffer_text(a, ctx);
            }
            editor
        });

        ModelRow {
            name_editor,
            alias_editor,
            remove_mouse_state: Default::default(),
            config_key,
        }
    }

    pub fn prefill(
        &mut self,
        endpoint: Option<&CustomEndpoint>,
        editing_index: Option<usize>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.editing_index = editing_index;
        self.endpoint_name_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(endpoint.map(|e| e.name.as_str()).unwrap_or(""), ctx);
        });
        self.endpoint_url_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(endpoint.map(|e| e.url.as_str()).unwrap_or(""), ctx);
        });
        let url = self.endpoint_url_editor.as_ref(ctx).buffer_text(ctx);
        self.url_has_error = !url.trim().is_empty() && validate_url(&url).is_err();
        self.api_key_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(endpoint.map(|e| e.api_key.as_str()).unwrap_or(""), ctx);
        });
        // Rebuild model rows
        // Old model row editors will be dropped with the modal body
        self.model_rows.clear();
        let font_family = Appearance::as_ref(ctx).ui_font_family();
        let text_colors = crate::settings_view::editor_text_colors(Appearance::as_ref(ctx));
        if let Some(ep) = endpoint {
            for model in &ep.models {
                self.model_rows.push(Self::create_model_row(
                    Some(&model.name),
                    model.alias.as_deref(),
                    Some(model.config_key.clone()),
                    font_family,
                    &text_colors,
                    ctx,
                ));
            }
        }
        if self.model_rows.is_empty() {
            self.model_rows.push(Self::create_model_row(
                None,
                None,
                None,
                font_family,
                &text_colors,
                ctx,
            ));
        }
        for row in &self.model_rows {
            let name_editor = row.name_editor.clone();
            ctx.subscribe_to_view(&name_editor, |me, editor, event, ctx| {
                me.handle_model_editor_event(&editor, event, ctx);
            });
            let alias_editor = row.alias_editor.clone();
            ctx.subscribe_to_view(&alias_editor, |me, editor, event, ctx| {
                me.handle_model_editor_event(&editor, event, ctx);
            });
        }
    }

    pub fn on_open(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.endpoint_name_editor);
        ctx.notify();
    }

    pub fn on_close(&mut self, ctx: &mut ViewContext<Self>) {
        self.endpoint_name_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
        });
        self.endpoint_url_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
        });
        self.api_key_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
        });
        for row in &self.model_rows {
            row.name_editor.update(ctx, |editor, ctx| {
                editor.clear_buffer_and_reset_undo_stack(ctx);
            });
            row.alias_editor.update(ctx, |editor, ctx| {
                editor.clear_buffer_and_reset_undo_stack(ctx);
            });
        }
    }

    fn save(&mut self, ctx: &mut ViewContext<Self>) {
        self.validate_url_field(ctx);
        if !self.is_valid(ctx) {
            return;
        }
        let name = self.endpoint_name_editor.as_ref(ctx).buffer_text(ctx);
        let url = self.endpoint_url_editor.as_ref(ctx).buffer_text(ctx);
        let api_key = self.api_key_editor.as_ref(ctx).buffer_text(ctx);
        let models: Vec<(String, Option<String>, Option<String>)> = self
            .model_rows
            .iter()
            .map(|row| {
                let name = row.name_editor.as_ref(ctx).buffer_text(ctx);
                let alias = row.alias_editor.as_ref(ctx).buffer_text(ctx);
                let alias_opt = if alias.trim().is_empty() {
                    None
                } else {
                    Some(alias)
                };
                (name, alias_opt, row.config_key.clone())
            })
            .filter(|(name, _, _)| !name.trim().is_empty())
            .collect();
        if let Some(index) = self.editing_index {
            ctx.emit(CustomEndpointModalEvent::SaveEndpoint {
                index,
                name,
                url,
                api_key,
                models,
            });
        } else {
            ctx.emit(CustomEndpointModalEvent::AddEndpoint {
                name,
                url,
                api_key,
                models,
            });
        }
    }

    fn cancel(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(CustomEndpointModalEvent::Close);
    }

    fn add_model(&mut self, ctx: &mut ViewContext<Self>) {
        let font_family = Appearance::as_ref(ctx).ui_font_family();
        let text_colors = crate::settings_view::editor_text_colors(Appearance::as_ref(ctx));
        let row = Self::create_model_row(None, None, None, font_family, &text_colors, ctx);
        // Subscribe to the new editors
        let name_editor = row.name_editor.clone();
        ctx.subscribe_to_view(&name_editor, |me, editor, event, ctx| {
            me.handle_model_editor_event(&editor, event, ctx);
        });
        let alias_editor = row.alias_editor.clone();
        ctx.subscribe_to_view(&alias_editor, |me, editor, event, ctx| {
            me.handle_model_editor_event(&editor, event, ctx);
        });
        self.model_rows.push(row);
        ctx.notify();
    }

    fn remove_model(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if index < self.model_rows.len() {
            let _row = self.model_rows.remove(index);
            ctx.notify();
        }
    }

    fn is_valid(&self, app: &AppContext) -> bool {
        let name = self.endpoint_name_editor.as_ref(app).buffer_text(app);
        let url = self.endpoint_url_editor.as_ref(app).buffer_text(app);
        let api_key = self.api_key_editor.as_ref(app).buffer_text(app);
        let has_models = self.model_rows.iter().any(|row| {
            !row.name_editor
                .as_ref(app)
                .buffer_text(app)
                .trim()
                .is_empty()
        });
        is_endpoint_form_valid(&name, &url, &api_key, has_models)
    }

    fn focus_next_editor(&self, current: &ViewHandle<EditorView>, ctx: &mut ViewContext<Self>) {
        let mut editors: Vec<&ViewHandle<EditorView>> = vec![
            &self.endpoint_name_editor,
            &self.endpoint_url_editor,
            &self.api_key_editor,
        ];
        for row in &self.model_rows {
            editors.push(&row.name_editor);
            editors.push(&row.alias_editor);
        }
        if let Some(pos) = editors.iter().position(|e| *e == current) {
            let next = (pos + 1) % editors.len();
            ctx.focus(editors[next]);
        }
    }

    fn focus_prev_editor(&self, current: &ViewHandle<EditorView>, ctx: &mut ViewContext<Self>) {
        let mut editors: Vec<&ViewHandle<EditorView>> = vec![
            &self.endpoint_name_editor,
            &self.endpoint_url_editor,
            &self.api_key_editor,
        ];
        for row in &self.model_rows {
            editors.push(&row.name_editor);
            editors.push(&row.alias_editor);
        }
        if let Some(pos) = editors.iter().position(|e| *e == current) {
            let prev = if pos == 0 { editors.len() - 1 } else { pos - 1 };
            ctx.focus(editors[prev]);
        }
    }

    fn handle_endpoint_name_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Navigate(NavigationKey::Tab) => {
                ctx.focus(&self.endpoint_url_editor);
            }
            EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                self.focus_prev_editor(&self.endpoint_name_editor, ctx);
            }
            EditorEvent::Enter => {
                ctx.focus(&self.endpoint_url_editor);
            }
            EditorEvent::Escape => {
                self.cancel(ctx);
            }
            EditorEvent::Edited(_) => {
                ctx.notify();
            }
            _ => {}
        }
    }

    fn handle_endpoint_url_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Navigate(NavigationKey::Tab) => {
                self.validate_url_field(ctx);
                ctx.focus(&self.api_key_editor);
            }
            EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                self.validate_url_field(ctx);
                ctx.focus(&self.endpoint_name_editor);
            }
            EditorEvent::Enter => {
                self.validate_url_field(ctx);
                ctx.focus(&self.api_key_editor);
            }
            EditorEvent::Escape => {
                self.cancel(ctx);
            }
            EditorEvent::Edited(_) => {
                if !self.validate_url_field(ctx) {
                    ctx.notify();
                }
            }
            _ => {}
        }
    }

    fn validate_url_field(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let url = self.endpoint_url_editor.as_ref(ctx).buffer_text(ctx);
        let had_error = self.url_has_error;
        self.url_has_error = !url.trim().is_empty() && validate_url(&url).is_err();
        let changed = self.url_has_error != had_error;
        if changed {
            ctx.notify();
        }
        changed
    }

    fn handle_api_key_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Navigate(NavigationKey::Tab) => {
                if let Some(first_row) = self.model_rows.first() {
                    ctx.focus(&first_row.name_editor);
                }
            }
            EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                ctx.focus(&self.endpoint_url_editor);
            }
            EditorEvent::Enter => {
                if let Some(first_row) = self.model_rows.first() {
                    ctx.focus(&first_row.name_editor);
                }
            }
            EditorEvent::Escape => {
                self.cancel(ctx);
            }
            EditorEvent::Edited(_) => {
                ctx.notify();
            }
            _ => {}
        }
    }

    fn handle_model_editor_event(
        &mut self,
        editor: &ViewHandle<EditorView>,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Navigate(NavigationKey::Tab) | EditorEvent::Enter => {
                self.focus_next_editor(editor, ctx);
            }
            EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                self.focus_prev_editor(editor, ctx);
            }
            EditorEvent::Escape => {
                self.cancel(ctx);
            }
            EditorEvent::Edited(_) => {
                ctx.notify();
            }
            _ => {}
        }
    }
}

impl Entity for CustomEndpointModal {
    type Event = CustomEndpointModalEvent;
}

impl View for CustomEndpointModal {
    fn ui_name() -> &'static str {
        "CustomEndpointModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let is_valid = self.is_valid(app);
        let is_editing = self.editing_index.is_some();

        let label_font_family = appearance.ui_font_family();
        let label_text_color = theme.active_ui_text_color().into();
        let label = move |text: &'static str| {
            Text::new(text, label_font_family, LABEL_FONT_SIZE)
                .with_color(label_text_color)
                .finish()
        };

        let input_style = UiComponentStyles {
            width: Some(INPUT_WIDTH),
            ..Default::default()
        };
        let button_style = UiComponentStyles {
            font_size: Some(14.),
            padding: Some(Coords::uniform(8.).left(12.).right(12.)),
            ..Default::default()
        };

        let mut column = Flex::column();

        // Description
        column.add_child(
            Container::new(
                Text::new(
                    "Provide your endpoint details below. You can add as many models from the endpoint as you'd like and can also provide aliases for the model picker in your input.",
                    appearance.ui_font_family(),
                    LABEL_FONT_SIZE,
                )
                .with_color(theme.nonactive_ui_text_color().into())
                .soft_wrap(true)
                .finish(),
            )
            .with_margin_bottom(16.)
            .finish(),
        );

        // Endpoint name
        column.add_child(
            Container::new(label("Endpoint name"))
                .with_margin_bottom(4.)
                .finish(),
        );
        column.add_child(
            Container::new(
                appearance
                    .ui_builder()
                    .text_input(self.endpoint_name_editor.clone())
                    .with_style(input_style)
                    .build()
                    .finish(),
            )
            .with_margin_bottom(16.)
            .finish(),
        );

        // Endpoint URL
        column.add_child(
            Container::new(label("Endpoint URL"))
                .with_margin_bottom(4.)
                .finish(),
        );
        let url_border_fill = if self.url_has_error {
            theme.ui_error_color().into()
        } else {
            theme.outline()
        };
        column.add_child(
            Container::new(
                appearance
                    .ui_builder()
                    .text_input(self.endpoint_url_editor.clone())
                    .with_style(input_style)
                    .build()
                    .finish(),
            )
            .with_border(Border::all(1.).with_border_fill(url_border_fill))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_margin_bottom(16.)
            .finish(),
        );

        // API key
        column.add_child(
            Container::new(label("API key"))
                .with_margin_bottom(4.)
                .finish(),
        );
        column.add_child(
            Container::new(
                appearance
                    .ui_builder()
                    .text_input(self.api_key_editor.clone())
                    .with_style(input_style)
                    .build()
                    .finish(),
            )
            .with_margin_bottom(16.)
            .finish(),
        );

        // Model rows
        let has_remove_model_button = self.model_rows.len() > 1;
        let mut model_labels = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(MODEL_ROW_SPACING)
            .with_child(
                ConstrainedBox::new(label("Model name"))
                    .with_width(MODEL_INPUT_WIDTH)
                    .finish(),
            )
            .with_child(
                ConstrainedBox::new(label("Model alias (optional)"))
                    .with_width(MODEL_INPUT_WIDTH)
                    .finish(),
            );
        if has_remove_model_button {
            model_labels.add_child(
                ConstrainedBox::new(Empty::new().finish())
                    .with_width(REMOVE_MODEL_BUTTON_COL_WIDTH)
                    .finish(),
            );
        }

        column.add_child(
            Container::new(model_labels.finish())
                .with_margin_bottom(4.)
                .finish(),
        );

        for (i, row) in self.model_rows.iter().enumerate() {
            let name_input = appearance
                .ui_builder()
                .text_input(row.name_editor.clone())
                .with_style(UiComponentStyles {
                    width: Some(MODEL_INPUT_WIDTH),
                    ..Default::default()
                })
                .build()
                .finish();

            let alias_input = appearance
                .ui_builder()
                .text_input(row.alias_editor.clone())
                .with_style(UiComponentStyles {
                    width: Some(MODEL_INPUT_WIDTH),
                    ..Default::default()
                })
                .build()
                .finish();

            let remove_button = if self.model_rows.len() > 1 {
                appearance
                    .ui_builder()
                    .close_button(20., row.remove_mouse_state.clone())
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(CustomEndpointModalAction::RemoveModel(i));
                    })
                    .finish()
            } else {
                Empty::new().finish()
            };

            let mut row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(MODEL_ROW_SPACING)
                .with_child(name_input)
                .with_child(alias_input);
            if has_remove_model_button {
                row.add_child(
                    ConstrainedBox::new(remove_button)
                        .with_width(REMOVE_MODEL_BUTTON_COL_WIDTH)
                        .finish(),
                );
            }
            let row = row.finish();

            column.add_child(Container::new(row).with_margin_bottom(12.).finish());
        }

        // + Add model button
        column.add_child(
            Container::new(
                appearance
                    .ui_builder()
                    .button(
                        ButtonVariant::Secondary,
                        self.add_model_button_mouse_state.clone(),
                    )
                    .with_text_label("+ Add model".to_string())
                    .with_style(UiComponentStyles {
                        font_size: Some(14.),
                        padding: Some(Coords::uniform(6.).left(8.).right(8.)),
                        ..Default::default()
                    })
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(CustomEndpointModalAction::AddModel);
                    })
                    .finish(),
            )
            .with_margin_bottom(24.)
            .finish(),
        );

        // Bottom buttons row
        let mut buttons_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        // Remove button (only when editing)
        if is_editing {
            buttons_row.add_child(ChildView::new(&self.remove_endpoint_button).finish());
        }

        buttons_row.add_child(Expanded::new(1., Empty::new().finish()).finish());

        buttons_row.add_child(
            appearance
                .ui_builder()
                .button(
                    ButtonVariant::Secondary,
                    self.cancel_button_mouse_state.clone(),
                )
                .with_text_label("Cancel".to_string())
                .with_style(button_style)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(CustomEndpointModalAction::Cancel);
                })
                .finish(),
        );

        let mut save_button = appearance
            .ui_builder()
            .button(ButtonVariant::Accent, self.save_button_mouse_state.clone())
            .with_text_label(if is_editing {
                "Save".to_string()
            } else {
                "Add endpoint".to_string()
            })
            .with_style(button_style);
        if !is_valid {
            save_button = save_button.disabled();
        }

        buttons_row.add_child(
            Container::new(
                save_button
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(CustomEndpointModalAction::Save);
                    })
                    .finish(),
            )
            .with_margin_left(12.)
            .finish(),
        );

        column.add_child(buttons_row.finish());

        column.finish()
    }
}

fn validate_url(url: &str) -> Result<(), &'static str> {
    if url.trim().is_empty() {
        return Ok(());
    }
    let parsed = Url::parse(url).map_err(|_| "Invalid URL")?;
    if parsed.scheme() != "https" {
        return Err("URL must use HTTPS");
    }
    let Some(host) = parsed.host_str().filter(|h| !h.is_empty()) else {
        return Err("URL must include a host");
    };
    if is_restricted_host(host) {
        return Err("URL must not use a local or private host");
    }
    Ok(())
}

fn is_endpoint_form_valid(name: &str, url: &str, api_key: &str, has_models: bool) -> bool {
    !name.trim().is_empty()
        && !url.trim().is_empty()
        && !api_key.trim().is_empty()
        && has_models
        && validate_url(url).is_ok()
}

fn is_restricted_host(host: &str) -> bool {
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(is_restricted_ip)
}

fn is_restricted_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_restricted_ipv4(ip),
        IpAddr::V6(ip) => is_restricted_ipv6(ip),
    }
}

fn is_restricted_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_unspecified() || ip.is_private() || ip.is_link_local()
}

fn is_restricted_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || is_ipv6_unique_local(ip) || is_ipv6_link_local(ip)
    {
        return true;
    }
    if let Some(ipv4) = ip.to_ipv4_mapped() {
        return is_restricted_ipv4(ipv4);
    }
    false
}

fn is_ipv6_unique_local(ip: Ipv6Addr) -> bool {
    ip.segments()[0] & 0xfe00 == 0xfc00
}

fn is_ipv6_link_local(ip: Ipv6Addr) -> bool {
    ip.segments()[0] & 0xffc0 == 0xfe80
}
impl TypedActionView for CustomEndpointModal {
    type Action = CustomEndpointModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CustomEndpointModalAction::Cancel => self.cancel(ctx),
            CustomEndpointModalAction::Save => self.save(ctx),
            CustomEndpointModalAction::AddModel => self.add_model(ctx),
            CustomEndpointModalAction::RemoveModel(index) => self.remove_model(*index, ctx),
            CustomEndpointModalAction::RemoveEndpoint => {
                if let Some(index) = self.editing_index {
                    ctx.emit(CustomEndpointModalEvent::RemoveEndpoint { index });
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "custom_inference_modal_tests.rs"]
mod tests;

pub struct CustomEndpointModalViewState {
    state: ModalViewState<Modal<CustomEndpointModal>>,
}

impl CustomEndpointModalViewState {
    pub fn new(state: ModalViewState<Modal<CustomEndpointModal>>) -> Self {
        Self { state }
    }

    pub fn is_open(&self) -> bool {
        self.state.is_open()
    }

    pub fn render(&self) -> Box<dyn Element> {
        self.state.render()
    }

    pub fn set_title<T: View>(&mut self, title: Option<String>, ctx: &mut ViewContext<T>) {
        self.state.view.update(ctx, |modal, ctx| {
            modal.set_title(title);
            ctx.notify();
        });
    }

    pub fn prefill<T: View>(
        &mut self,
        endpoint: Option<&CustomEndpoint>,
        editing_index: Option<usize>,
        ctx: &mut ViewContext<T>,
    ) {
        self.state.view.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                body.prefill(endpoint, editing_index, ctx);
            });
        });
    }

    pub fn open<T: View>(&mut self, ctx: &mut ViewContext<T>) {
        self.state.open();
        self.state.view.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                body.on_open(ctx);
            });
        });
    }

    pub fn close<T: View>(&mut self, ctx: &mut ViewContext<T>) {
        self.state.close();
        self.state.view.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                body.on_close(ctx);
            });
        });
    }
}
