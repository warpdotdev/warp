use warpui::elements::{
    ChildView, Container, CrossAxisAlignment, Element, Flex, MainAxisAlignment, MainAxisSize,
    ParentElement, Text,
};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use crate::ai::llms::LLMId;
use crate::appearance::Appearance;
use crate::view_components::action_button::{ActionButton, NakedTheme, PrimaryTheme};
use crate::view_components::dropdown::{Dropdown, DropdownItem};

/// Width shared by the model dropdown's top bar and open menu so long model
/// names stay readable inside the modal.
const MODEL_DROPDOWN_WIDTH: f32 = 400.;
const MODEL_DROPDOWN_MAX_HEIGHT: f32 = 250.;

pub enum SetDefaultModelModalBodyEvent {
    /// The user dismissed the prompt without choosing a model.
    Close,
    /// The user committed `LLMId` as their new default Agent Mode model.
    SetDefault(LLMId),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SetDefaultModelModalBodyAction {
    /// Carries the index into `model_choices` of the picked model.
    SelectModel(usize),
    Save,
    Cancel,
}

/// Body of the "set your default model" prompt that appears after a BYO API key
/// or custom endpoint is saved. It is hosted inside a [`crate::modal::Modal`],
/// which supplies the title, close button, and backdrop.
pub struct SetDefaultModelModalBody {
    description: String,
    /// `(model id, label)` pairs offered in the dropdown. The id flows back out
    /// through [`SetDefaultModelModalBodyEvent::SetDefault`] on save.
    model_choices: Vec<(LLMId, String)>,
    selected_index: usize,
    model_dropdown: ViewHandle<Dropdown<SetDefaultModelModalBodyAction>>,
    cancel_button: ViewHandle<ActionButton>,
    save_button: ViewHandle<ActionButton>,
}

impl SetDefaultModelModalBody {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let model_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(MODEL_DROPDOWN_WIDTH);
            dropdown.set_menu_width(MODEL_DROPDOWN_WIDTH, ctx);
            dropdown.set_menu_max_height(MODEL_DROPDOWN_MAX_HEIGHT, ctx);
            dropdown
        });

        let cancel_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Not now", NakedTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(SetDefaultModelModalBodyAction::Cancel);
            })
        });

        let save_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Set as default", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(SetDefaultModelModalBodyAction::Save);
            })
        });

        Self {
            description: String::new(),
            model_choices: Vec::new(),
            selected_index: 0,
            model_dropdown,
            cancel_button,
            save_button,
        }
    }

    /// Populates the prompt for a freshly added credential. The first model is
    /// pre-selected so a user can accept without opening the dropdown.
    pub fn set_choices(
        &mut self,
        description: String,
        model_choices: Vec<(LLMId, String)>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.description = description;
        self.model_choices = model_choices;
        self.selected_index = 0;

        let items = self
            .model_choices
            .iter()
            .enumerate()
            .map(|(index, (_, label))| {
                DropdownItem::new(
                    label.clone(),
                    SetDefaultModelModalBodyAction::SelectModel(index),
                )
            })
            .collect();
        self.model_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(items, ctx);
            dropdown.set_selected_by_index(0, ctx);
        });
        ctx.notify();
    }
}

impl Entity for SetDefaultModelModalBody {
    type Event = SetDefaultModelModalBodyEvent;
}

impl View for SetDefaultModelModalBody {
    fn ui_name() -> &'static str {
        "SetDefaultModelModalBody"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let description = Container::new(
            Text::new(
                self.description.clone(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(theme.nonactive_ui_text_color().into())
            .soft_wrap(true)
            .finish(),
        )
        .with_margin_bottom(20.)
        .finish();

        let dropdown = Container::new(ChildView::new(&self.model_dropdown).finish())
            .with_margin_bottom(24.)
            .finish();

        let buttons_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(ChildView::new(&self.cancel_button).finish())
            .with_child(
                Container::new(ChildView::new(&self.save_button).finish())
                    .with_margin_left(12.)
                    .finish(),
            )
            .finish();

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(description)
            .with_child(dropdown)
            .with_child(buttons_row)
            .finish()
    }
}

impl TypedActionView for SetDefaultModelModalBody {
    type Action = SetDefaultModelModalBodyAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SetDefaultModelModalBodyAction::SelectModel(index) => {
                self.selected_index = *index;
                ctx.notify();
            }
            SetDefaultModelModalBodyAction::Save => {
                if let Some((id, _)) = self.model_choices.get(self.selected_index) {
                    ctx.emit(SetDefaultModelModalBodyEvent::SetDefault(id.clone()));
                }
            }
            SetDefaultModelModalBodyAction::Cancel => {
                ctx.emit(SetDefaultModelModalBodyEvent::Close);
            }
        }
    }
}
