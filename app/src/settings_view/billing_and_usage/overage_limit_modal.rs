use warpui::{
    elements::{Align, Clipped},
    ui_components::components::{Coords, UiComponentStyles},
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::{
    editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions, TextOptions},
    Appearance,
};
use warpui::{
    elements::{
        Border, ChildView, Container, CornerRadius, CrossAxisAlignment, Expanded, Flex,
        MouseStateHandle, Padding, ParentElement, Radius, Text,
    },
    ui_components::{button::ButtonVariant, components::UiComponent},
};

const MAXIMUM_SPENDING_LIMIT_CENTS: u32 = 999999999;

pub struct SpendingLimitModal {
    amount_editor: ViewHandle<EditorView>,
    cancel_button_mouse_state: MouseStateHandle,
    update_button_mouse_state: MouseStateHandle,
    input_error_state: Option<SpendingLimitModalInputErrorState>,
}

#[derive(Debug, Clone)]
pub enum SpendingLimitModalEvent {
    Close,
    Update { amount_cents: u32 },
}

impl Entity for SpendingLimitModal {
    type Event = SpendingLimitModalEvent;
}

#[derive(Debug, Clone)]
pub enum SpendingLimitModalAction {
    Close,
    Update,
}

pub enum SpendingLimitModalInputErrorState {
    InvalidNumberFormat,
    NumberOutOfRange,
}

impl SpendingLimitModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let font_family = Appearance::as_ref(ctx).ui_font_family();
        let amount_editor = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_family_override: Some(font_family),
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("50.00", ctx);
            editor
        });
        ctx.subscribe_to_view(&amount_editor, |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        Self {
            amount_editor,
            cancel_button_mouse_state: MouseStateHandle::default(),
            update_button_mouse_state: MouseStateHandle::default(),
            input_error_state: None,
        }
    }

    fn parse_amount(&self, app: &AppContext) -> Option<u32> {
        let text = self.amount_editor.as_ref(app).buffer_text(app);
        let text = text.trim();
        if text.is_empty() {
            return None;
        }

        let cleaned = text.strip_prefix('$').unwrap_or(text);

        if let Ok(dollars) = cleaned.parse::<f64>() {
            Some((dollars * 100.0).round() as u32)
        } else {
            None
        }
    }

    fn validate_input(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self.amount_editor.as_ref(ctx).buffer_text(ctx);

        if !self.is_valid_us_currency_format(&text) {
            self.input_error_state = Some(SpendingLimitModalInputErrorState::InvalidNumberFormat);
        } else if self
            .parse_amount(ctx)
            .is_some_and(|cents| !self.is_valid_number_range(cents))
        {
            self.input_error_state = Some(SpendingLimitModalInputErrorState::NumberOutOfRange);
        } else {
            self.input_error_state = None;
        }

        ctx.notify();
    }

    fn is_valid_number_range(&self, amount_cents: u32) -> bool {
        if amount_cents < 1 {
            return false;
        }

        if amount_cents > MAXIMUM_SPENDING_LIMIT_CENTS {
            return false;
        }

        true
    }

    fn is_valid_us_currency_format(&self, text: &str) -> bool {
        if text.is_empty() {
            return true;
        }

        let cleaned = text.strip_prefix('$').unwrap_or(text);

        let decimal_count = cleaned.chars().filter(|&c| c == '.').count();
        if decimal_count > 1 {
            return false;
        }

        if !cleaned.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return false;
        }

        if let Some(decimal_pos) = cleaned.find('.') {
            let after_decimal = &cleaned[decimal_pos + 1..];
            if after_decimal.len() > 2 {
                return false;
            }
        }

        true
    }

    pub fn update_amount_editor(&self, cents: u32, ctx: &mut ViewContext<Self>) {
        let placeholder_text = format!("{:.2}", cents as f64 / 100.0);
        self.amount_editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text(&placeholder_text, ctx);
            editor.clear_buffer(ctx);
        });

        ctx.notify();
    }

    fn error_text(&self) -> Option<String> {
        match self.input_error_state {
            Some(SpendingLimitModalInputErrorState::InvalidNumberFormat) => {
                Some("Please enter a valid currency amount".to_string())
            }
            Some(SpendingLimitModalInputErrorState::NumberOutOfRange) => {
                Some("Please enter a price between $0.01 and $10,000,000".to_string())
            }
            None => None,
        }
    }

    pub fn focus_input(&self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.amount_editor);
    }

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Enter => {
                if self.input_error_state.is_none() {
                    if let Some(amount_cents) = self.parse_amount(ctx) {
                        ctx.emit(SpendingLimitModalEvent::Update { amount_cents });
                    }
                }
            }
            EditorEvent::Escape => {
                ctx.emit(SpendingLimitModalEvent::Close);
            }
            EditorEvent::Edited(_) => {
                self.validate_input(ctx);
            }
            _ => {}
        }
    }
}

