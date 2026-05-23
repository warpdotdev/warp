use std::cmp;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;

use fuzzy_match::{match_indices_case_insensitive, FuzzyMatchResult};
use instant::Instant;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::builder::MIN_FONT_SIZE;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warp_editor::editor::NavigationKey;
use warpui::clipboard::ClipboardContent;
use warpui::color::ColorU;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, Dismiss, DispatchEventResult, DropShadow, Empty,
    EventHandler, Flex, Highlight, Hoverable, MainAxisAlignment, MainAxisSize, MouseInBehavior,
    MouseStateHandle, OffsetPositioning, ParentElement, PositionedElementAnchor,
    PositionedElementOffsetBounds, Radius, SavePosition, ScrollStateHandle, Scrollable,
    ScrollableElement, ScrollbarWidth, Shrinkable, Stack, Text, UniformList, UniformListState,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::FixedBinding;
use warpui::r#async::Timer;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::units::Pixels;
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity as _, TypedActionView, View,
    ViewContext, ViewHandle, WindowId,
};

use crate::ai::cloud_environments::CloudAmbientAgentEnvironment;
use crate::cloud_object::model::generic_string_model::StringModel;
use crate::cloud_object::CloudObjectLookup as _;
use crate::editor::{
    EditorOptions, EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, TextOptions,
};
use crate::server::ids::{ClientId, HashableId, ServerId, SyncId};
use crate::ui_components::icons::Icon;
use crate::view_components::copyable_text_field::{
    render_copyable_text_field, CopyButtonPlacement, CopyableTextFieldConfig,
    COPY_FEEDBACK_DURATION,
};

/// Trait for items that can be displayed in a generic menu
pub trait GenericMenuItem: Debug + 'static {
    /// Enable downcasting to concrete types
    fn as_any(&self) -> &dyn std::any::Any;

    /// Display name for the menu item
    fn name(&self) -> String;

    /// Icon to display for the menu item (None for no icon)
    fn icon(&self, _app: &AppContext) -> Option<Icon>;

    /// Data associated with this menu item action
    fn action_data(&self) -> String;

    /// Optional element to render on the right side of the menu item
    fn right_side_element(&self, _app: &AppContext) -> Option<Box<dyn Element>> {
        None
    }
}

#[derive(Debug, Clone)]
pub struct FixedFooter {
    action_item: Arc<dyn GenericMenuItem>,
    mouse_state: MouseStateHandle,
}

impl FixedFooter {
    pub fn new(action_item: Arc<dyn GenericMenuItem>) -> Self {
        Self {
            action_item,
            mouse_state: Default::default(),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ChipMenuType {
    Directories,
    Branches,
    CodeReview,
    Environments,
}

const LABEL_HORIZONTAL_PADDING: f32 = 14.;
const SEARCH_INPUT_HORIZONTAL_PADDING: f32 = 8.;
const LABEL_VERTICAL_PADDING: f32 = 5.;
const MENU_VERTICAL_PADDING: f32 = 9.;
const MENU_WIDTH: f32 = 360.;

// Environments menu sizing from Figma mock.
const ENV_MENU_WIDTH: f32 = 321.;
const ENV_MENU_MAX_HEIGHT: f32 = 200.;
const ENV_MENU_VERTICAL_PADDING: f32 = 4.;
const ENV_MENU_ITEM_HORIZONTAL_PADDING: f32 = 16.;
const ENV_MENU_ITEM_VERTICAL_PADDING: f32 = 4.;
const ENV_MENU_ICON_SIZE: f32 = 16.;
const ENV_MENU_ICON_SLOT_SIZE: f32 = 16.;
const ENV_MENU_ITEM_FONT_SIZE: f32 = 14.;
const ENV_MENU_SEARCH_VERTICAL_PADDING: f32 = 4.;
// Bottom padding under the search field. The model selector's bottom padding
// is effectively `SEARCH_VERTICAL_PADDING (4) + MENU_CONTENT_VERTICAL_PADDING
// (4) = 8` because its `Menu` wraps the pinned footer in another 4px of
// content padding. We don't have that wrapper, so we bake the same 8px
// directly into the footer container.
const ENV_MENU_SEARCH_BOTTOM_PADDING: f32 = 8.;
const ENV_MENU_SEARCH_FOOTER_TOP_MARGIN: f32 = 4.;

// Environments sidecar sizing from Figma mock.
const ENV_SIDE_CAR_WIDTH: f32 = 320.;
const ENV_SIDE_CAR_HEIGHT: f32 = 108.;
const ENV_SIDE_CAR_HORIZONTAL_GAP: f32 = 1.;
const ENV_SIDE_CAR_PADDING: f32 = 12.;
const ENV_SIDE_CAR_ROW_GAP: f32 = 8.;
const ENV_SIDE_CAR_ICON_LABEL_GAP: f32 = 4.;
const ENV_SIDE_CAR_ICON_SIZE: f32 = 12.;
const ENV_SIDE_CAR_COPY_BUTTON_SIZE: f32 = 16.;
const ENV_SIDE_CAR_OUTER_RADIUS: f32 = 6.;
const ENV_SIDE_CAR_INNER_RADIUS: f32 = 4.;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new(
            "up",
            DisplayChipMenuAction::SelectUp,
            id!(DisplayChipMenu::ui_name()),
        ),
        FixedBinding::new(
            "down",
            DisplayChipMenuAction::SelectDown,
            id!(DisplayChipMenu::ui_name()),
        ),
        FixedBinding::new(
            "escape",
            DisplayChipMenuAction::Close,
            id!(DisplayChipMenu::ui_name()),
        ),
        FixedBinding::new(
            "enter",
            DisplayChipMenuAction::SelectEnter,
            id!(DisplayChipMenu::ui_name()),
        ),
    ]);
}

#[derive(Debug, Clone)]
struct FilteredMenuItem {
    item: Arc<dyn GenericMenuItem>,
    match_result: Option<FuzzyMatchResult>,
}

