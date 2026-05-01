use std::sync::Arc;

use pathfinder_geometry::vector::vec2f;
use warpui::{
    elements::{
        Border, ChildAnchor, ChildView, Container, OffsetPositioning, ParentAnchor,
        ParentElement as _, ParentOffsetBounds, Stack,
    },
    AppContext, Element, Entity, EntityId, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;

use crate::ai::blocklist::agent_view::agent_input_footer::AgentInputButtonTheme;
use crate::ai::llms::{LLMPreferences, LLMPreferencesEvent};
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpEscapeKey, PropagateAndNoOpNavigationKeys,
    SingleLineEditorOptions, TextOptions,
};
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields, MenuVariant};
use crate::terminal::input::{MenuPositioning, MenuPositioningProvider};
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ButtonSize};
use warp_editor::editor::NavigationKey;

const ITEM_FONT_SIZE: f32 = 14.;

const MENU_HORIZONTAL_PADDING: f32 = 16.;

const ITEM_VERTICAL_PADDING: f32 = 4.;

const MENU_CONTENT_VERTICAL_PADDING: f32 = 4.;

const MENU_WIDTH: f32 = 320.;

const MENU_MAX_HEIGHT: f32 = 200.;

const ITEM_ICON_SIZE: f32 = 16.;

const SEARCH_FONT_SIZE: f32 = 14.;

const SEARCH_VERTICAL_PADDING: f32 = 4.;

// Extra space between the last menu item and the divider above the search
// footer. Combined with the item's own 4px bottom padding, this yields 8px
// of total breathing room above the divider line.
const SEARCH_FOOTER_TOP_MARGIN: f32 = 4.;

const SEARCH_PLACEHOLDER_TEXT: &str = "Search models";

const BUTTON_TOOLTIP: &str = "Choose agent model";

const NO_RESULTS_LABEL: &str = "No results";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelSelectorAction {
    ToggleMenu,
    SelectModel(crate::ai::llms::LLMId),
}

pub enum ModelSelectorEvent {
    MenuVisibilityChanged { open: bool },
}

pub struct ModelSelector {
    button: ViewHandle<ActionButton>,
    menu: ViewHandle<Menu<ModelSelectorAction>>,
    search_editor: ViewHandle<EditorView>,
    search_query: String,
    is_menu_open: bool,
    menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
    terminal_view_id: EntityId,
}

