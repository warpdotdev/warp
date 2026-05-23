use warpui::elements::{
    ClippedScrollStateHandle, Container, CrossAxisAlignment, DispatchEventResult, EventHandler,
    Flex, Hoverable, MouseInBehavior, MouseStateHandle, ParentElement, SavePosition, ScrollTarget,
    ScrollToPositionMode,
};
use warpui::keymap::FixedBinding;
use warpui::ui_components::button::Button;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, TypedActionView, View, ViewContext, ViewHandle, WeakViewHandle,
};

use super::numbered_button::{
    build_inline_input_content, build_numbered_button, build_text_button_content,
};

const MARGIN_BETWEEN_BUTTONS: f32 = 4.;
const NUMBER_SELECT_ENABLED: &str = "NumberSelectEnabled";
const KEYBOARD_NAVIGATION_ENABLED: &str = "KeyboardNavigationEnabled";
const ENTER_ACTIVATION_ENABLED: &str = "EnterActivationEnabled";

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    for i in 1..=9u8 {
        app.register_fixed_bindings([FixedBinding::new(
            format!("{i}"),
            NumberShortcutButtonsAction::NumberSelect(i as usize - 1),
            id!(NumberShortcutButtons::ui_name()) & id!(NUMBER_SELECT_ENABLED),
        )]);
    }

    app.register_fixed_bindings([
        FixedBinding::new(
            "up",
            NumberShortcutButtonsAction::ArrowUp,
            id!(NumberShortcutButtons::ui_name()) & id!(KEYBOARD_NAVIGATION_ENABLED),
        ),
        FixedBinding::new(
            "down",
            NumberShortcutButtonsAction::ArrowDown,
            id!(NumberShortcutButtons::ui_name()) & id!(KEYBOARD_NAVIGATION_ENABLED),
        ),
        FixedBinding::new(
            "tab",
            NumberShortcutButtonsAction::ArrowDown,
            id!(NumberShortcutButtons::ui_name()) & id!(KEYBOARD_NAVIGATION_ENABLED),
        ),
        FixedBinding::new(
            "shift-tab",
            NumberShortcutButtonsAction::ArrowUp,
            id!(NumberShortcutButtons::ui_name()) & id!(KEYBOARD_NAVIGATION_ENABLED),
        ),
        FixedBinding::new(
            "enter",
            NumberShortcutButtonsAction::ActivateSelected,
            id!(NumberShortcutButtons::ui_name()) & id!(ENTER_ACTIVATION_ENABLED),
        ),
        FixedBinding::new(
            "numpadenter",
            NumberShortcutButtonsAction::ActivateSelected,
            id!(NumberShortcutButtons::ui_name()) & id!(ENTER_ACTIVATION_ENABLED),
        ),
    ]);
}

#[derive(Debug, Clone)]
pub enum NumberShortcutButtonsAction {
    RowHovered(usize),
    ListUnhovered,
    ButtonClicked(usize),
    NumberSelect(usize),
    ArrowUp,
    ArrowDown,
    ActivateSelected,
}

pub enum NumberShortcutButtonsEvent {}

pub type ButtonBuilder = Box<dyn Fn(bool, &warpui::AppContext) -> Button>;
pub type OnButtonClickFn = Box<dyn Fn(&mut ViewContext<NumberShortcutButtons>)>;

#[derive(Clone, Default)]
pub struct NumberShortcutButtonsConfig {
    enable_keyboard_navigation: bool,
    enable_enter_to_activate: bool,
    scroll_state: Option<ClippedScrollStateHandle>,
}

impl NumberShortcutButtonsConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_keyboard_navigation(mut self) -> Self {
        self.enable_keyboard_navigation = true;
        self
    }

    pub fn with_enter_to_activate(mut self, enabled: bool) -> Self {
        self.enable_enter_to_activate = enabled;
        self
    }

    pub fn with_scroll_state(mut self, scroll_state: ClippedScrollStateHandle) -> Self {
        self.scroll_state = Some(scroll_state);
        self
    }

    fn keyboard_navigation_enabled(&self) -> bool {
        self.enable_keyboard_navigation
    }

    fn enter_activation_enabled(&self) -> bool {
        self.enable_enter_to_activate && self.enable_keyboard_navigation
    }
}

pub struct NumberShortcutButtonBuilder {
    button_builder: ButtonBuilder,
    on_click: OnButtonClickFn,
}