#[derive(Clone, Debug)]
struct EnvironmentSidecarData {
    name: String,
    id: String,
    image: String,
    repos_text: String,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum EnvironmentSidecarSide {
    Left,
    Right,
}

/// Builds an optional synthetic menu item from the current search query.
///
/// When set, [`DisplayChipMenu`] calls the builder on every search-query
/// change. If the builder returns `Some(item)` and no existing menu item
/// already has the same name (compared ASCII case-insensitively), the
/// returned item is prepended to the filtered results so the user can act on
/// the unmatched query (for example, "Create new branch <name>"). The
/// builder itself is responsible for validating the query (e.g. rejecting
/// empty / invalid inputs) and returning `None` when no synthetic item
/// should be offered.
pub type CreateItemFromQueryFn =
    dyn Fn(&str) -> Option<Arc<dyn GenericMenuItem>> + Send + Sync + 'static;

/// Returns whether `query` matches any of `item_names`, ignoring ASCII case.
///
/// Used by [`DisplayChipMenu::update_filtered_items`] to suppress the
/// "create from query" affordance when an existing item already covers the
/// query. The comparison is case-insensitive on purpose: case-insensitive
/// filesystems (the default on macOS and Windows) treat refs like `main` and
/// `Main` as the same branch, so offering "Create new branch \"Main\"" while
/// `main` already exists would just hand the user a `branch already exists`
/// failure from git.
fn query_matches_existing_name<I, S>(item_names: I, query: &str) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    item_names
        .into_iter()
        .any(|name| name.as_ref().eq_ignore_ascii_case(query))
}

pub struct DisplayChipMenu {
    list_state: UniformListState,
    scroll_state: ScrollStateHandle,
    menu_items: Vec<Arc<dyn GenericMenuItem>>,
    filtered_items: Vec<FilteredMenuItem>,
    selected_index: usize,
    is_footer_selected: bool,
    fixed_footer: Option<FixedFooter>,
    search_input: Option<ViewHandle<EditorView>>,
    search_query: String,
    chip_menu_type: ChipMenuType,
    /// When set, the menu offers a synthetic "create from query" item whenever
    /// the user's query doesn't exactly match an existing item. See
    /// [`CreateItemFromQueryFn`].
    create_item_from_query: Option<Arc<CreateItemFromQueryFn>>,

    // Environment sidecar state
    window_id: WindowId,
    env_sidecar_copy_id_mouse_state: MouseStateHandle,
    env_sidecar_copy_image_mouse_state: MouseStateHandle,
    env_sidecar_copy_feedback_times: HashMap<String, Instant>,
    env_sidecar_scroll_state: ClippedScrollStateHandle,
}

#[derive(Debug, Clone)]
pub enum DisplayChipMenuAction {
    SelectItem { index: usize },
    Select { index: usize },
    SelectUp,
    SelectDown,
    SelectEnter,
    SelectFixedFooterOption,
    CopyEnvironmentSidecarField { key: String, value: String },
    Close,
}

impl DisplayChipMenu {
    fn menu_width(&self) -> f32 {
        match self.chip_menu_type {
            ChipMenuType::Environments => ENV_MENU_WIDTH,
            ChipMenuType::Directories | ChipMenuType::Branches | ChipMenuType::CodeReview => {
                MENU_WIDTH
            }
        }
    }

    fn menu_item_horizontal_padding(&self) -> f32 {
        match self.chip_menu_type {
            ChipMenuType::Environments => ENV_MENU_ITEM_HORIZONTAL_PADDING,
            ChipMenuType::Directories | ChipMenuType::Branches | ChipMenuType::CodeReview => {
                LABEL_HORIZONTAL_PADDING
            }
        }
    }

    fn menu_item_vertical_padding(&self) -> f32 {
        match self.chip_menu_type {
            ChipMenuType::Environments => ENV_MENU_ITEM_VERTICAL_PADDING,
            ChipMenuType::Directories | ChipMenuType::Branches | ChipMenuType::CodeReview => {
                LABEL_VERTICAL_PADDING
            }
        }
    }

    fn menu_vertical_padding(&self) -> f32 {
        match self.chip_menu_type {
            ChipMenuType::Environments => ENV_MENU_VERTICAL_PADDING,
            ChipMenuType::Directories | ChipMenuType::Branches | ChipMenuType::CodeReview => {
                MENU_VERTICAL_PADDING
            }
        }
    }

    fn figma_menu_drop_shadow() -> DropShadow {
        DropShadow::default()
    }

    pub fn new<T: GenericMenuItem>(
        menu_items: Vec<T>,
        fixed_footer_option: Option<FixedFooter>,
        chip_menu_type: ChipMenuType,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let search_input = match chip_menu_type {
            ChipMenuType::Directories | ChipMenuType::Branches | ChipMenuType::Environments => {
                Some(ctx.add_typed_action_view(|ctx| {
                    let appearance = Appearance::handle(ctx).as_ref(ctx);

                    let text_options = match chip_menu_type {
                        ChipMenuType::Environments => {
                            TextOptions::ui_text(Some(ENV_MENU_ITEM_FONT_SIZE), appearance)
                        }
                        ChipMenuType::Directories
                        | ChipMenuType::Branches
                        | ChipMenuType::CodeReview => {
                            let ui_font_family = appearance.ui_font_family();
                            let mut options = TextOptions::ui_font_size(appearance);
                            options.font_family_override = Some(ui_font_family);
                            options
                        }
                    };

                    let options = EditorOptions {
                        autogrow: false,
                        soft_wrap: false,
                        single_line: true,
                        text: text_options,
                        propagate_and_no_op_vertical_navigation_keys:
                            PropagateAndNoOpNavigationKeys::Always,
                        ..Default::default()
                    };
                    let mut editor = EditorView::new(options, ctx);
                    let placeholder_text = match chip_menu_type {
                        ChipMenuType::Directories => "Search directories...",
                        ChipMenuType::Branches => "Search branches...",
                        ChipMenuType::Environments => "Search environments...",
                        ChipMenuType::CodeReview => {
                            unreachable!("search input should not be constructed")
                        }
                    };
                    editor.set_placeholder_text(placeholder_text, ctx);
                    editor
                }))
            }
            ChipMenuType::CodeReview => None,
        };

        // Subscribe to editor changes to update search query (only if search input exists)
        if let Some(ref search_input_handle) = search_input {
            ctx.subscribe_to_view(
                search_input_handle,
                |menu, _editor, event, ctx| match event {
                    EditorEvent::Edited(_) => {
                        if let Some(ref search_input) = menu.search_input {
                            let new_query = search_input
                                .read(ctx, |editor, ctx| editor.buffer_text(ctx).to_string());
                            if new_query != menu.search_query {
                                menu.update_search_query(new_query, ctx);
                            }
                        }
                    }
                    EditorEvent::Escape => {
                        menu.close(ctx);
                    }
                    EditorEvent::Navigate(NavigationKey::Up) => {
                        menu.select_prev(ctx);
                    }
                    EditorEvent::Navigate(NavigationKey::Down) => {
                        menu.select_next(ctx);
                    }
                    EditorEvent::Enter => {
                        menu.select_enter(ctx);
                    }
                    _ => {}
                },
            );
        }

        let menu_items: Vec<Arc<dyn GenericMenuItem>> = menu_items
            .into_iter()
            .map(|value| {
                let arc: Arc<dyn GenericMenuItem> = Arc::new(value);
                arc
            })
            .collect();

        let filtered_items: Vec<FilteredMenuItem> = menu_items
            .iter()
            .map(|item| FilteredMenuItem {
                item: item.clone(),
                match_result: None,
            })
            .collect();

        // Always start selection at the top (first item) for consistent behavior
        let initial_selected_index = 0;

        Self {
            list_state: Default::default(),
            scroll_state: Default::default(),
            menu_items,
            filtered_items,
            selected_index: initial_selected_index,
            fixed_footer: fixed_footer_option,
            is_footer_selected: false,
            search_input,
            search_query: String::new(),
            chip_menu_type,
            create_item_from_query: None,

            window_id: ctx.window_id(),
            env_sidecar_copy_id_mouse_state: Default::default(),
            env_sidecar_copy_image_mouse_state: Default::default(),
            env_sidecar_copy_feedback_times: HashMap::new(),
            env_sidecar_scroll_state: Default::default(),
        }
    }

