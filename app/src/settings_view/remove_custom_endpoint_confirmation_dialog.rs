use crate::{
    appearance::Appearance,
    ui_components::dialog::{dialog_styles, Dialog},
    view_components::action_button::{ActionButton, DangerPrimaryTheme, NakedTheme},
};
use warp_core::ui::theme::color::internal_colors;
use warpui::{
    elements::{
        Border, ChildView, Container, CornerRadius, Dismiss, Empty, Flex, ParentElement, Radius,
        Text,
    },
    fonts::{Properties, Weight},
    ui_components::components::UiComponent,
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

const DIALOG_WIDTH: f32 = 450.;

pub enum RemoveCustomEndpointConfirmationDialogEvent {
    Cancel,
    Confirm(usize),
}

#[derive(Debug)]
pub enum RemoveCustomEndpointConfirmationDialogAction {
    Cancel,
    Confirm,
}

pub struct RemoveCustomEndpointConfirmationDialog {
    visible: bool,
    endpoint_index: Option<usize>,
    endpoint_name: String,
    model_labels: Vec<String>,
    cancel_button: ViewHandle<ActionButton>,
    confirm_button: ViewHandle<ActionButton>,
}

impl RemoveCustomEndpointConfirmationDialog {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let cancel_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Cancel", NakedTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(RemoveCustomEndpointConfirmationDialogAction::Cancel);
            })
        });

        let confirm_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Remove endpoint", DangerPrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(RemoveCustomEndpointConfirmationDialogAction::Confirm);
            })
        });

        Self {
            visible: false,
            endpoint_index: None,
            endpoint_name: String::new(),
            model_labels: Vec::new(),
            cancel_button,
            confirm_button,
        }
    }

    pub fn show(
        &mut self,
        endpoint_index: usize,
        endpoint_name: String,
        model_labels: Vec<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.endpoint_index = Some(endpoint_index);
        self.endpoint_name = endpoint_name;
        self.model_labels = model_labels;
        self.visible = true;
        ctx.notify();
    }

    pub fn hide(&mut self, ctx: &mut ViewContext<Self>) {
        self.visible = false;
        ctx.notify();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }
}

impl Entity for RemoveCustomEndpointConfirmationDialog {
    type Event = RemoveCustomEndpointConfirmationDialogEvent;
}

impl View for RemoveCustomEndpointConfirmationDialog {
    fn ui_name() -> &'static str {
        "RemoveCustomEndpointConfirmationDialog"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if !self.visible {
            return Empty::new().finish();
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let description = "Are you sure you want to remove this endpoint? You won't be able to use its models in your agent sessions moving forward.".to_string();

        let endpoint_title = Text::new_inline(
            self.endpoint_name.clone(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_style(Properties::default().weight(Weight::Semibold))
        .with_color(theme.active_ui_text_color().into())
        .finish();

        let chip_border = internal_colors::fg_overlay_3(theme);
        let chip_text = theme.active_ui_text_color();

        let chips =
            super::render_model_chips(self.model_labels.iter().cloned(), appearance, chip_text);

        let endpoint_card = Container::new(
            Flex::column()
                .with_spacing(8.)
                .with_child(endpoint_title)
                .with_child(chips)
                .finish(),
        )
        .with_uniform_padding(12.)
        .with_background(internal_colors::fg_overlay_1(theme))
        .with_border(Border::all(1.).with_border_fill(chip_border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .finish();

        let dialog = Dialog::new(
            "Remove endpoint?".to_string(),
            Some(description),
            dialog_styles(appearance),
        )
        .with_child(endpoint_card)
        .with_bottom_row_child(ChildView::new(&self.cancel_button).finish())
        .with_bottom_row_child(
            Container::new(ChildView::new(&self.confirm_button).finish())
                .with_margin_left(12.)
                .finish(),
        )
        .with_width(DIALOG_WIDTH)
        .build()
        .finish();

        Dismiss::new(dialog)
            .prevent_interaction_with_other_elements()
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(RemoveCustomEndpointConfirmationDialogAction::Cancel)
            })
            .finish()
    }
}

impl TypedActionView for RemoveCustomEndpointConfirmationDialog {
    type Action = RemoveCustomEndpointConfirmationDialogAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            RemoveCustomEndpointConfirmationDialogAction::Cancel => {
                ctx.emit(RemoveCustomEndpointConfirmationDialogEvent::Cancel)
            }
            RemoveCustomEndpointConfirmationDialogAction::Confirm => {
                if let Some(index) = self.endpoint_index {
                    ctx.emit(RemoveCustomEndpointConfirmationDialogEvent::Confirm(index));
                }
            }
        }
    }
}
