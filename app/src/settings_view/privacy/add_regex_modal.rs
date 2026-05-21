use crate::editor::Event as EditorEvent;
use crate::modal::{Modal, ModalViewState};
use crate::{
    appearance::Appearance,
    editor::{EditorView, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions, TextOptions},
    i18n::{self, I18nKey},
    settings::LanguageSettings,
};
use regex::Regex;
use warp_editor::editor::NavigationKey;
use warpui::elements::{CrossAxisAlignment, Expanded, MainAxisSize};
use warpui::{
    elements::{ChildView, Container, Empty, Flex, MouseStateHandle, ParentElement, Text},
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

const LABEL_FONT_SIZE: f32 = 12.;

pub struct AddRegexModal {
    name_editor: ViewHandle<EditorView>,
    pattern_editor: ViewHandle<EditorView>,
    cancel_button_mouse_state: MouseStateHandle,
    submit_button_mouse_state: MouseStateHandle,
}

#[derive(Debug)]
pub enum AddRegexModalAction {
    Cancel,
    Submit,
}

pub enum AddRegexModalEvent {
    Close,
    Submit { name: String, pattern: String },
}

impl AddRegexModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let font_family = Appearance::as_ref(ctx).ui_font_family();

        let name_editor = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_family_override: Some(font_family),
                    ..Default::default()
                },
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text(i18n::tr(ctx, I18nKey::PrivacyModalNamePlaceholder), ctx);
            editor
        });

        let pattern_editor = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_family_override: Some(font_family),
                    ..Default::default()
                },
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("\\bAIza[0-9A-Za-z-_]{35}\\b", ctx);
            editor
        });

        // Subscribe to editor events for tab navigation and re-rendering
        ctx.subscribe_to_view(&name_editor, |me, _, event, ctx| {
            me.handle_name_editor_event(event, ctx);
        });
        ctx.subscribe_to_view(&pattern_editor, |me, _, event, ctx| {
            me.handle_pattern_editor_event(event, ctx);
        });
        ctx.subscribe_to_model(&LanguageSettings::handle(ctx), |me, _, _, ctx| {
            me.update_localized_controls(ctx);
            ctx.notify();
        });

        Self {
            name_editor,
            pattern_editor,
            cancel_button_mouse_state: Default::default(),
            submit_button_mouse_state: Default::default(),
        }
    }

    fn update_localized_controls(&mut self, ctx: &mut ViewContext<Self>) {
        self.name_editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text(i18n::tr(ctx, I18nKey::PrivacyModalNamePlaceholder), ctx);
        });
    }

    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        let name = self.name_editor.as_ref(ctx).buffer_text(ctx);
        let pattern = self.pattern_editor.as_ref(ctx).buffer_text(ctx);

        let is_valid_regex = Regex::new(&pattern).is_ok();
        if !pattern.trim().is_empty() && is_valid_regex {
            ctx.emit(AddRegexModalEvent::Submit { name, pattern });
        }
    }

    fn cancel(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(AddRegexModalEvent::Close);
    }

    pub fn on_close(&mut self, ctx: &mut ViewContext<Self>) {
        self.name_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
        });
        self.pattern_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
        });
    }

    pub fn on_open(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.name_editor);
    }

    fn handle_name_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Navigate(NavigationKey::Tab) => {
                ctx.focus(&self.pattern_editor);
            }
            EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                // Wrap around to pattern editor (last field)
                ctx.focus(&self.pattern_editor);
            }
            EditorEvent::Enter => {
                // Submit if pattern is not empty and valid regex (same logic as submit button)
                let pattern = self.pattern_editor.as_ref(ctx).buffer_text(ctx);
                let is_valid_regex = Regex::new(&pattern).is_ok();
                if !pattern.trim().is_empty() && is_valid_regex {
                    self.submit(ctx);
                }
            }
            EditorEvent::Escape => {
                // Close modal like clicking Cancel or X button
                self.cancel(ctx);
            }
            EditorEvent::Edited(_) => {
                // Re-render to update validation when name field changes
                ctx.notify();
            }
            _ => {}
        }
    }

    fn handle_pattern_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Navigate(NavigationKey::Tab) => {
                ctx.focus(&self.name_editor);
            }
            EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                ctx.focus(&self.name_editor);
            }
            EditorEvent::Enter => {
                // Submit if pattern is not empty and valid regex (same logic as submit button)
                let pattern = self.pattern_editor.as_ref(ctx).buffer_text(ctx);
                let is_valid_regex = Regex::new(&pattern).is_ok();
                if !pattern.trim().is_empty() && is_valid_regex {
                    self.submit(ctx);
                }
            }
            EditorEvent::Escape => {
                // Close modal like clicking Cancel or X button
                self.cancel(ctx);
            }
            EditorEvent::Edited(_) => {
                // Re-render to update button state when pattern field changes
                ctx.notify();
            }
            _ => {}
        }
    }
}