    /// Register a builder that produces a synthetic top-of-list item for
    /// otherwise-unmatched search queries. See [`CreateItemFromQueryFn`].
    pub fn with_create_item_from_query(mut self, builder: Arc<CreateItemFromQueryFn>) -> Self {
        self.create_item_from_query = Some(builder);
        self
    }

    pub fn reset_selected_index(&mut self) {
        if self.filtered_items.is_empty() && self.fixed_footer.is_some() {
            self.is_footer_selected = true;
            return;
        }
        self.selected_index = 0;
        self.is_footer_selected = false;
        self.env_sidecar_scroll_state.scroll_to(Pixels::zero());
    }

    /// Update the menu items and reset the selected index
    pub fn update_menu_items<T: GenericMenuItem>(
        &mut self,
        new_items: Vec<T>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.menu_items = new_items
            .into_iter()
            .map(|value| {
                let arc: Arc<dyn GenericMenuItem> = Arc::new(value);
                arc
            })
            .collect();
        self.update_filtered_items();
        self.reset_selected_index();

        // Scroll to the selected item
        if !self.filtered_items.is_empty() {
            self.list_state.scroll_to(self.selected_index);
        }

        ctx.notify();
    }

    fn update_filtered_items(&mut self) {
        if self.search_query.is_empty() {
            // No search query - show all items
            self.filtered_items = self
                .menu_items
                .iter()
                .map(|item| FilteredMenuItem {
                    item: item.clone(),
                    match_result: None,
                })
                .collect();
            return;
        }

        // Filter items based on search query
        self.filtered_items = self
            .menu_items
            .iter()
            .filter_map(|item| {
                let item_name = item.name();
                match_indices_case_insensitive(&item_name, &self.search_query).map(|match_result| {
                    FilteredMenuItem {
                        item: item.clone(),
                        match_result: Some(match_result),
                    }
                })
            })
            .collect();

        // Sort by match score (higher scores first)
        self.filtered_items.sort_by(|a, b| {
            let score_a = a.match_result.as_ref().map(|r| r.score).unwrap_or(0);
            let score_b = b.match_result.as_ref().map(|r| r.score).unwrap_or(0);
            score_b.cmp(&score_a)
        });

        // Offer a synthetic top-of-list "create from query" item when the
        // current query has no exact match against an existing item. This is
        // what powers the "Create new branch …" affordance in the branch
        // switcher.
        if let Some(builder) = self.create_item_from_query.as_ref() {
            let trimmed = self.search_query.trim();
            let already_matches_existing = query_matches_existing_name(
                self.menu_items.iter().map(|item| item.name()),
                trimmed,
            );
            if !already_matches_existing {
                if let Some(synthetic) = builder(trimmed) {
                    self.filtered_items.insert(
                        0,
                        FilteredMenuItem {
                            item: synthetic,
                            match_result: None,
                        },
                    );
                }
            }
        }
    }

    pub fn update_search_query(&mut self, query: String, ctx: &mut ViewContext<Self>) {
        self.search_query = query;
        self.update_filtered_items();

        // Always start at the top after filtering for consistent behavior
        self.reset_selected_index();
        if !self.filtered_items.is_empty() {
            self.list_state.scroll_to(self.selected_index);
        }

        ctx.notify();
    }

    fn select_item(&mut self, item: Arc<dyn GenericMenuItem>, ctx: &mut ViewContext<Self>) {
        ctx.emit(PromptDisplayMenuEvent::MenuAction(GenericMenuEvent {
            action_item: item.clone(),
        }));
        ctx.notify();
    }

    fn select(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if self.selected_index != index {
            self.selected_index = index;
            self.env_sidecar_scroll_state.scroll_to(Pixels::zero());
        }
        ctx.notify();
    }

    pub fn select_index(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if index >= self.filtered_items.len() {
            return;
        }
        self.is_footer_selected = false;
        self.select(index, ctx);
        self.list_state.scroll_to(self.selected_index);
    }

    fn is_footer_selected(&self) -> bool {
        self.is_footer_selected
            || self
                .fixed_footer
                .as_ref()
                .is_some_and(|f| f.mouse_state.lock().is_ok_and(|state| state.is_hovered()))
    }

