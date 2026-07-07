use warpui::elements::{Container, CornerRadius, Dismiss, MouseStateHandle, Radius};
use warpui::fonts::Weight;
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::appearance::Appearance;
use crate::editor::{EditorOptions, EditorView, Event as EditorEvent, TextOptions};

/// This View is a text that can be hovered over. Upon clicking,
/// the text becomes a text input that can be submitted
/// by hitting enter or clicking outside the input.
pub struct ClickableTextInput {
    text: String,
    text_button_mouse_handle: MouseStateHandle,
    show_text_as_hoverable: bool,
    editor: ViewHandle<EditorView>,
}

impl ClickableTextInput {
    pub fn new(text: String, ctx: &mut ViewContext<Self>) -> Self {
        let editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = EditorOptions {
                autogrow: true,
                soft_wrap: true,
                text: TextOptions::ui_text(None, appearance),
                ..Default::default()
            };
            EditorView::new(options, ctx)
        });
        ctx.subscribe_to_view(&editor, Self::handle_editor_event);

        Self {
            text,
            text_button_mouse_handle: Default::default(),
            show_text_as_hoverable: true,
            editor,
        }
    }

    pub fn set_placeholder_text(&mut self, text: impl Into<String>, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text(text, ctx);
        });
    }

    fn submit_input(&mut self, ctx: &mut ViewContext<Self>) {
        let content = self
            .editor
            .read(ctx, |editor, ctx| editor.buffer_text(ctx).trim().to_owned());
        if !content.is_empty() {
            ctx.emit(ClickableTextInputEvent::Submit(content));
        }
        self.show_text_as_hoverable = true;
        ctx.notify();
    }

    fn handle_editor_event(
        &mut self,
        _handle: ViewHandle<EditorView>,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Enter => {
                self.submit_input(ctx);
            }
            EditorEvent::Edited(_) => {
                ctx.notify();
            }
            _ => {}
        }
    }
}

impl View for ClickableTextInput {
    fn ui_name() -> &'static str {
        "ClickableTextInput"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        if self.show_text_as_hoverable {
            appearance
                .ui_builder()
                .button(ButtonVariant::Text, self.text_button_mouse_handle.clone())
                .with_centered_text_label(self.text.clone())
                .with_style(UiComponentStyles {
                    font_color: Some(appearance.theme().active_ui_text_color().into()),
                    font_weight: Some(Weight::Bold),
                    font_size: Some(24.),
                    ..Default::default()
                })
                .with_hovered_styles(UiComponentStyles {
                    font_color: Some(appearance.theme().nonactive_ui_text_color().into()),
                    ..Default::default()
                })
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(ClickableTextInputAction::ShowEditor)
                })
                .finish()
        } else {
            let current_theme = appearance.theme();
            let input_box = Container::new(
                Dismiss::new(
                    appearance
                        .ui_builder()
                        .text_input(self.editor.clone())
                        .with_style(UiComponentStyles {
                            width: Some(200.),
                            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
                            border_width: Some(1.),
                            border_color: Some(current_theme.accent_button_color().into()),
                            background: Some(current_theme.surface_2().into_solid().into()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(ClickableTextInputAction::Submit))
                .finish(),
            );
            input_box.finish()
        }
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.editor);
            ctx.notify();
        }
    }
}

#[derive(Debug)]
pub enum ClickableTextInputEvent {
    Submit(String),
}

impl Entity for ClickableTextInput {
    type Event = ClickableTextInputEvent;
}

#[derive(Debug)]
pub enum ClickableTextInputAction {
    ShowEditor,
    UpdateText(String),
    Submit,
}

impl TypedActionView for ClickableTextInput {
    type Action = ClickableTextInputAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ClickableTextInputAction::ShowEditor => {
                self.show_text_as_hoverable = false;
                self.editor.update(ctx, |editor, ctx| {
                    editor.clear_buffer(ctx);
                });
                ctx.focus(&self.editor);
                ctx.notify();
            }
            ClickableTextInputAction::UpdateText(new_text) => {
                self.text = new_text.to_string();
                ctx.notify();
            }
            ClickableTextInputAction::Submit => {
                self.submit_input(ctx);
            }
        }
    }
}