impl ModelSelector {
    pub fn new(
        menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
        terminal_view_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", AgentInputButtonTheme)
                .with_size(ButtonSize::AgentInputButton)
                .with_tooltip(BUTTON_TOOLTIP)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(ModelSelectorAction::ToggleMenu);
                })
        });

        let search_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(SEARCH_FONT_SIZE), appearance),
                    select_all_on_focus: true,
                    clear_selections_on_blur: true,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    propagate_and_no_op_escape_key: PropagateAndNoOpEscapeKey::PropagateFirst,
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text(SEARCH_PLACEHOLDER_TEXT, ctx);
            editor
        });
        ctx.subscribe_to_view(&search_editor, |me, _, event, ctx| {
            me.handle_search_editor_event(event, ctx);
        });

        let search_editor_for_footer = search_editor.clone();
        let menu = ctx.add_typed_action_view(move |_ctx| {
            let mut menu = Menu::new()
                .with_width(MENU_WIDTH)
                .with_drop_shadow()
                .with_menu_variant(MenuVariant::scrollable())
                .prevent_interaction_with_other_elements();
            menu.set_content_padding_overrides(
                Some(MENU_CONTENT_VERTICAL_PADDING),
                Some(MENU_CONTENT_VERTICAL_PADDING),
            );
            menu.set_height(MENU_MAX_HEIGHT);
            let editor_handle = search_editor_for_footer.clone();
            menu.set_pinned_footer_builder(move |app| render_search_footer(&editor_handle, app));
            menu
        });

        ctx.subscribe_to_view(&menu, |me, _, event, ctx| match event {
            MenuEvent::Close { .. } => {
                me.set_menu_visibility(false, ctx);
            }
            MenuEvent::ItemSelected | MenuEvent::ItemHovered => {}
        });

        ctx.subscribe_to_model(
            &LLMPreferences::handle(ctx),
            |me, _, event, ctx| match event {
                LLMPreferencesEvent::UpdatedActiveAgentModeLLM
                | LLMPreferencesEvent::UpdatedAvailableLLMs => {
                    me.refresh_button(ctx);
                    me.refresh_menu(ctx);
                }
                LLMPreferencesEvent::UpdatedActiveCodingLLM => {}
            },
        );

        ctx.subscribe_to_model(&Appearance::handle(ctx), |me, _, _, ctx| {
            me.refresh_menu(ctx);
        });

        let mut me = Self {
            button,
            menu,
            search_editor,
            search_query: String::new(),
            is_menu_open: false,
            menu_positioning_provider,
            terminal_view_id,
        };
        me.refresh_button(ctx);
        me.refresh_menu(ctx);
        me
    }

    pub fn is_menu_open(&self) -> bool {
        self.is_menu_open
    }

    pub fn open_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.set_menu_visibility(true, ctx);
    }

    fn set_menu_visibility(&mut self, is_open: bool, ctx: &mut ViewContext<Self>) {
        if self.is_menu_open == is_open {
            return;
        }
        self.is_menu_open = is_open;
        if is_open {
            ctx.focus(&self.search_editor);
        } else {
            self.search_editor.update(ctx, |editor, ctx| {
                editor.system_clear_buffer(true, ctx);
            });
            if !self.search_query.is_empty() {
                self.search_query.clear();
                self.refresh_menu(ctx);
            }
        }
        ctx.emit(ModelSelectorEvent::MenuVisibilityChanged { open: is_open });
        ctx.notify();
    }

    fn handle_search_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => {
                let new_query = self.search_editor.as_ref(ctx).buffer_text(ctx);
                if new_query != self.search_query {
                    self.search_query = new_query;
                    self.refresh_menu(ctx);
                }
            }
            EditorEvent::Navigate(NavigationKey::Up) => {
                self.menu.update(ctx, |menu, ctx| menu.select_previous(ctx));
            }
            EditorEvent::Navigate(NavigationKey::Down) => {
                self.menu.update(ctx, |menu, ctx| menu.select_next(ctx));
            }
            EditorEvent::Enter => {
                let selected_action =
                    self.menu
                        .as_ref(ctx)
                        .selected_item()
                        .and_then(|item| match item {
                            MenuItem::Item(fields) => fields.on_select_action().cloned(),
                            _ => None,
                        });
                if let Some(action) = selected_action {
                    <Self as TypedActionView>::handle_action(self, &action, ctx);
                }
            }
            EditorEvent::Escape => {
                self.set_menu_visibility(false, ctx);
            }
            _ => {}
        }
    }

    fn refresh_button(&mut self, ctx: &mut ViewContext<Self>) {
        let active = LLMPreferences::as_ref(ctx)
            .get_active_base_model(ctx, Some(self.terminal_view_id))
            .display_name
            .clone();
        self.button.update(ctx, |button, ctx| {
            button.set_label(active, ctx);
        });
    }

    fn refresh_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme();
        let border = Border::all(1.).with_border_color(internal_colors::neutral_4(theme));
        let hover_background: Fill = internal_colors::fg_overlay_2(theme);

        let llm_preferences = LLMPreferences::as_ref(ctx);
        let active_llm_id = llm_preferences
            .get_active_base_model(ctx, Some(self.terminal_view_id))
            .id
            .clone();

        let query = self.search_query.trim().to_lowercase();

        let mut items: Vec<MenuItem<ModelSelectorAction>> = llm_preferences
            .get_base_llm_choices_for_agent_mode()
            .filter_map(|llm| {
                let display_name = llm.menu_display_name();
                if !query.is_empty() && !display_name.to_lowercase().contains(&query) {
                    return None;
                }
                let icon = llm.provider.icon().unwrap_or(Icon::Oz);
                Some(MenuItem::Item(
                    MenuItemFields::new(display_name)
                        .with_icon(icon)
                        .with_icon_size_override(ITEM_ICON_SIZE)
                        .with_font_size_override(ITEM_FONT_SIZE)
                        .with_padding_override(ITEM_VERTICAL_PADDING, MENU_HORIZONTAL_PADDING)
                        .with_override_hover_background_color(hover_background)
                        .with_on_select_action(ModelSelectorAction::SelectModel(llm.id.clone()))
                        .with_disabled(llm.disable_reason.is_some()),
                ))
            })
            .collect();

        if items.is_empty() {
            let no_results_text_color = internal_colors::text_sub(theme, theme.surface_2());
            items.push(MenuItem::Item(
                MenuItemFields::new(NO_RESULTS_LABEL)
                    .with_font_size_override(ITEM_FONT_SIZE)
                    .with_padding_override(ITEM_VERTICAL_PADDING, MENU_HORIZONTAL_PADDING)
                    .with_override_text_color(no_results_text_color)
                    .with_no_interaction_on_hover(),
            ));
        }

        self.menu.update(ctx, |menu, ctx| {
            menu.set_border(Some(border));
            menu.set_items(items, ctx);
            menu.set_selected_by_action(&ModelSelectorAction::SelectModel(active_llm_id), ctx);
        });
    }

    fn menu_positioning(&self, app: &AppContext) -> OffsetPositioning {
        match self.menu_positioning_provider.menu_position(app) {
            MenuPositioning::BelowInputBox => OffsetPositioning::offset_from_parent(
                vec2f(0., 4.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::BottomRight,
                ChildAnchor::TopRight,
            ),
            MenuPositioning::AboveInputBox => OffsetPositioning::offset_from_parent(
                vec2f(0., -4.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::TopRight,
                ChildAnchor::BottomRight,
            ),
        }
    }
}

fn render_search_footer(
    search_editor: &ViewHandle<EditorView>,
    app: &AppContext,
) -> Box<dyn Element> {
    let theme = Appearance::as_ref(app).theme();

    let divider_color = internal_colors::fg_overlay_2(theme);

    Container::new(ChildView::new(search_editor).finish())
        .with_margin_top(SEARCH_FOOTER_TOP_MARGIN)
        .with_padding_left(MENU_HORIZONTAL_PADDING)
        .with_padding_right(MENU_HORIZONTAL_PADDING)
        .with_padding_top(SEARCH_VERTICAL_PADDING)
        .with_padding_bottom(SEARCH_VERTICAL_PADDING)
        .with_border(Border::top(1.).with_border_fill(divider_color))
        .finish()
}

impl Entity for ModelSelector {
    type Event = ModelSelectorEvent;
}

impl TypedActionView for ModelSelector {
    type Action = ModelSelectorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ModelSelectorAction::ToggleMenu => {
                let new_state = !self.is_menu_open;
                self.set_menu_visibility(new_state, ctx);
            }
            ModelSelectorAction::SelectModel(llm_id) => {
                let terminal_view_id = self.terminal_view_id;
                let id_for_update = llm_id.clone();
                LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
                    prefs.update_preferred_agent_mode_llm(&id_for_update, terminal_view_id, ctx);
                });
                self.set_menu_visibility(false, ctx);
            }
        }
    }
}

impl View for ModelSelector {
    fn ui_name() -> &'static str {
        "ModelSelector"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let mut stack = Stack::new();
        stack.add_child(ChildView::new(&self.button).finish());

        if self.is_menu_open {
            let positioning = self.menu_positioning(app);
            stack.add_positioned_overlay_child(ChildView::new(&self.menu).finish(), positioning);
        }

        stack.finish()
    }
}