    fn select_prev(&mut self, ctx: &mut ViewContext<Self>) {
        if self.filtered_items.is_empty() {
            return;
        }
        let has_footer = self.fixed_footer.is_some();

        if self.selected_index == 0 {
            if has_footer && !self.is_footer_selected() {
                self.is_footer_selected = true;
            } else {
                self.is_footer_selected = false;
                self.selected_index = self.filtered_items.len() - 1;
                self.env_sidecar_scroll_state.scroll_to(Pixels::zero());
            }
        } else {
            self.is_footer_selected = false;
            self.selected_index -= 1;
            self.env_sidecar_scroll_state.scroll_to(Pixels::zero());
        }
        self.list_state.scroll_to(self.selected_index);
        ctx.notify();
    }

    fn select_next(&mut self, ctx: &mut ViewContext<Self>) {
        if self.filtered_items.is_empty() {
            return;
        }
        let has_footer = self.fixed_footer.is_some();

        self.selected_index += 1;
        if self.is_footer_selected() {
            self.is_footer_selected = false;
            self.selected_index = 0;
            self.env_sidecar_scroll_state.scroll_to(Pixels::zero());
        } else if self.selected_index >= self.filtered_items.len() {
            self.selected_index = 0;
            self.env_sidecar_scroll_state.scroll_to(Pixels::zero());
            if has_footer && !self.is_footer_selected() {
                self.is_footer_selected = true;
            }
        } else {
            self.env_sidecar_scroll_state.scroll_to(Pixels::zero());
        }
        self.list_state.scroll_to(self.selected_index);
        ctx.notify();
    }

    fn select_enter(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_footer_selected() {
            self.select_fixed_footer_option(ctx);
            return;
        }

        if self.selected_index < self.filtered_items.len() {
            let item = self.filtered_items[self.selected_index].item.clone();
            self.select_item(item, ctx);
        }
    }

    fn select_fixed_footer_option(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(footer_option) = &self.fixed_footer {
            ctx.emit(PromptDisplayMenuEvent::MenuAction(GenericMenuEvent {
                action_item: footer_option.action_item.clone(),
            }));
            ctx.notify();
        }
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(PromptDisplayMenuEvent::CloseMenu);
        ctx.notify();
    }

    fn should_show_environment_sidecar(&self) -> bool {
        self.chip_menu_type == ChipMenuType::Environments
            && !self.is_footer_selected()
            && self.selected_index < self.filtered_items.len()
    }

    fn parse_sync_id_lossy(s: &str) -> SyncId {
        if let Some(hashed) = ClientId::from_hash(s) {
            SyncId::ClientId(hashed)
        } else {
            SyncId::ServerId(ServerId::from_string_lossy(s))
        }
    }

    fn environment_sidecar_data(&self, app: &AppContext) -> Option<EnvironmentSidecarData> {
        if !self.should_show_environment_sidecar() {
            return None;
        }

        let item = self.filtered_items.get(self.selected_index)?.item.clone();
        let sync_id = Self::parse_sync_id_lossy(&item.action_data());
        let env = CloudAmbientAgentEnvironment::get_by_id(&sync_id, app)?;

        let repo_names = env
            .model()
            .string_model
            .github_repos
            .iter()
            .map(|repo| repo.repo.clone())
            .collect::<Vec<_>>();
        let repos_text = if repo_names.is_empty() {
            "(none)".to_string()
        } else {
            repo_names.join(", ")
        };

        Some(EnvironmentSidecarData {
            name: env.model().string_model.display_name(),
            id: env.id.to_string(),
            image: env.model().string_model.base_image.to_string(),
            repos_text,
        })
    }

    fn environment_sidecar_anchor_id(&self) -> Option<String> {
        if !self.should_show_environment_sidecar() {
            return None;
        }

        Some(format!("MenuPromptChip-{}", self.selected_index))
    }

    fn environment_sidecar_side(
        &self,
        position_id: &str,
        app: &AppContext,
    ) -> EnvironmentSidecarSide {
        let Some(window) = app.windows().platform_window(self.window_id) else {
            return EnvironmentSidecarSide::Left;
        };

        // Anchor is the currently selected/hovered row.
        let Some(anchor_rect) =
            app.element_position_by_id_at_last_frame(self.window_id, position_id)
        else {
            return EnvironmentSidecarSide::Left;
        };

        let gap = ENV_SIDE_CAR_HORIZONTAL_GAP;
        let window_width = window.size().x();

        // If sidecar is on the right of the anchor.
        let right_edge_if_on_right = anchor_rect.max_x() + gap + ENV_SIDE_CAR_WIDTH;
        let overflow_right = (right_edge_if_on_right - window_width).max(0.);

        // If sidecar is on the left of the anchor.
        let left_edge_if_on_left = anchor_rect.min_x() - gap - ENV_SIDE_CAR_WIDTH;
        let overflow_left = (0. - left_edge_if_on_left).max(0.);

        let would_overflow_right = overflow_right > 0.;
        let would_overflow_left = overflow_left > 0.;

        match (would_overflow_left, would_overflow_right) {
            (true, false) => EnvironmentSidecarSide::Right,
            (false, true) => EnvironmentSidecarSide::Left,
            (false, false) => EnvironmentSidecarSide::Left,
            (true, true) => {
                if overflow_left <= overflow_right {
                    EnvironmentSidecarSide::Left
                } else {
                    EnvironmentSidecarSide::Right
                }
            }
        }
    }

    fn environment_sidecar_positioning(
        &self,
        position_id: String,
        app: &AppContext,
    ) -> Option<OffsetPositioning> {
        // Ensure anchor rect exists in cache; otherwise positioning will be wrong.
        app.element_position_by_id_at_last_frame(self.window_id, &position_id)?;

        let side = self.environment_sidecar_side(&position_id, app);
        let offset_y = -ENV_MENU_VERTICAL_PADDING;

        Some(match side {
            EnvironmentSidecarSide::Right => OffsetPositioning::offset_from_save_position_element(
                position_id,
                vec2f(ENV_SIDE_CAR_HORIZONTAL_GAP, offset_y),
                PositionedElementOffsetBounds::WindowByPosition,
                PositionedElementAnchor::TopRight,
                ChildAnchor::TopLeft,
            ),
            EnvironmentSidecarSide::Left => OffsetPositioning::offset_from_save_position_element(
                position_id,
                vec2f(-ENV_SIDE_CAR_HORIZONTAL_GAP, offset_y),
                PositionedElementOffsetBounds::WindowByPosition,
                PositionedElementAnchor::TopLeft,
                ChildAnchor::TopRight,
            ),
        })
    }

