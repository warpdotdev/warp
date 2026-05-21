//! This module contains the code for the editable accept autosuggestion keybinding
//! shown inline in the input.
use crate::appearance::Appearance;
use crate::editor::ACCEPT_AUTOSUGGESTION_KEYBINDING_NAME;
use crate::menu::{Menu, MenuItemFields};
use crate::settings_view::keybindings::{KeybindingChangedEvent, KeybindingChangedNotifier};
use crate::terminal::input::OPEN_COMPLETIONS_KEYBINDING_NAME;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::util::bindings::{
    keybinding_name_to_keystroke, reset_keybinding_to_default, set_custom_keybinding,
};
use crate::workspace::WorkspaceAction;
use lazy_static::lazy_static;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::Fill;
use warpui::elements::{Border, ChildView, Flex, ParentElement};
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Radius, DEFAULT_UI_LINE_HEIGHT_RATIO,
};
use warpui::keymap::Keystroke;
use warpui::platform::Cursor;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::ui_components::keyboard_shortcut::KeyboardShortcut;
use warpui::ViewContext;
use warpui::{
    elements::{
        ChildAnchor, CornerRadius, Element, Hoverable, MouseStateHandle, OffsetPositioning,
        ParentAnchor, ParentOffsetBounds, Stack,
    },
    AppContext, SingletonEntity,
};
use warpui::{Entity, TypedActionView, View, ViewHandle};

use super::EditorElement;

pub const AUTOSUGGESTION_HINT_MINIMUM_HEIGHT: f32 = 12.;

lazy_static! {
    static ref TAB_KEYSTROKE: Keystroke = Keystroke {
        key: "tab".to_string(),
        ..Default::default()
    };
    static ref CTRL_SPACE_KEYSTROKE: Keystroke = Keystroke {
        key: " ".into(),
        ctrl: true,
        ..Default::default()
    };
}

pub struct AcceptAutosuggestionKeybinding {
    is_menu_open: bool,
    select_keybinding_menu: ViewHandle<Menu<AcceptAutosuggestionKeybindingAction>>,

    autosuggestion_hint_mouse_handle: MouseStateHandle,
    accept_autosuggestion_keybinding: Option<Keystroke>,
}

pub enum AcceptAutosuggestionKeybindingEvent {
    SetAcceptAutosuggestionKeybinding {},
    OpenSettingsForCustomKeybinding,
}

impl Entity for AcceptAutosuggestionKeybinding {
    type Event = AcceptAutosuggestionKeybindingEvent;
}

impl AcceptAutosuggestionKeybinding {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let accept_autosuggestion_keybinding =
            keybinding_name_to_keystroke(ACCEPT_AUTOSUGGESTION_KEYBINDING_NAME, ctx);
        let select_keybinding_menu = ctx.add_typed_action_view(|ctx| {
            Menu::new().with_border(
                Border::all(1.)
                    .with_border_color(blended_colors::neutral_4(Appearance::as_ref(ctx).theme())),
            )
        });
        let right_arrow = Keystroke::parse("right").expect("can parse keystroke");
        let tab = Keystroke::parse("tab").expect("can parse keystroke");
        let shift_right_arrow = Keystroke::parse("shift-right").expect("can parse keystroke");
        let menu_items: Vec<crate::menu::MenuItem<AcceptAutosuggestionKeybindingAction>> = vec![
            MenuItemFields::new(right_arrow.displayed())
                .with_on_select_action(
                    AcceptAutosuggestionKeybindingAction::SetAcceptAutosuggestionKeybinding {
                        keystroke: right_arrow,
                    },
                )
                .into_item(),
            MenuItemFields::new(tab.displayed().to_lowercase())
                .with_on_select_action(
                    AcceptAutosuggestionKeybindingAction::SetAcceptAutosuggestionKeybinding {
                        keystroke: tab,
                    },
                )
                .into_item(),
            MenuItemFields::new(shift_right_arrow.displayed().to_lowercase())
                .with_on_select_action(
                    AcceptAutosuggestionKeybindingAction::SetAcceptAutosuggestionKeybinding {
                        keystroke: shift_right_arrow,
                    },
                )
                .into_item(),
            MenuItemFields::new(crate::i18n::tr_static(ctx, "Custom..."))
                .with_on_select_action(
                    AcceptAutosuggestionKeybindingAction::OpenSettingsForCustomKeybinding,
                )
                .into_item(),
        ];
        select_keybinding_menu.update(ctx, |menu, ctx| {
            menu.set_items(menu_items, ctx);
        });
        ctx.subscribe_to_view(&select_keybinding_menu, |me, _, event, ctx| {
            if let crate::menu::Event::Close { .. } = event {
                me.close_menu(ctx);
            }
        });

