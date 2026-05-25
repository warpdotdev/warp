use warpui::elements::{
    Border, Container, CornerRadius, CrossAxisAlignment, Expanded, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius,
};
use warpui::keymap::macros::*;
use warpui::keymap::{FixedBinding, Keystroke};
use warpui::platform::Cursor;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::keyboard_shortcut::KeyboardShortcut;
use warpui::ui_components::text::Span;
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::appearance::Appearance;
use crate::ui_components::blended_colors;

type ItemLabelFn<T> = Box<dyn Fn(&T, &AppContext) -> Span>;

const HAS_ITEMS: &str = "HasItems";

pub fn init(app: &mut AppContext) {
    let context = id!(ToggleableItemsView::<()>::ui_name()) & id!(HAS_ITEMS);
    app.register_fixed_bindings([
        FixedBinding::new("enter", ToggleableItemsAction::Submit, context.clone()),
        FixedBinding::new(
            "numpadenter",
            ToggleableItemsAction::Submit,
            context.clone(),
        ),
        FixedBinding::new("up", ToggleableItemsAction::ArrowUp, context.clone()),
        FixedBinding::new("down", ToggleableItemsAction::ArrowDown, context.clone()),
        FixedBinding::new(
            "cmdorctrl-enter",
            ToggleableItemsAction::ToggleFocused,
            context,
        ),
    ]);
}

/// Builder for configuring how individual items should be displayed and selected.
///
/// # Type Parameters
/// - `T`: The data type for each item
pub struct ToggleableItemBuilder<T> {
    label_fn: ItemLabelFn<T>,
    is_selected_fn: Box<dyn Fn(&T) -> bool>,
}

impl<T> ToggleableItemBuilder<T> {
    pub fn new(
        label_fn: impl Fn(&T, &AppContext) -> Span + 'static,
        is_selected_fn: impl Fn(&T) -> bool + 'static,
    ) -> Self {
        Self {
            label_fn: Box::new(label_fn),
            is_selected_fn: Box::new(is_selected_fn),
        }
    }
}

/// Internal action type for interacting with an item list.
#[derive(Debug, Clone, Copy)]
pub enum ToggleableItemsAction {
    ToggleItem(usize),
    ToggleFocused,
    ArrowUp,
    ArrowDown,
    Submit,
}

/// A generic view for displaying multiple items with checkboxes.
///
/// # Type Parameters
/// - `T`: The data type for each item
pub struct ToggleableItemsView<T> {
    items: Vec<T>,
    selected_states: Vec<bool>,
    label_fn: ItemLabelFn<T>,
    checkbox_mouse_states: Vec<MouseStateHandle>,
    row_mouse_states: Vec<MouseStateHandle>,
    selected_item_index: usize,
}

impl<T> ToggleableItemsView<T> {
    pub fn new(items: Vec<T>, builder: ToggleableItemBuilder<T>) -> Self {
        let count = items.len();
        let selected_states = items
            .iter()
            .map(|item| (builder.is_selected_fn)(item))
            .collect();

        Self {
            items,
            selected_states,
            label_fn: builder.label_fn,
            checkbox_mouse_states: (0..count).map(|_| MouseStateHandle::default()).collect(),
            row_mouse_states: (0..count).map(|_| MouseStateHandle::default()).collect(),
            selected_item_index: 0,
        }
    }

    /// Get the currently selected items.
    pub fn get_selected_items(&self) -> impl Iterator<Item = &T> + '_ {
        self.items
            .iter()
            .zip(self.selected_states.iter())
            .filter_map(|(item, selected)| selected.then_some(item))
    }
}

pub enum ToggleableItemsEvent {
    SelectionChanged,
    SubmitRequested,
}

impl<T: 'static> Entity for ToggleableItemsView<T> {
    type Event = ToggleableItemsEvent;
}