    fn environment_sidecar_overlay(
        &self,
        app: &AppContext,
    ) -> Option<(Box<dyn Element>, OffsetPositioning)> {
        let data = self.environment_sidecar_data(app)?;
        let position_id = self.environment_sidecar_anchor_id()?;
        let positioning = self.environment_sidecar_positioning(position_id, app)?;
        Some((self.render_environment_sidecar(&data, app), positioning))
    }

    fn render_environment_sidecar(
        &self,
        data: &EnvironmentSidecarData,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let background = theme.surface_2();
        let label_text_color = theme.sub_text_color(background).into_solid();
        let main_text_color = theme.main_text_color(background).into_solid();

        let label_font_size = 12.;
        let value_font_size = 12.;

        let id_key = format!("env-sidecar:{}:id", data.id);
        let image_key = format!("env-sidecar:{}:image", data.id);

        let icon = |icon: Icon| {
            ConstrainedBox::new(icon.to_warpui_icon(Fill::Solid(label_text_color)).finish())
                .with_width(ENV_SIDE_CAR_ICON_SIZE)
                .with_height(ENV_SIDE_CAR_ICON_SIZE)
                .finish()
        };

        let label_text = |text: &str| {
            Text::new_inline(
                text.to_string(),
                appearance.ui_font_family(),
                label_font_size,
            )
            .with_color(label_text_color)
            .finish()
        };

        let value_text = |text: String| {
            Text::new(text, appearance.ui_font_family(), value_font_size)
                .with_color(main_text_color)
                .with_selectable(true)
                .finish()
        };

        let id_value = {
            let env_id = data.id.clone();
            render_copyable_text_field(
                CopyableTextFieldConfig::new(env_id.clone())
                    .with_font_size(value_font_size)
                    .with_text_color(main_text_color)
                    .with_icon_size(ENV_SIDE_CAR_COPY_BUTTON_SIZE)
                    .with_mouse_state(self.env_sidecar_copy_id_mouse_state.clone())
                    .with_last_copied_at(self.env_sidecar_copy_feedback_times.get(&id_key))
                    .with_wrap_text(true)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_copy_button_placement(CopyButtonPlacement::NextToText),
                move |ctx| {
                    ctx.dispatch_typed_action(DisplayChipMenuAction::CopyEnvironmentSidecarField {
                        key: id_key.clone(),
                        value: env_id.clone(),
                    });
                },
                app,
            )
        };

        let image_value = {
            let docker_image = data.image.clone();
            render_copyable_text_field(
                CopyableTextFieldConfig::new(docker_image.clone())
                    .with_font_size(value_font_size)
                    .with_text_color(main_text_color)
                    .with_icon_size(ENV_SIDE_CAR_COPY_BUTTON_SIZE)
                    .with_mouse_state(self.env_sidecar_copy_image_mouse_state.clone())
                    .with_last_copied_at(self.env_sidecar_copy_feedback_times.get(&image_key))
                    .with_wrap_text(true)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_copy_button_placement(CopyButtonPlacement::NextToText),
                move |ctx| {
                    ctx.dispatch_typed_action(DisplayChipMenuAction::CopyEnvironmentSidecarField {
                        key: image_key.clone(),
                        value: docker_image.clone(),
                    });
                },
                app,
            )
        };

        let row = |row_icon: Icon, label: &str, value: Box<dyn Element>, is_last: bool| {
            let label_cluster = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Container::new(icon(row_icon))
                        .with_margin_right(ENV_SIDE_CAR_ICON_LABEL_GAP)
                        .finish(),
                )
                .with_child(label_text(label))
                .finish();

            let element = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(
                    Container::new(label_cluster)
                        .with_margin_right(ENV_SIDE_CAR_ROW_GAP)
                        .finish(),
                )
                .with_child(Shrinkable::new(1., value).finish())
                .finish();

            if is_last {
                element
            } else {
                Container::new(element)
                    .with_margin_bottom(ENV_SIDE_CAR_ROW_GAP)
                    .finish()
            }
        };

        let content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(row(
                Icon::Globe4,
                "Name:",
                value_text(data.name.clone()),
                false,
            ))
            .with_child(row(Icon::Hash, "ID:", id_value, false))
            .with_child(row(Icon::Docker, "Image:", image_value, false))
            .with_child(row(
                Icon::Github,
                "Repos:",
                value_text(data.repos_text.clone()),
                true,
            ))
            .finish();

        let scrollable_content = ClippedScrollable::vertical(
            self.env_sidecar_scroll_state.clone(),
            content,
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            // Leave the scrollbar gutter background transparent.
            warpui::elements::Fill::None,
        )
        .with_padding_start(0.)
        .with_padding_end(0.)
        .with_overlayed_scrollbar()
        .finish();