impl NumberShortcutButtonBuilder {
    pub fn new(
        button_builder: impl Fn(bool, &warpui::AppContext) -> Button + 'static,
        on_click: impl Fn(&mut ViewContext<NumberShortcutButtons>) + 'static,
    ) -> Self {
        Self {
            button_builder: Box::new(button_builder),
            on_click: Box::new(on_click),
        }
    }
}

pub fn numbered_shortcut_button<A: warpui::Action + Clone + 'static>(
    number: usize,
    text_label: String,
    is_checked: bool,
    recommended: bool,
    use_markdown: bool,
    mouse_state: MouseStateHandle,
    action: A,
) -> NumberShortcutButtonBuilder {
    NumberShortcutButtonBuilder::new(
        move |is_selected, app| {
            build_numbered_button(
                number,
                build_text_button_content(&text_label, recommended, use_markdown, app),
                is_checked,
                is_selected,
                &mouse_state,
                app,
            )
        },
        move |ctx: &mut ViewContext<NumberShortcutButtons>| {
            ctx.dispatch_typed_action_deferred(action.clone());
        },
    )
}

pub fn inline_input_shortcut_button(
    number: usize,
    input_view: ViewHandle<super::compact_agent_input::CompactAgentInput>,
    mouse_state: MouseStateHandle,
) -> NumberShortcutButtonBuilder {
    NumberShortcutButtonBuilder::new(
        move |is_selected, app| {
            build_numbered_button(
                number,
                build_inline_input_content(&input_view),
                false,
                is_selected,
                &mouse_state,
                app,
            )
        },
        move |_ctx: &mut ViewContext<NumberShortcutButtons>| {
            // Click on the input row is a no-op; the text input handles its own interactions.
        },
    )
}

pub struct NumberShortcutButtons {
    button_builders: Vec<NumberShortcutButtonBuilder>,
    selected_button_index: Option<usize>,
    config: NumberShortcutButtonsConfig,
    self_handle: WeakViewHandle<Self>,
    mouse_state: MouseStateHandle,
}

impl NumberShortcutButtons {
    pub fn new_with_config(
        button_builders: Vec<NumberShortcutButtonBuilder>,
        selected_button_index: Option<usize>,
        config: NumberShortcutButtonsConfig,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let selected_button_index = selected_button_index
            .filter(|_| !button_builders.is_empty())
            .map(|index| index.min(button_builders.len() - 1));

        Self {
            button_builders,
            selected_button_index,
            config,
            self_handle: ctx.handle(),
            mouse_state: MouseStateHandle::default(),
        }
    }

    pub fn selected_button_index(&self) -> Option<usize> {
        self.selected_button_index
    }
    fn selected_button_position_id(&self) -> Option<String> {
        self.selected_button_index
            .map(|selected_button_index| self.button_position_id(selected_button_index))
    }

    fn button_position_id(&self, index: usize) -> String {
        format!(
            "number_shortcut_buttons_{}_{}",
            self.self_handle.id(),
            index
        )
    }

    fn has_descendent_focus(&self, app: &AppContext) -> bool {
        self.self_handle.window_id(app).is_some_and(|window_id| {
            !app.check_view_focused(window_id, &self.self_handle.id())
                && app.check_view_or_child_focused(window_id, &self.self_handle.id())
        })
    }

    fn keyboard_shortcuts_enabled(&self, app: &AppContext) -> bool {
        !self.has_descendent_focus(app)
    }

    fn select_prev(&mut self) {
        if self.button_builders.is_empty() {
            return;
        }
        let button_count = self.button_builders.len();

        self.selected_button_index = Some(match self.selected_button_index {
            // Arrow-up wraps from the first option back to the last option.
            Some(selected_button_index) => {
                (selected_button_index + button_count - 1) % button_count
            }
            None => 0,
        });
    }

    fn select_next(&mut self) {
        if self.button_builders.is_empty() {
            return;
        }
        let button_count = self.button_builders.len();

        self.selected_button_index = Some(match self.selected_button_index {
            // Arrow-down wraps from the last option back to the first option.
            Some(selected_button_index) => (selected_button_index + 1) % button_count,
            None => 0,
        });
    }

    fn scroll_selected_button_into_view(&self) {
        let Some(scroll_state) = self.config.scroll_state.as_ref() else {
            return;
        };
        let Some(position_id) = self.selected_button_position_id() else {
            return;
        };

        scroll_state.scroll_to_position(ScrollTarget {
            position_id,
            mode: ScrollToPositionMode::FullyIntoView,
        });
    }

    fn set_selected_button_index(&mut self, index: usize) -> bool {
        if index >= self.button_builders.len() || self.selected_button_index == Some(index) {
            return false;
        }

        self.selected_button_index = Some(index);
        self.scroll_selected_button_into_view();
        true
    }