        // Subscribe to when the accept autosuggestion keybinding changes.
        let notifier = KeybindingChangedNotifier::handle(ctx);
        ctx.subscribe_to_model(&notifier, |me, _, event, ctx| match event {
            KeybindingChangedEvent::BindingChanged {
                binding_name,
                new_trigger,
            } => {
                if binding_name == ACCEPT_AUTOSUGGESTION_KEYBINDING_NAME {
                    me.accept_autosuggestion_keybinding = new_trigger.clone();
                    me.update_menu_selected_item(ctx);
                }
            }
        });

        let mut me = Self {
            is_menu_open: false,
            select_keybinding_menu,
            autosuggestion_hint_mouse_handle: Default::default(),
            accept_autosuggestion_keybinding,
        };
        me.update_menu_selected_item(ctx);
        me
    }

    pub fn close_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_menu_open = false;
        ctx.notify();
    }

    /// Updates the menu selected item based on the current accept autosuggestion keybinding.
    fn update_menu_selected_item(&mut self, ctx: &mut ViewContext<Self>) {
        let accept_autosuggestion_keybinding_displayed = self
            .accept_autosuggestion_keybinding
            .as_ref()
            .map(|keystroke| keystroke.displayed().to_lowercase());
        self.select_keybinding_menu.update(ctx, |menu, ctx| {
            if let Some(accept_autosuggestion_keybinding_displayed) =
                accept_autosuggestion_keybinding_displayed
            {
                let found =
                    menu.set_selected_by_name(accept_autosuggestion_keybinding_displayed, ctx);
                // If the keybinding is not one of our default options, select the "Custom..." item.
                if !found {
                    menu.set_selected_by_name(crate::i18n::tr_static(ctx, "Custom..."), ctx);
                }
            } else {
                // If the keybinding is not set, we show right arrow which always works.
                menu.set_selected_by_action(
                    &AcceptAutosuggestionKeybindingAction::SetAcceptAutosuggestionKeybinding {
                        keystroke: Keystroke::parse("right").expect("can parse keystroke"),
                    },
                    ctx,
                );
            }
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptAutosuggestionKeybindingAction {
    SetAcceptAutosuggestionKeybinding { keystroke: Keystroke },
    OpenSettingsForCustomKeybinding,
    OpenMenu,
}

impl TypedActionView for AcceptAutosuggestionKeybinding {
    type Action = AcceptAutosuggestionKeybindingAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AcceptAutosuggestionKeybindingAction::SetAcceptAutosuggestionKeybinding {
                keystroke,
            } => {
                // When set to right arrow, we reset to the default (clears keybinding)
                // so that it's still possible to use right arrow to navigate the input when the cursor isn't at the end of the line.
                if keystroke.displayed() == "→" {
                    reset_keybinding_to_default(ACCEPT_AUTOSUGGESTION_KEYBINDING_NAME, ctx);
                } else {
                    // Whatever keybinding is set here will always work regardless of where the cursor is.
                    set_custom_keybinding(ACCEPT_AUTOSUGGESTION_KEYBINDING_NAME, keystroke, ctx);
                }

                let open_completions_keystroke =
                    keybinding_name_to_keystroke(OPEN_COMPLETIONS_KEYBINDING_NAME, ctx);
                // If we're setting accept autosuggestion to tab and that conflicts with open completions,
                // set open completions to ctrl-space.
                if *keystroke == *TAB_KEYSTROKE
                    && open_completions_keystroke
                        .as_ref()
                        .is_some_and(|keystroke| *keystroke == *TAB_KEYSTROKE)
                {
                    set_custom_keybinding(
                        OPEN_COMPLETIONS_KEYBINDING_NAME,
                        &CTRL_SPACE_KEYSTROKE,
                        ctx,
                    );
                } else if *keystroke != *TAB_KEYSTROKE
                    && open_completions_keystroke
                        .is_some_and(|keystroke| keystroke == *CTRL_SPACE_KEYSTROKE)
                {
                    // If we're setting accept autosuggestion to anything other than tab, switch open completions back to tab if it was set to ctrl-space above.
                    // Note that we can't tell if the user explicitly set it to ctrl-space vs we did it as a default above.
                    reset_keybinding_to_default(OPEN_COMPLETIONS_KEYBINDING_NAME, ctx);
                }
            }
            AcceptAutosuggestionKeybindingAction::OpenSettingsForCustomKeybinding => ctx
                .dispatch_typed_action(&WorkspaceAction::ConfigureKeybindingSettings {
                    keybinding_name: Some("Accept Autosuggestion".to_owned()),
                }),
            AcceptAutosuggestionKeybindingAction::OpenMenu => {
                self.is_menu_open = true;
            }
        };
        ctx.notify();
    }
}

impl View for AcceptAutosuggestionKeybinding {
    fn ui_name() -> &'static str {
        "AcceptAutosuggestionKeybinding"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);
        // Because the icon's origin is NOT the top of the line but the top of the cursor,
        // we should render it with line height ratio no larger than DEFAULT_UI_LINE_HEIGHT_RATIO.
        // With larger line height ratios, there wouldn't be enough space in the icon to render the text.
        // But we do need to account for smaller line height ratios so the text can be rendered in the smaller space.
        let line_height_ratio = appearance
            .line_height_ratio()
            .min(DEFAULT_UI_LINE_HEIGHT_RATIO);
        // We want the keybinding icon to be the same height as the cursor in the input.
        let height =
            EditorElement::cursor_height(appearance.monospace_font_size(), line_height_ratio)
                .max(AUTOSUGGESTION_HINT_MINIMUM_HEIGHT);
        let disabled_color = blended_colors::semantic_text_disabled(appearance.theme());
        // If there's no keybinding set, right arrow always works.
        let keystroke = self
            .accept_autosuggestion_keybinding
            .clone()
            .unwrap_or(Keystroke::parse("right").expect("can parse keystroke"));
        let border_width = 1.;

        // The stack contains the editable keybinding, tooltip that shows upon mouse hover,
        // and menu on click.
        let mut stack = Stack::new();
        if self.is_menu_open {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.select_keybinding_menu).finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., -1.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopLeft,
                    ChildAnchor::BottomLeft,
                ),
            );
        }

        let is_menu_open = self.is_menu_open;
        Hoverable::new(self.autosuggestion_hint_mouse_handle.clone(), |state| {
            // Colors are inverted when hovered or menu is open.
            let (font_color, background_color) = if is_menu_open || state.is_hovered() {
                (
                    appearance.theme().background().into(),
                    Some(blended_colors::semantic_text_disabled(appearance.theme())),
                )
            } else {
                (
                    blended_colors::semantic_text_disabled(appearance.theme()),
                    None,
                )
            };
            let font_size = appearance.monospace_font_size() - 2.;
            // We use UI font family for the keyboard shortcut even though it's in the input
            // because monospace font family is configurable, and might not handle symbols like
            // modifiers and arrows well.
            let keystroke = KeyboardShortcut::new(
                &keystroke,
                UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_size: Some(font_size),
                    font_color: Some(font_color),
                    ..Default::default()
                },
            )
            .lowercase_modifier()
            .text_only()
            .with_line_height_ratio(line_height_ratio)
            .build()
            .finish();

            let height_without_border = height - border_width * 2.;
            let chevron_down = Container::new(
                ConstrainedBox::new(
                    Icon::ArrowDropDown
                        .to_warpui_icon(Fill::Solid(font_color))
                        .finish(),
                )
                .with_height(height_without_border)
                .with_width(height_without_border)
                .finish(),
            )
            .finish();

            let mut editable_keystroke = Container::new(
                Flex::row()
                    .with_children([keystroke, chevron_down])
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .finish(),
            )
            // Padding on the left for the keyboard shortcut.
            // The arrow down icon already has its own padding so no need on the right.
            .with_padding_left(4.)
            .with_border(Border::all(border_width).with_border_color(disabled_color))
            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(25.)));

            if let Some(background_color) = background_color {
                editable_keystroke = editable_keystroke.with_background_color(background_color);
            }

            let editable_keystroke_element = ConstrainedBox::new(editable_keystroke.finish())
                .with_max_height(height)
                .finish();

            stack.add_child(editable_keystroke_element);

            // Add tooltip on hover.
            if !is_menu_open && state.is_hovered() {
                let tool_tip = appearance
                    .ui_builder()
                    .autosuggestion_tool_tip(
                        crate::i18n::tr_static(ctx, "Change keybinding").into(),
                    )
                    .build()
                    .finish();
                stack.add_positioned_overlay_child(
                    tool_tip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -4.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopLeft,
                        ChildAnchor::BottomLeft,
                    ),
                );
            }
            stack.finish()
        })
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(AcceptAutosuggestionKeybindingAction::OpenMenu);
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }
}