        let inner = Container::new(scrollable_content)
            .with_uniform_padding(ENV_SIDE_CAR_PADDING)
            .with_border(
                Border::all(1.).with_border_fill(Fill::Solid(internal_colors::neutral_2(theme))),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                ENV_SIDE_CAR_INNER_RADIUS,
            )))
            .finish();

        let outer = Container::new(inner)
            .with_background(background)
            .with_border(
                Border::all(1.).with_border_fill(Fill::Solid(internal_colors::neutral_4(theme))),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                ENV_SIDE_CAR_OUTER_RADIUS,
            )))
            .with_drop_shadow(Self::figma_menu_drop_shadow())
            .finish();

        ConstrainedBox::new(outer)
            .with_width(ENV_SIDE_CAR_WIDTH)
            .with_min_height(ENV_SIDE_CAR_HEIGHT)
            .finish()
    }

    fn render_fixed_footer_option(
        &self,
        app: &AppContext,
        footer_option: &FixedFooter,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let chip_menu_type = self.chip_menu_type;
        let (font_size, icon_size) = match chip_menu_type {
            ChipMenuType::Environments => (ENV_MENU_ITEM_FONT_SIZE, ENV_MENU_ICON_SIZE),
            ChipMenuType::Directories | ChipMenuType::Branches | ChipMenuType::CodeReview => {
                let font_size = appearance.ui_font_size();
                (font_size, font_size * 0.8)
            }
        };

        let item_horizontal_padding = self.menu_item_horizontal_padding();
        let item_vertical_padding = self.menu_item_vertical_padding();

        let is_footer_selected = self.is_footer_selected();
        ConstrainedBox::new(
            Hoverable::new(footer_option.mouse_state.clone(), move |mouse_state| {
                let is_active = mouse_state.is_hovered() || is_footer_selected;

                let background_color = if is_active {
                    match chip_menu_type {
                        ChipMenuType::Environments => Some(internal_colors::fg_overlay_4(theme)),
                        ChipMenuType::Directories
                        | ChipMenuType::Branches
                        | ChipMenuType::CodeReview => Some(theme.accent()),
                    }
                } else {
                    None
                };

                let text_color = if is_active {
                    match chip_menu_type {
                        ChipMenuType::Environments => {
                            theme.main_text_color(theme.surface_2()).into_solid()
                        }
                        ChipMenuType::Directories
                        | ChipMenuType::Branches
                        | ChipMenuType::CodeReview => {
                            theme.main_text_color(theme.accent()).into_solid()
                        }
                    }
                } else {
                    theme.sub_text_color(theme.surface_2()).into_solid()
                };

                // Update icon and text colors based on hover state
                let mut updated_text =
                    Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

                // Add icon if it exists
                if let Some(icon) = footer_option.action_item.icon(app) {
                    updated_text.add_child(
                        Container::new(
                            ConstrainedBox::new(
                                icon.to_warpui_icon(Fill::Solid(text_color)).finish(),
                            )
                            .with_height(icon_size)
                            .with_width(icon_size)
                            .finish(),
                        )
                        .with_margin_right(8.)
                        .finish(),
                    );
                } else {
                    // Add spacing equivalent to icon + margin for alignment
                    updated_text.add_child(
                        ConstrainedBox::new(Empty::new().finish())
                            .with_width(icon_size + 8.)
                            .finish(),
                    );
                }

                // Add the text element
                updated_text.add_child(
                    Text::new_inline(
                        footer_option.action_item.name(),
                        appearance.ui_font_family(),
                        font_size,
                    )
                    .autosize_text(MIN_FONT_SIZE)
                    .with_color(text_color)
                    .finish(),
                );

                let mut container = Container::new(updated_text.finish())
                    .with_horizontal_padding(item_horizontal_padding)
                    .with_vertical_padding(item_vertical_padding)
                    .with_border(Border::top(1.0));

                if let Some(bg_color) = background_color {
                    container = container.with_background(bg_color);
                }

                container.finish()
            })
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(DisplayChipMenuAction::SelectFixedFooterOption);
            })
            .finish(),
        )
        .with_width(self.menu_width())
        .finish()
    }

    fn render_env_search_footer(
        &self,
        search_input: &ViewHandle<EditorView>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = Appearance::as_ref(app).theme();
        let divider_color = internal_colors::fg_overlay_2(theme);

        Container::new(ChildView::new(search_input).finish())
            .with_margin_top(ENV_MENU_SEARCH_FOOTER_TOP_MARGIN)
            .with_horizontal_padding(ENV_MENU_ITEM_HORIZONTAL_PADDING)
            .with_padding_top(ENV_MENU_SEARCH_VERTICAL_PADDING)
            .with_padding_bottom(ENV_MENU_SEARCH_BOTTOM_PADDING)
            .with_border(Border::top(1.).with_border_fill(divider_color))
            .finish()
    }

    fn render_items(&self, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme();
        if self.filtered_items.is_empty() {
            // Show "No results" if search is active but no matches.
            if !self.search_query.is_empty() {
                // Match the model selector dropdown's no-results row exactly
                // for the environments menu (label, font size, paddings, and
                // text color); other chip menus keep their existing styling.
                let (label, font_size, horizontal_padding, vertical_padding, text_color) =
                    match self.chip_menu_type {
                        ChipMenuType::Environments => (
                            "No results",
                            ENV_MENU_ITEM_FONT_SIZE,
                            ENV_MENU_ITEM_HORIZONTAL_PADDING,
                            ENV_MENU_ITEM_VERTICAL_PADDING,
                            internal_colors::text_sub(theme, theme.surface_2()),
                        ),
                        ChipMenuType::Directories
                        | ChipMenuType::Branches
                        | ChipMenuType::CodeReview => (
                            "No results found",
                            appearance.ui_font_size(),
                            LABEL_HORIZONTAL_PADDING,
                            LABEL_VERTICAL_PADDING * 2.0,
                            theme.sub_text_color(theme.surface_2()).into_solid(),
                        ),
                    };
                return Container::new(
                    Text::new(label, appearance.ui_font_family(), font_size)
                        .with_color(text_color)
                        .finish(),
                )
                .with_horizontal_padding(horizontal_padding)
                .with_vertical_padding(vertical_padding)
                .finish();
            }
            return Empty::new().finish();
        }

        let selected_index = self.selected_index;
        let filtered_items_length = self.filtered_items.len();
        let filtered_items = self.filtered_items.clone();
        let is_footer_hovered = self.is_footer_selected();
        let menu_width = self.menu_width();
        let item_horizontal_padding = self.menu_item_horizontal_padding();
        let item_vertical_padding = self.menu_item_vertical_padding();
        let chip_menu_type = self.chip_menu_type;
        let list = UniformList::new(
            self.list_state.clone(),
            filtered_items.len(),
            move |mut range, app| {
                let appearance = Appearance::as_ref(app);
                let theme = appearance.theme();

                range.end = cmp::min(range.end, filtered_items.len());
                range
                    .map(|index| {
                        let filtered_item = &filtered_items[index];
                        let item = &filtered_item.item;
                        let display_text_str = item.name();

                        let is_selected = index == selected_index && !is_footer_hovered;

                        let font_size = if matches!(chip_menu_type, ChipMenuType::Environments) {
                            ENV_MENU_ITEM_FONT_SIZE
                        } else {
                            appearance.ui_font_size()
                        };
                        let icon_size = font_size * 0.8; // Icon slightly smaller than text

                        let (main_text, selected_background) = match chip_menu_type {
                            ChipMenuType::Environments => (
                                theme.main_text_color(theme.surface_2()).into_solid(),
                                is_selected.then_some(internal_colors::fg_overlay_4(theme)),
                            ),
                            ChipMenuType::Directories
                            | ChipMenuType::Branches
                            | ChipMenuType::CodeReview => {
                                if is_selected {
                                    let bg = theme.accent();
                                    (theme.main_text_color(bg).into_solid(), Some(bg))
                                } else {
                                    (theme.main_text_color(theme.surface_2()).into_solid(), None)
                                }
                            }
                        };

                        // Create main container with SpaceBetween to float right elements to far right
                        let mut main_container = Flex::row()
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_main_axis_size(MainAxisSize::Max);

                        // Create left side container with icon and main text
                        let mut left_side =
                            Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

                        let icon_gap = 8.;
                        if matches!(chip_menu_type, ChipMenuType::Environments) {
                            if let Some(icon) = item.icon(app) {
                                let icon_slot_size = ENV_MENU_ICON_SLOT_SIZE;
                                let glyph_size = ENV_MENU_ICON_SIZE;

                                let icon_glyph = ConstrainedBox::new(
                                    icon.to_warpui_icon(Fill::Solid(main_text)).finish(),
                                )
                                .with_width(glyph_size)
                                .with_height(glyph_size)
                                .finish();

                                let icon_slot = ConstrainedBox::new(
                                    Flex::row()
                                        .with_main_axis_alignment(MainAxisAlignment::Center)
                                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                        .with_child(icon_glyph)
                                        .finish(),
                                )
                                .with_width(icon_slot_size)
                                .with_height(icon_slot_size)
                                .finish();

                                left_side.add_child(
                                    Container::new(icon_slot)
                                        .with_margin_right(icon_gap)
                                        .finish(),
                                );
                            }
                        } else if let Some(icon) = item.icon(app) {
                            left_side.add_child(
                                Container::new(
                                    ConstrainedBox::new(
                                        icon.to_warpui_icon(Fill::Solid(main_text)).finish(),
                                    )
                                    .with_height(icon_size)
                                    .with_width(icon_size)
                                    .finish(),
                                )
                                .with_margin_right(icon_gap)
                                .finish(),
                            );
                        } else {
                            // Add spacing equivalent to icon + margin for alignment
                            left_side.add_child(
                                ConstrainedBox::new(Empty::new().finish())
                                    .with_width(icon_size + icon_gap)
                                    .finish(),
                            );
                        }

                        // Create main text with highlighting if there's a match result
                        let display_text = if let Some(match_result) = &filtered_item.match_result {
                            Text::new_inline(
                                display_text_str,
                                appearance.ui_font_family(),
                                font_size,
                            )
                            .autosize_text(MIN_FONT_SIZE)
                            .with_color(main_text)
                            .with_single_highlight(
                                Highlight::new()
                                    .with_properties(Properties::default().weight(Weight::Bold))
                                    .with_foreground_color(main_text),
                                match_result.matched_indices.clone(),
                            )
                        } else {
                            Text::new_inline(
                                display_text_str,
                                appearance.ui_font_family(),
                                font_size,
                            )
                            .autosize_text(MIN_FONT_SIZE)
                            .with_color(main_text)
                        };

                        left_side.add_child(display_text.finish());

                        // Add left side to main container
                        main_container.add_child(left_side.finish());

                        // Add right-side element if available, using SpaceBetween alignment
                        if let Some(right_element) = item.right_side_element(app) {
                            main_container = main_container
                                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween);
                            main_container.add_child(right_element);
                        }

                        let mut container = Container::new(main_container.finish())
                            .with_horizontal_padding(item_horizontal_padding)
                            .with_vertical_padding(item_vertical_padding);

                        if !matches!(chip_menu_type, ChipMenuType::Environments)
                            && (is_selected || index < filtered_items_length - 1)
                        {
                            container = container.with_border(Border::bottom(1.0));
                        }

                        if let Some(bg) = selected_background {
                            container = container.with_background(bg);
                        }

                        SavePosition::new(
                            EventHandler::new(container.finish())
                                .on_left_mouse_down(move |ctx, _, _| {
                                    ctx.dispatch_typed_action(DisplayChipMenuAction::SelectItem {
                                        index,
                                    });
                                    DispatchEventResult::StopPropagation
                                })
                                .on_mouse_in(
                                    move |ctx, _, _| {
                                        ctx.dispatch_typed_action(DisplayChipMenuAction::Select {
                                            index,
                                        });
                                        ctx.notify();
                                        DispatchEventResult::StopPropagation
                                    },
                                    Some(MouseInBehavior {
                                        fire_on_synthetic_events: false,
                                        fire_when_covered: false,
                                    }),
                                )
                                .finish(),
                            format!("MenuPromptChip-{index}").as_str(),
                        )
                        .finish()
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
            },
        );

        let (scrollbar_width, max_height, overlayed_scrollbar) = match self.chip_menu_type {
            ChipMenuType::Environments => (
                ScrollbarWidth::Auto,
                ENV_MENU_MAX_HEIGHT - (ENV_MENU_VERTICAL_PADDING * 2.0),
                true,
            ),
            ChipMenuType::Directories | ChipMenuType::Branches | ChipMenuType::CodeReview => {
                (ScrollbarWidth::None, 200., false)
            }
        };

        let mut scrollable = Scrollable::vertical(
            self.scroll_state.clone(),
            list.finish_scrollable(),
            scrollbar_width,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .with_padding_end(0.)
        .with_padding_start(0.);

        if overlayed_scrollbar {
            scrollable = scrollable.with_overlayed_scrollbar();
        }

        // Return just the scrollable content area (no outer styling)
        ConstrainedBox::new(scrollable.finish())
            .with_width(menu_width)
            .with_max_height(max_height)
            .finish()
    }
}

impl View for DisplayChipMenu {
    fn ui_name() -> &'static str {
        "DisplayMenu"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            if let Some(ref search_input) = self.search_input {
                ctx.focus(search_input);
            }
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // Create vertical flex container for main content + search input + sticky fixed footer option
        let mut main_container = Flex::column();

        let border_radius = Radius::Pixels(6.);

        match self.chip_menu_type {
            ChipMenuType::Environments => {
                if !self.menu_items.is_empty() {
                    main_container.add_child(
                        Container::new(self.render_items(app))
                            .with_padding_top(self.menu_vertical_padding())
                            .with_padding_bottom(self.menu_vertical_padding())
                            .finish(),
                    );
                }
                if let Some(ref footer_option) = self.fixed_footer {
                    main_container.add_child(self.render_fixed_footer_option(app, footer_option));
                }
                if let Some(ref search_input_handle) = self.search_input {
                    main_container
                        .add_child(self.render_env_search_footer(search_input_handle, app));
                }
            }
            ChipMenuType::Directories | ChipMenuType::Branches | ChipMenuType::CodeReview => {
                if let Some(ref search_input_handle) = self.search_input {
                    let search_input = appearance
                        .ui_builder()
                        .text_input(search_input_handle.clone())
                        .with_style(UiComponentStyles {
                            background: Some(Fill::Solid(ColorU::new(0, 0, 0, 0)).into()),
                            border_color: None,
                            border_width: Some(0.),
                            border_radius: None,
                            width: Some(self.menu_width() - (SEARCH_INPUT_HORIZONTAL_PADDING * 2.)),
                            padding: Some(Coords::uniform(4.)),
                            ..Default::default()
                        })
                        .build()
                        .finish();

                    let search_input_container = Container::new(search_input)
                        .with_horizontal_padding(SEARCH_INPUT_HORIZONTAL_PADDING)
                        .with_vertical_padding(2.)
                        .with_background(theme.surface_1())
                        .with_border(Border::all(1.0).with_border_color(theme.surface_2().into()))
                        .with_corner_radius(CornerRadius::with_top(border_radius))
                        .finish();

                    main_container.add_child(search_input_container);
                }
                if let Some(ref footer_option) = self.fixed_footer {
                    main_container.add_child(self.render_fixed_footer_option(app, footer_option));
                }
                if !self.menu_items.is_empty() {
                    main_container.add_child(
                        Container::new(self.render_items(app))
                            .with_padding_bottom(self.menu_vertical_padding())
                            .finish(),
                    );
                }
            }
        }

        let menu_card = {
            let menu_container = Container::new(main_container.finish())
                .with_background(theme.surface_2())
                .with_corner_radius(CornerRadius::with_all(border_radius));

            let menu_container = match self.chip_menu_type {
                ChipMenuType::Environments => menu_container
                    .with_border(
                        Border::all(1.)
                            .with_border_fill(Fill::Solid(internal_colors::neutral_4(theme))),
                    )
                    .with_drop_shadow(Self::figma_menu_drop_shadow()),
                ChipMenuType::Directories | ChipMenuType::Branches | ChipMenuType::CodeReview => {
                    menu_container.with_drop_shadow(DropShadow::default())
                }
            };

            ConstrainedBox::new(menu_container.finish())
                .with_width(self.menu_width())
                .finish()
        };

        let mut stack = Stack::new();
        stack.add_child(menu_card);

        if self.should_show_environment_sidecar() {
            if let Some((sidecar, positioning)) = self.environment_sidecar_overlay(app) {
                stack.add_positioned_overlay_child(sidecar, positioning);
            }
        }

        Dismiss::new(stack.finish())
            .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(DisplayChipMenuAction::Close))
            .prevent_interaction_with_other_elements()
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct GenericMenuEvent {
    pub action_item: Arc<dyn GenericMenuItem>,
}

pub enum PromptDisplayMenuEvent {
    MenuAction(GenericMenuEvent),
    CloseMenu,
}

impl Entity for DisplayChipMenu {
    type Event = PromptDisplayMenuEvent;
}

impl TypedActionView for DisplayChipMenu {
    type Action = DisplayChipMenuAction;

    fn handle_action(&mut self, action: &DisplayChipMenuAction, ctx: &mut ViewContext<Self>) {
        match action {
            DisplayChipMenuAction::SelectItem { index } => {
                if *index >= self.filtered_items.len() {
                    return;
                }
                let item = self.filtered_items[*index].item.clone();
                self.select_item(item, ctx)
            }
            DisplayChipMenuAction::Select { index } => self.select(*index, ctx),
            DisplayChipMenuAction::SelectUp => self.select_prev(ctx),
            DisplayChipMenuAction::SelectDown => self.select_next(ctx),
            DisplayChipMenuAction::SelectEnter => self.select_enter(ctx),
            DisplayChipMenuAction::SelectFixedFooterOption => self.select_fixed_footer_option(ctx),
            DisplayChipMenuAction::CopyEnvironmentSidecarField { key, value } => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(value.clone()));

                self.env_sidecar_copy_feedback_times
                    .insert(key.clone(), Instant::now());

                let duration = COPY_FEEDBACK_DURATION;
                ctx.spawn(
                    async move {
                        Timer::after(duration).await;
                    },
                    move |me, _, ctx| {
                        // Clean up old entries.
                        me.env_sidecar_copy_feedback_times
                            .retain(|_, time| time.elapsed() < duration);
                        ctx.notify();
                    },
                );

                ctx.notify();
            }
            DisplayChipMenuAction::Close => self.close(ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::query_matches_existing_name;

    #[test]
    fn query_matches_existing_name_is_ascii_case_insensitive() {
        let names = ["main", "feature/Foo"];
        assert!(query_matches_existing_name(names, "main"));
        assert!(query_matches_existing_name(names, "Main"));
        assert!(query_matches_existing_name(names, "MAIN"));
        assert!(query_matches_existing_name(names, "feature/foo"));
        assert!(query_matches_existing_name(names, "FEATURE/FOO"));
    }

    #[test]
    fn query_matches_existing_name_returns_false_when_no_overlap() {
        let names = ["main", "feature/foo"];
        assert!(!query_matches_existing_name(names, "develop"));
        assert!(!query_matches_existing_name(names, "feature/bar"));
    }

    #[test]
    fn query_matches_existing_name_returns_false_for_empty_input() {
        let names: [&str; 0] = [];
        assert!(!query_matches_existing_name(names, "main"));
    }

    #[test]
    fn query_matches_existing_name_works_with_owned_strings() {
        let names = [String::from("main"), String::from("Develop")];
        assert!(query_matches_existing_name(names.iter(), "Main"));
        assert!(query_matches_existing_name(names.iter(), "develop"));
    }
}