    fn clear_selected_button_index(&mut self) -> bool {
        let did_update = self.selected_button_index.is_some();
        self.selected_button_index = None;
        did_update
    }

    fn activate_button_at(&self, index: usize, ctx: &mut ViewContext<Self>) {
        let Some(builder) = self.button_builders.get(index) else {
            return;
        };

        (builder.on_click)(ctx);
    }
}

impl View for NumberShortcutButtons {
    fn ui_name() -> &'static str {
        "NumberShortcutButtons"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        Hoverable::new(self.mouse_state.clone(), |_| {
            let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
            for (index, button_builder) in self.button_builders.iter().enumerate() {
                let is_selected = self.selected_button_index == Some(index);
                let button = (button_builder.button_builder)(is_selected, app);
                let mut hoverable = button.build();
                hoverable = hoverable.on_click(move |ctx, _app, _pos| {
                    ctx.dispatch_typed_action(NumberShortcutButtonsAction::ButtonClicked(index));
                });
                let hoverable = EventHandler::new(hoverable.finish())
                    .with_always_handle()
                    .on_mouse_in(
                        move |ctx, _, _| {
                            ctx.dispatch_typed_action(NumberShortcutButtonsAction::RowHovered(
                                index,
                            ));
                            DispatchEventResult::PropagateToParent
                        },
                        Some(MouseInBehavior {
                            fire_on_synthetic_events: false,
                            fire_when_covered: true,
                        }),
                    )
                    .finish();
                let margin_bottom = if index == self.button_builders.len() - 1 {
                    0.
                } else {
                    MARGIN_BETWEEN_BUTTONS
                };
                content.add_child(
                    SavePosition::new(
                        Container::new(hoverable)
                            .with_margin_bottom(margin_bottom)
                            .finish(),
                        &self.button_position_id(index),
                    )
                    .finish(),
                );
            }
            content.finish()
        })
        .on_hover(|is_hovered, ctx, _app, _position| {
            if !is_hovered {
                ctx.dispatch_typed_action(NumberShortcutButtonsAction::ListUnhovered);
            }
        })
        .finish()
    }

    fn keymap_context(&self, app: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();
        if !self.button_builders.is_empty() && self.keyboard_shortcuts_enabled(app) {
            context.set.insert(NUMBER_SELECT_ENABLED);
            if self.config.keyboard_navigation_enabled() {
                context.set.insert(KEYBOARD_NAVIGATION_ENABLED);
            }
            if self.config.enter_activation_enabled() {
                context.set.insert(ENTER_ACTIVATION_ENABLED);
            }
        }
        context
    }
}

impl TypedActionView for NumberShortcutButtons {
    type Action = NumberShortcutButtonsAction;

    fn handle_action(&mut self, action: &NumberShortcutButtonsAction, ctx: &mut ViewContext<Self>) {
        let should_update = match action {
            NumberShortcutButtonsAction::RowHovered(index) => {
                self.set_selected_button_index(*index)
            }
            NumberShortcutButtonsAction::ListUnhovered => self.clear_selected_button_index(),
            NumberShortcutButtonsAction::ButtonClicked(index) => {
                let did_update = self.set_selected_button_index(*index);
                self.activate_button_at(*index, ctx);
                did_update
            }
            NumberShortcutButtonsAction::NumberSelect(index) => {
                if *index >= self.button_builders.len() {
                    false
                } else {
                    let did_update = self.set_selected_button_index(*index);
                    self.activate_button_at(*index, ctx);
                    self.clear_selected_button_index() || did_update
                }
            }
            NumberShortcutButtonsAction::ArrowUp => {
                let previous_index = self.selected_button_index;
                self.select_prev();
                self.scroll_selected_button_into_view();
                previous_index != self.selected_button_index
            }
            NumberShortcutButtonsAction::ArrowDown => {
                let previous_index = self.selected_button_index;
                self.select_next();
                self.scroll_selected_button_into_view();
                previous_index != self.selected_button_index
            }
            NumberShortcutButtonsAction::ActivateSelected => {
                if let Some(selected_button_index) = self.selected_button_index {
                    self.activate_button_at(selected_button_index, ctx);
                }
                false
            }
        };

        if should_update {
            ctx.notify();
        }
    }
}

impl Entity for NumberShortcutButtons {
    type Event = NumberShortcutButtonsEvent;
}

#[cfg(test)]
#[path = "number_shortcut_buttons_tests.rs"]
mod tests;