impl Entity for AddRegexModal {
    type Event = AddRegexModalEvent;
}

impl View for AddRegexModal {
    fn ui_name() -> &'static str {
        "AddRegexModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // Check if regex field has at least 1 character
        let pattern_text = self.pattern_editor.as_ref(app).buffer_text(app);
        let is_valid_regex = Regex::new(&pattern_text).is_ok();
        let is_submit_enabled = !pattern_text.trim().is_empty() && is_valid_regex;

        let name_label = Text::new(
            i18n::tr(app, I18nKey::PrivacyModalNameOptional),
            appearance.ui_font_family(),
            LABEL_FONT_SIZE,
        )
        .with_color(theme.active_ui_text_color().into())
        .finish();

        let regex_label = Text::new(
            i18n::tr(app, I18nKey::PrivacyModalRegexPattern),
            appearance.ui_font_family(),
            LABEL_FONT_SIZE,
        )
        .with_color(theme.active_ui_text_color().into())
        .finish();

        let button_style = UiComponentStyles {
            font_size: Some(14.),
            padding: Some(Coords::uniform(8.).left(12.).right(12.)),
            ..Default::default()
        };

        let mut add_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.submit_button_mouse_state.clone(),
            )
            .with_text_label(i18n::tr(app, I18nKey::PrivacyAddRegex).to_string())
            .with_style(button_style);

        if !is_submit_enabled {
            add_button = add_button.disabled();
        }

        let buttons_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Expanded::new(
                    1.,
                    Container::new(if !is_valid_regex && !pattern_text.trim().is_empty() {
                        Text::new(
                            i18n::tr(app, I18nKey::PrivacyModalInvalidRegex),
                            appearance.ui_font_family(),
                            LABEL_FONT_SIZE,
                        )
                        .with_color(
                            appearance
                                .theme()
                                .sub_text_color(appearance.theme().background())
                                .into(),
                        )
                        .finish()
                    } else {
                        Empty::new().finish()
                    })
                    .with_margin_bottom(8.)
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                appearance
                    .ui_builder()
                    .button(
                        ButtonVariant::Secondary,
                        self.cancel_button_mouse_state.clone(),
                    )
                    .with_text_label(i18n::tr(app, I18nKey::PrivacyModalCancel).to_string())
                    .with_style(button_style)
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(AddRegexModalAction::Cancel);
                    })
                    .finish(),
            )
            .with_child(
                Container::new(
                    add_button
                        .build()
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(AddRegexModalAction::Submit);
                        })
                        .finish(),
                )
                .with_margin_left(12.)
                .finish(),
            )
            .finish();

        Flex::column()
            .with_child(Container::new(name_label).with_margin_bottom(4.).finish())
            .with_child(
                Container::new(ChildView::new(&self.name_editor).finish())
                    .with_margin_bottom(16.)
                    .finish(),
            )
            .with_child(Container::new(regex_label).with_margin_bottom(4.).finish())
            .with_child(
                Container::new(ChildView::new(&self.pattern_editor).finish())
                    .with_margin_bottom(24.)
                    .finish(),
            )
            .with_child(buttons_row)
            .finish()
    }
}

impl TypedActionView for AddRegexModal {
    type Action = AddRegexModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AddRegexModalAction::Cancel => self.cancel(ctx),
            AddRegexModalAction::Submit => self.submit(ctx),
        }
    }
}

pub struct AddRegexModalViewState {
    state: ModalViewState<Modal<AddRegexModal>>,
}

impl AddRegexModalViewState {
    pub fn new(state: ModalViewState<Modal<AddRegexModal>>) -> Self {
        Self { state }
    }

    pub fn is_open(&self) -> bool {
        self.state.is_open()
    }

    pub fn render(&self) -> Box<dyn Element> {
        self.state.render()
    }

    pub fn set_title<T: View>(&mut self, title: Option<String>, ctx: &mut ViewContext<T>) {
        self.state.view.update(ctx, |modal, _| {
            modal.set_title(title);
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