impl<T: 'static> View for ToggleableItemsView<T> {
    fn ui_name() -> &'static str {
        "ToggleableItemsView"
    }

    fn keymap_context(&self, _app: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();
        if !self.items.is_empty() {
            context.set.insert(HAS_ITEMS);
        }
        context
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme();
        let border_color = blended_colors::neutral_4(theme);

        let mut outer_container =
            Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        let mut checkboxes_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(4.);

        // Render checkboxes for each item
        for (index, item) in self.items.iter().enumerate() {
            let is_selected = self.selected_states[index];
            let is_focused = index == self.selected_item_index;
            let label = (self.label_fn)(item, ctx);

            let checkbox = appearance
                .ui_builder()
                .checkbox(self.checkbox_mouse_states[index].clone(), None)
                .check(is_selected)
                .with_label(label)
                .build()
                .finish();

            let row_inner: Box<dyn Element> = if is_focused {
                let toggle_keystroke = if cfg!(target_os = "macos") {
                    Keystroke::parse("cmd-enter").expect("can parse cmd-enter")
                } else {
                    Keystroke::parse("ctrl-enter").expect("can parse ctrl-enter")
                };

                let hint_styles = UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_size: Some(appearance.monospace_font_size() - 2.),
                    font_color: Some(blended_colors::text_sub(theme, theme.surface_1())),
                    ..Default::default()
                };

                let shortcut = KeyboardShortcut::new(&toggle_keystroke, hint_styles)
                    .text_only()
                    .build()
                    .finish();

                let hint_text = Span::new(
                    "to toggle selection",
                    UiComponentStyles {
                        margin: Some(Coords::default().left(6.)),
                        ..hint_styles
                    },
                )
                .build()
                .finish();

                let hint = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(shortcut)
                    .with_child(hint_text)
                    .finish();

                Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(Expanded::new(1., checkbox).finish())
                    .with_child(hint)
                    .finish()
            } else {
                checkbox
            };

            // Make the entire row clickable (not just the checkbox control) by wrapping the
            // padded/bordered container in a separate Hoverable click target.
            // Use accent border color when focused to show keyboard focus.
            let row_border_color = if is_focused {
                theme.accent().into()
            } else {
                border_color
            };

            let row = Hoverable::new(self.row_mouse_states[index].clone(), |_| {
                Container::new(row_inner)
                    .with_horizontal_padding(12.)
                    .with_vertical_padding(8.)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .with_border(Border::all(1.).with_border_fill(row_border_color))
                    .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(ToggleableItemsAction::ToggleItem(index));
            })
            .finish();

            checkboxes_column.add_child(row);
        }

        outer_container.add_child(checkboxes_column.finish());

        outer_container.finish()
    }
}

impl<T: 'static> TypedActionView for ToggleableItemsView<T> {
    type Action = ToggleableItemsAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ToggleableItemsAction::ToggleItem(index) => {
                if let Some(selected) = self.selected_states.get_mut(*index) {
                    *selected = !*selected;
                    ctx.emit(ToggleableItemsEvent::SelectionChanged);
                    ctx.notify();
                }
            }
            ToggleableItemsAction::ToggleFocused => {
                if let Some(selected) = self.selected_states.get_mut(self.selected_item_index) {
                    *selected = !*selected;
                    ctx.emit(ToggleableItemsEvent::SelectionChanged);
                    ctx.notify();
                }
            }
            ToggleableItemsAction::ArrowUp => {
                if !self.items.is_empty() {
                    self.selected_item_index =
                        (self.selected_item_index + self.items.len() - 1) % self.items.len();
                    ctx.notify();
                }
            }
            ToggleableItemsAction::ArrowDown => {
                if !self.items.is_empty() {
                    self.selected_item_index = (self.selected_item_index + 1) % self.items.len();
                    ctx.notify();
                }
            }
            ToggleableItemsAction::Submit => {
                ctx.emit(ToggleableItemsEvent::SubmitRequested);
            }
        }
    }
}