impl View for SpendingLimitModal {
    fn ui_name() -> &'static str {
        "SpendingLimitModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let description_text = Text::new(
            "Warp will prevent use of premium models when this dollar limit is reached. Resets on a monthly basis.",
            appearance.ui_font_family(),
            14.,
        )
        .with_color(theme.sub_text_color(theme.surface_2()).into())
        .finish();

        let additional_note_text = Text::new(
            "Note that AI credits made near your chosen limit may exceed it by a few dollars.",
            appearance.ui_font_family(),
            12.,
        )
        .with_color(theme.sub_text_color(theme.surface_2()).into())
        .finish();

        let description_section = Flex::column()
            .with_child(
                Container::new(description_text)
                    .with_margin_bottom(8.)
                    .finish(),
            )
            .with_child(additional_note_text)
            .finish();

        let input_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Text::new("$", appearance.ui_font_family(), appearance.ui_font_size())
                    .with_color(theme.active_ui_text_color().into())
                    .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    Container::new(
                        Clipped::new(ChildView::new(&self.amount_editor).finish()).finish(),
                    )
                    .with_margin_left(12.)
                    .finish(),
                )
                .finish(),
            )
            .finish();

        let border_color = if self.input_error_state.is_some() {
            theme.ui_error_color().into()
        } else {
            theme.outline()
        };

        let input_container = Container::new(input_row)
            .with_padding(Padding::uniform(8.).with_left(16.).with_right(16.))
            .with_border(Border::all(1.).with_border_fill(border_color))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .finish();

        let button_style = UiComponentStyles {
            font_size: Some(14.),
            padding: Some(Coords::uniform(8.).left(12.).right(12.)),
            ..Default::default()
        };

        let mut update_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.update_button_mouse_state.clone(),
            )
            .with_text_label("Update".to_string())
            .with_style(button_style);

        if self.input_error_state.is_some() {
            update_button = update_button.disabled();
        }

        let buttons_row = Flex::row()
            .with_child(
                appearance
                    .ui_builder()
                    .button(
                        ButtonVariant::Secondary,
                        self.cancel_button_mouse_state.clone(),
                    )
                    .with_text_label("Cancel".to_string())
                    .with_style(button_style)
                    .build()
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(SpendingLimitModalAction::Close);
                    })
                    .finish(),
            )
            .with_child(
                Container::new(
                    update_button
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(SpendingLimitModalAction::Update);
                        })
                        .finish(),
                )
                .with_margin_left(12.)
                .finish(),
            )
            .finish();

        let mut main_column = Flex::column()
            .with_child(
                Container::new(description_section)
                    .with_margin_bottom(16.)
                    .finish(),
            )
            .with_child(
                Container::new(input_container)
                    .with_margin_bottom(if self.input_error_state.is_some() {
                        8.
                    } else {
                        24.
                    })
                    .finish(),
            );

        if let Some(error_text) = self.error_text() {
            let error_text = Text::new(error_text, appearance.ui_font_family(), 12.)
                .with_color(theme.ui_error_color())
                .finish();

            main_column =
                main_column.with_child(Container::new(error_text).with_margin_bottom(24.).finish());
        }

        main_column
            .with_child(Align::new(buttons_row).right().finish())
            .finish()
    }
}

impl TypedActionView for SpendingLimitModal {
    type Action = SpendingLimitModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SpendingLimitModalAction::Close => {
                self.amount_editor.update(ctx, |editor, ctx| {
                    editor.clear_buffer(ctx);
                });
                ctx.emit(SpendingLimitModalEvent::Close);
            }
            SpendingLimitModalAction::Update => {
                if let Some(amount_cents) = self.parse_amount(ctx) {
                    ctx.emit(SpendingLimitModalEvent::Update { amount_cents });
                }
            }
        }
    }
}
