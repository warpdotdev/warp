use crate::ai::agent::conversation::ConversationStatus;
use crate::ai::conversation_status_ui::{render_status_element, STATUS_ELEMENT_PADDING};
use crate::appearance::Appearance;
/// Tab module contains structures related to Tabs (such as TabData or TabComponent) that simplify
/// the rendering and management of tabs in general.
use crate::editor::EditorView;
use crate::features::FeatureFlag;
use crate::launch_configs::launch_config::LaunchConfig;
use crate::menu::{MenuAction, MenuItem, MenuItemFields};
use crate::pane_group::{PaneGroup, PaneId};
use settings::Setting as _;
use std::sync::Arc;
use std::time::Duration;

use crate::shell_indicator::ShellIndicatorType;
use crate::terminal::shared_session::render_util::shared_session_indicator_color;
use crate::terminal::view::TerminalViewState;
use crate::themes::theme::{AnsiColorIdentifier, Fill as ThemeFill, VerticalGradient};
use crate::ui_components::buttons::icon_button;
use crate::ui_components::color_dot::{render_color_dot, TAB_COLOR_OPTIONS};
use crate::ui_components::icons::{Icon, ICON_DIMENSIONS};
use crate::util::color::{coloru_with_opacity, Opacity};
use crate::util::truncation::truncate_from_end;

use crate::window_settings::WindowSettings;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspace::tab_settings::{
    TabCloseButtonPosition, TabSettings, VerticalTabsDisplayGranularity,
};
use crate::workspace::{
    PaneViewLocator, TabBarDropTargetData, TabBarLocation, TabContextMenuAnchor, WorkspaceAction,
};
use crate::BlocklistAIHistoryModel;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use serde::{Deserialize, Serialize};
use warp_core::context_flag::ContextFlag;
use warp_core::ui::builder::UiBuilder;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::AnsiColors;
use warpui::elements::{
    Align, Border, ChildAnchor, Clipped, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, DragAxis, Draggable, DraggableState, DropTarget, Element, Empty, Fill,
    Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, Padding,
    ParentAnchor, ParentElement, ParentOffsetBounds, PositionedElementAnchor,
    PositionedElementOffsetBounds, Radius, Rect, SavePosition, Shrinkable, SizeConstraintCondition,
    SizeConstraintSwitch, Stack, Text,
};
use warpui::fonts::Weight;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::text_input::TextInput;
use warpui::{AppContext, SingletonEntity, ViewHandle};

pub const TAB_BAR_BORDER_HEIGHT: f32 = 1.0;
const TAB_INDICATOR_HEIGHT: f32 = 14.0;

/// True when the user has opted into vertical tabs and the feature flag is on.
/// Exposed so binding-description overrides in `workspace/mod.rs` and context-
/// menu builders here can share a single predicate.
pub fn uses_vertical_tabs(ctx: &AppContext) -> bool {
    FeatureFlag::VerticalTabs.is_enabled() && *TabSettings::as_ref(ctx).use_vertical_tabs
}

const WARP_2_TAB_COLOR_OPACITY: Opacity = 25;
const WARP_2_HOVERED_TAB_COLOR_OPACITY: Opacity = 50;
const TAB_CLOSE_BUTTON_OPACITY: Opacity = 60;
const TAB_CLOSE_BUTTON_WIDTH: f32 = 20.0;
const MAX_TOOLTIP_LENGTH: usize = 80;

const TAB_INDICATOR_SYNCED_COLOR: u32 = 0x4A93FFFF;

// Width threshold (in px) below which we render an icon-only tab
const COMPACT_TAB_WIDTH_THRESHOLD: f32 = 42.0;
// Horizontal inset for the tab close button
const TAB_CLOSE_BUTTON_HORIZONTAL_INSET: f32 = 2.0;

/// Represents the user's manual tab-color selection state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectedTabColor {
    /// No manual override — fall back to the default directory color.
    #[default]
    Unset,
    /// User explicitly cleared the color (overrides any default).
    Cleared,
    /// User explicitly chose this color.
    Color(AnsiColorIdentifier),
}

impl SelectedTabColor {
    /// Resolves the effective tab color: manual selection takes priority,
    /// falling back to `default` when no override is set.
    pub(crate) fn resolve(
        self,
        default: Option<AnsiColorIdentifier>,
    ) -> Option<AnsiColorIdentifier> {
        match self {
            SelectedTabColor::Color(c) => Some(c),
            SelectedTabColor::Cleared => None,
            SelectedTabColor::Unset => default,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[allow(clippy::enum_variant_names)]
pub enum TabTelemetryAction {
    CloseTab,
    CloseOtherTabs,
    CloseTabsToRight,
    SetColor,
    ResetColor,
}
#[derive(Debug, Clone)]
pub enum NewSessionMenuItem {
    OpenLaunchConfig(LaunchConfig),
    OpenLaunchConfigDocs,
    CreateNewTabConfig,
}

#[derive(Clone, Copy)]
pub struct PaneNameMenuTarget {
    pub locator: PaneViewLocator,
    pub rename_label: &'static str,
    pub reset_label: &'static str,
}

/// TabData struct holds the state of the given tab. It includes the pane group and mouse states
/// used for closing the tab or tracking the mouse over the rendered tab.
/// It has to be "stored" by some View to persist its state.
///
/// TODO(vorporeal): Probably want to split this into TabData and "RenderableTabData",
/// where the latter is more of a view model and holds the state handles.
#[derive(Clone)]
pub struct TabData {
    pub pane_group: ViewHandle<PaneGroup>,
    pub tab_mouse_state: MouseStateHandle,
    pub close_mouse_state: MouseStateHandle,
    pub tooltip_mouse_state: MouseStateHandle,
    pub draggable_state: DraggableState,
    /// Color derived from the directory→color mapping (set automatically).
    pub default_directory_color: Option<AnsiColorIdentifier>,
    /// Color chosen manually by the user (e.g. right-click menu).
    pub selected_color: SelectedTabColor,
    pub indicator_hover_state: MouseStateHandle,
    // Used by a later drag-tab branch to distinguish tabs that have moved into detached windows.
    pub detached: bool,
}

const TAB_COLOR_ICON_PATH: &str = "bundled/svg/ellipse.svg";
const TAB_NO_COLOR_ICON_PATH: &str = "bundled/svg/no_color_ellipse.svg";

impl TabData {
    pub fn new(pane_group: ViewHandle<PaneGroup>) -> Self {
        Self {
            pane_group,
            tab_mouse_state: Default::default(),
            close_mouse_state: Default::default(),
            tooltip_mouse_state: Default::default(),
            draggable_state: Default::default(),
            default_directory_color: None,
            selected_color: SelectedTabColor::Unset,
            indicator_hover_state: Default::default(),
            detached: false,
        }
    }

    /// The resolved tab color: manual selection takes priority over directory default.
    pub fn color(&self) -> Option<AnsiColorIdentifier> {
        self.selected_color.resolve(self.default_directory_color)
    }

    /// Returns the menu items for the context menu on right mouse click.
    pub fn menu_items(
        &self,
        index: usize,
        tabs_len: usize,
        ctx: &AppContext,
    ) -> Vec<MenuItem<WorkspaceAction>> {
        self.menu_items_with_pane_name_target(index, tabs_len, None, ctx)
    }

    pub fn menu_items_with_pane_name_target(
        &self,
        index: usize,
        tabs_len: usize,
        pane_name_target: Option<PaneNameMenuTarget>,
        ctx: &AppContext,
    ) -> Vec<MenuItem<WorkspaceAction>> {
        let appearance = Appearance::as_ref(ctx);
        let terminal_colors = appearance.theme().terminal_colors().normal;
        let mut menu_items = vec![];

        for section_items in [
            self.session_sharing_menu_items(index, ctx),
            self.copy_metadata_menu_items(pane_name_target, ctx),
            self.modify_tab_menu_items(index, tabs_len, pane_name_target, ctx),
            self.close_tab_menu_items(index, tabs_len, ctx),
            Self::save_config_menu_items(index),
            self.color_option_menu_items(index, terminal_colors),
        ] {
            if menu_items
                .last()
                .is_some_and(|item| !matches!(item, MenuItem::Separator))
            {
                menu_items.push(MenuItem::Separator);
            }
            menu_items.extend(section_items);
        }
        menu_items
    }

    fn session_sharing_menu_items(
        &self,
        index: usize,
        ctx: &AppContext,
    ) -> Vec<MenuItem<WorkspaceAction>> {
        let mut menu_items = vec![];

        if FeatureFlag::CreatingSharedSessions.is_enabled()
            && ContextFlag::CreateSharedSession.is_enabled()
        {
            let shared_session_view_ids = self.pane_group.as_ref(ctx).shared_session_view_ids(ctx);
            let focused_session_view = self.pane_group.as_ref(ctx).focused_session_view(ctx);

            // If the focused pane is one of the shared sessions, add an option to stop it specifically,
            // otherwise add an option to share it.
            if let Some(focused_session_view) = focused_session_view {
                if focused_session_view
                    .as_ref(ctx)
                    .model
                    .lock()
                    .shared_session_status()
                    .is_active_sharer()
                {
                    menu_items.push(
                        MenuItemFields::new("Stop sharing")
                            .with_on_select_action(WorkspaceAction::StopSharingSessionFromTabMenu {
                                terminal_view_id: focused_session_view.id(),
                            })
                            .into_item(),
                    );
                } else {
                    menu_items.push(
                        MenuItemFields::new("Share session")
                            .with_on_select_action(WorkspaceAction::OpenShareSessionModal(index))
                            .into_item(),
                    );
                }
            }

            // Always show an option to stop sharing all when there's at least 1 shared session in the tab.
            if !shared_session_view_ids.is_empty() {
                menu_items.push(
                    MenuItemFields::new("Stop sharing all")
                        .with_on_select_action(WorkspaceAction::StopSharingAllSessionsInTab {
                            pane_group: self.pane_group.downgrade(),
                        })
                        .into_item(),
                );
            }
        }

        // Add "Copy link" option if the focused session in this tab is being shared or viewed
        let is_shared_or_viewed = self
            .pane_group
            .as_ref(ctx)
            .focused_session_view(ctx)
            .map(|view| {
                view.as_ref(ctx)
                    .model
                    .lock()
                    .shared_session_status()
                    .is_sharer_or_viewer()
            })
            .unwrap_or(false);

        if is_shared_or_viewed {
            menu_items.push(
                MenuItemFields::new("Copy link")
                    .with_on_select_action(WorkspaceAction::CopySharedSessionLinkFromTab {
                        tab_index: index,
                    })
                    .into_item(),
            );
        }

        menu_items
    }

    fn copyable_pane_title(
        pane_group: &PaneGroup,
        pane_id: PaneId,
        ctx: &AppContext,
    ) -> Option<String> {
        pane_group.pane_by_id(pane_id).and_then(|pane| {
            let configuration = pane.pane_configuration();
            let configuration = configuration.as_ref(ctx);
            let title = configuration
                .custom_vertical_tabs_title()
                .unwrap_or_else(|| configuration.title());
            Self::copyable_metadata_value(Some(title.to_string()))
        })
    }

    fn copy_metadata_menu_items(
        &self,
        pane_name_target: Option<PaneNameMenuTarget>,
        ctx: &AppContext,
    ) -> Vec<MenuItem<WorkspaceAction>> {
        let pane_group = self.pane_group.as_ref(ctx);
        let mut menu_items = vec![];
        let tab_title = Self::copyable_metadata_value(Some(pane_group.display_title(ctx)));
        if !uses_vertical_tabs(ctx) {
            Self::push_copy_metadata_menu_item(&mut menu_items, "Copy tab title", tab_title);
            return menu_items;
        }

        let vertical_tabs_display_granularity = *TabSettings::as_ref(ctx)
            .vertical_tabs_display_granularity
            .value();
        let (title_label, title, terminal_view) = if matches!(
            vertical_tabs_display_granularity,
            VerticalTabsDisplayGranularity::Panes
        ) {
            let pane_id = pane_name_target
                .filter(|target| self.pane_group.id() == target.locator.pane_group_id)
                .and_then(|target| {
                    pane_group
                        .pane_by_id(target.locator.pane_id)
                        .map(|_| target.locator.pane_id)
                })
                .unwrap_or_else(|| pane_group.focused_pane_id(ctx));
            (
                "Copy pane title",
                Self::copyable_pane_title(pane_group, pane_id, ctx),
                pane_group.terminal_view_from_pane_id(pane_id, ctx),
            )
        } else {
            let terminal_view = pane_name_target
                .filter(|target| self.pane_group.id() == target.locator.pane_group_id)
                .and_then(|target| {
                    pane_group.terminal_view_from_pane_id(target.locator.pane_id, ctx)
                })
                .or_else(|| pane_group.focused_session_view(ctx));
            ("Copy tab title", tab_title, terminal_view)
        };

        if let Some(terminal_view) = terminal_view {
            let terminal_view = terminal_view.as_ref(ctx);
            Self::push_copy_metadata_menu_item(
                &mut menu_items,
                "Copy branch",
                Self::copyable_metadata_value(terminal_view.current_git_branch(ctx)),
            );
            Self::push_copy_metadata_menu_item(&mut menu_items, title_label, title);
            Self::push_copy_metadata_menu_item(
                &mut menu_items,
                "Copy working directory",
                Self::copyable_metadata_value(
                    terminal_view
                        .pwd()
                        .or_else(|| terminal_view.display_working_directory(ctx)),
                ),
            );
            Self::push_copy_metadata_menu_item(
                &mut menu_items,
                "Copy pull request link",
                Self::copyable_metadata_value(terminal_view.current_pull_request_url(ctx)),
            );
        } else {
            Self::push_copy_metadata_menu_item(&mut menu_items, title_label, title);
        }

        menu_items
    }

    fn push_copy_metadata_menu_item(
        menu_items: &mut Vec<MenuItem<WorkspaceAction>>,
        label: &'static str,
        value: Option<String>,
    ) {
        if let Some(value) = value {
            menu_items.push(
                MenuItemFields::new(label)
                    .with_on_select_action(WorkspaceAction::CopyTextToClipboard(value))
                    .into_item(),
            );
        }
    }

    fn copyable_metadata_value(value: Option<String>) -> Option<String> {
        value.filter(|value| !value.trim().is_empty())
    }

    fn modify_tab_menu_items(
        &self,
        index: usize,
        tabs_len: usize,
        pane_name_target: Option<PaneNameMenuTarget>,
        ctx: &AppContext,
    ) -> Vec<MenuItem<WorkspaceAction>> {
        let mut menu_items = vec![];
        let uses_vertical_tabs = uses_vertical_tabs(ctx);

        // TODO add option to show the keybinding once we figure out a nice API to retrieve
        // the actual keybinding (based on the user's preferences etc.)
        menu_items.append(&mut vec![MenuItemFields::new("Rename tab")
            .with_on_select_action(WorkspaceAction::RenameTab(index))
            .into_item()]);
        // Group together with rename option (note, resetting doesn't make
        // sense unless you're able to rename a tab).
        let title = self.pane_group.as_ref(ctx).custom_title(ctx);
        if title.is_some() {
            menu_items.push(
                MenuItemFields::new("Reset tab name")
                    .with_on_select_action(WorkspaceAction::ResetTabName(index))
                    .into_item(),
            );
        }
        if let Some(pane_name_target) = pane_name_target {
            menu_items.extend(self.pane_name_menu_items(pane_name_target, ctx));
        }
        // Don't show options that aren't relevant (moving end tabs, closing
        // other tabs when you don't have any others to close)
        let not_last_tab = index != tabs_len - 1;
        if not_last_tab {
            menu_items.push(
                MenuItemFields::new(if uses_vertical_tabs {
                    "Move Tab Down"
                } else {
                    "Move Tab Right"
                })
                .with_on_select_action(WorkspaceAction::MoveTabRight(index))
                .into_item(),
            );
        }
        if index != 0 {
            menu_items.push(
                MenuItemFields::new(if uses_vertical_tabs {
                    "Move Tab Up"
                } else {
                    "Move Tab Left"
                })
                .with_on_select_action(WorkspaceAction::MoveTabLeft(index))
                .into_item(),
            );
        }
        menu_items
    }

    fn pane_name_menu_items(
        &self,
        target: PaneNameMenuTarget,
        ctx: &AppContext,
    ) -> Vec<MenuItem<WorkspaceAction>> {
        let pane_group = self.pane_group.as_ref(ctx);
        if self.pane_group.id() != target.locator.pane_group_id {
            return vec![];
        }
        let Some(pane) = pane_group.pane_by_id(target.locator.pane_id) else {
            return vec![];
        };
        let configuration = pane.pane_configuration();
        let has_custom_name = configuration
            .as_ref(ctx)
            .custom_vertical_tabs_title()
            .is_some();

        let mut menu_items = vec![MenuItemFields::new(target.rename_label)
            .with_on_select_action(WorkspaceAction::RenamePane(target.locator))
            .into_item()];
        if has_custom_name {
            menu_items.push(
                MenuItemFields::new(target.reset_label)
                    .with_on_select_action(WorkspaceAction::ResetPaneName(target.locator))
                    .into_item(),
            );
        }
        menu_items
    }

    fn close_tab_menu_items(
        &self,
        index: usize,
        tabs_len: usize,
        ctx: &AppContext,
    ) -> Vec<MenuItem<WorkspaceAction>> {
        let mut menu_items = vec![];
        let uses_vertical_tabs = uses_vertical_tabs(ctx);

        if ContextFlag::CloseWindow.is_enabled() || tabs_len != 1 {
            menu_items.push(
                MenuItemFields::new("Close tab")
                    .with_on_select_action(WorkspaceAction::CloseTab(index))
                    .into_item(),
            );
        }
        if tabs_len > 1 {
            menu_items.push(
                MenuItemFields::new("Close other tabs")
                    .with_on_select_action(WorkspaceAction::CloseOtherTabs(index))
                    .into_item(),
            );
        }
        let not_last_tab = index != tabs_len - 1;
        if not_last_tab {
            menu_items.push(
                MenuItemFields::new(if uses_vertical_tabs {
                    "Close Tabs Below"
                } else {
                    "Close Tabs to the Right"
                })
                .with_on_select_action(WorkspaceAction::CloseTabsRight(index))
                .into_item(),
            );
        }
        menu_items
    }

    fn save_config_menu_items(index: usize) -> Vec<MenuItem<WorkspaceAction>> {
        if !FeatureFlag::TabConfigs.is_enabled() {
            return vec![];
        }
        vec![MenuItemFields::new("Save as new config")
            .with_on_select_action(WorkspaceAction::SaveCurrentTabAsNewConfig(index))
            .into_item()]
    }

    fn color_option_menu_items(
        &self,
        index: usize,
        terminal_colors: AnsiColors,
    ) -> Vec<MenuItem<WorkspaceAction>> {
        if FeatureFlag::DirectoryTabColors.is_enabled() {
            self.dot_color_option_menu_items(index, terminal_colors)
        } else {
            self.legacy_color_option_menu_items(index, terminal_colors)
        }
    }

    /// New dot-based color picker: default (no-color) + color options.
    /// Rendered as a single custom menu item with individually clickable dots.
    fn dot_color_option_menu_items(
        &self,
        index: usize,
        terminal_colors: AnsiColors,
    ) -> Vec<MenuItem<WorkspaceAction>> {
        let effective_color = self.color();
        let mouse_states: Vec<MouseStateHandle> = (0..TAB_COLOR_OPTIONS.len() + 1)
            .map(|_| MouseStateHandle::default())
            .collect();

        vec![MenuItem::Item(
            MenuItemFields::new_with_custom_label(
                Arc::new(move |_is_selected, _is_hovered, appearance, _app| {
                    let theme = appearance.theme();
                    let ring_color: ColorU = theme.accent().into();

                    let mut row = Flex::row()
                        .with_main_axis_alignment(MainAxisAlignment::SpaceEvenly)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_main_axis_size(MainAxisSize::Max);

                    for (ansi_id, mouse_state) in std::iter::once(None)
                        .chain(TAB_COLOR_OPTIONS.iter().copied().map(Some))
                        .zip(mouse_states.iter().cloned())
                    {
                        let is_selected = match ansi_id {
                            None => effective_color.is_none(),
                            Some(id) => effective_color == Some(id),
                        };
                        let dot_color: ColorU = match ansi_id {
                            None => ColorU::transparent_black(),
                            Some(id) => id.to_ansi_color(&terminal_colors).into(),
                        };
                        let tooltip = match ansi_id {
                            None => "Default (no color)".to_string(),
                            Some(id) => id.to_string(),
                        };

                        let dot = render_color_dot(
                            mouse_state,
                            dot_color,
                            is_selected,
                            ring_color,
                            ansi_id.is_none(),
                            theme.foreground(),
                            tooltip,
                            appearance,
                        )
                        .on_click(move |ctx, _, _| {
                            if let Some(color) = ansi_id {
                                ctx.dispatch_typed_action(WorkspaceAction::ToggleTabColor {
                                    color,
                                    tab_index: index,
                                });
                            } else if let Some(color) = effective_color {
                                ctx.dispatch_typed_action(WorkspaceAction::ToggleTabColor {
                                    color,
                                    tab_index: index,
                                });
                            }
                            ctx.dispatch_typed_action(MenuAction::Close(true));
                        });

                        row.add_child(dot.finish());
                    }

                    row.finish()
                }),
                None,
            )
            .no_highlight_on_hover()
            .with_no_interaction_on_hover(),
        )]
    }

    /// Legacy icon-based color picker with toggle behavior.
    fn legacy_color_option_menu_items(
        &self,
        index: usize,
        terminal_colors: AnsiColors,
    ) -> Vec<MenuItem<WorkspaceAction>> {
        vec![MenuItem::ItemsRow {
            items: TAB_COLOR_OPTIONS
                .iter()
                .map(|color_option| {
                    let color = color_option.to_ansi_color(&terminal_colors);
                    MenuItemFields::new_with_icon(
                        if self.color() == Some(*color_option) {
                            TAB_NO_COLOR_ICON_PATH
                        } else {
                            TAB_COLOR_ICON_PATH
                        },
                        color.into(),
                        color_option.to_string(),
                    )
                    .no_highlight_on_hover()
                    .with_on_select_action(WorkspaceAction::ToggleTabColor {
                        color: *color_option,
                        tab_index: index,
                    })
                })
                .collect(),
        }]
    }
}

/// Stores the state of the tab bar—info related to the entire list of tabs rather than any one
/// specific tab. Passed to the tab component so it can determine how to render.
#[derive(Clone, Copy)]
pub struct TabBarState {
    pub tab_count: usize,
    pub active_tab_index: Option<usize>,
    pub is_any_tab_renaming: bool,
    pub is_any_tab_dragging: bool,
    pub hover_fixed_width: Option<f32>,
}

/// Possible states of the tab indicator.
#[derive(Clone)]
enum Indicator {
    None,
    UnsavedChanges,
    /// This pane's inputs are being synced.
    Synced,
    Error,
    /// At least one of the panes in this tab is being shared.
    Shared,
    /// One of the panes in this tab is maximized.
    Maximized,
    /// We should show a shell indicator for the tab.
    Shell(ShellIndicatorType),
    Agent {
        conversation_status: Option<ConversationStatus>,
    },
    AmbientAgent,
}

impl From<TerminalViewState> for Indicator {
    fn from(value: TerminalViewState) -> Self {
        match value {
            TerminalViewState::Errored => Indicator::Error,
            TerminalViewState::LongRunning => Indicator::None,
            TerminalViewState::Normal => Indicator::None,
        }
    }
}

/// TabComponent is a custom UiComponent responsible for rendering a single tab in the tab bar. It
/// relies on the TabData, and requires the editor (for the tab name editing). Note that since it's
/// a UiComponent its state is not persisted between the renders.
pub struct TabComponent<'a> {
    tab: TabData,
    tab_bar: TabBarState,
    editor: ViewHandle<EditorView>,
    title: String,
    has_custom_title: bool,
    tab_index: usize,
    styles: TabStyles,
    ui_builder: UiBuilder,
    indicator: Indicator,
    close_button_position: TabCloseButtonPosition,
    appearance: &'a Appearance,
    tooltip_message: Option<String>,
    tooltip_directory: Option<String>,
    tooltip_git_branch: Option<String>,
    is_drag_target: bool,
    background_opacity: u8,
    /// Set to `true` when this `TabComponent` is being rendered inside the
    /// floating chip overlay used during a cross-window tab drag. In that
    /// mode `build()` skips the outer `SavePosition`, `Draggable`, and
    /// `DropTarget` wrappers so the chip:
    ///   * does not write to `tab_position_id(tab_index)` in the target
    ///     window's position cache (which would corrupt
    ///     `tab_insertion_index_for_cursor` and cause the empty-slot
    ///     flicker), and
    ///   * does not act as its own draggable / drop target.
    for_drag_ghost: bool,
}

/// Structure that holds TabComponent styles.
struct TabStyles {
    background: Option<ThemeFill>,
    error_color: ColorU,
    sharing_color: ColorU,
    synced_input_indicator_color: ColorU,

    /// Default styles of the TabComponent
    default: UiComponentStyles,
    /// On top of the default styles, active contains extra styling for when the tab is active
    active: UiComponentStyles,
}

impl TabStyles {
    /// Merging tab styles (note that we apply it only to default styles for now).
    fn merge(self, style: UiComponentStyles) -> Self {
        Self {
            default: self.default.merge(style),
            ..self
        }
    }

    /// Returns the default styling (based on the current settings and ui builder, hence not
    /// implementing Default trait).
    fn default(appearance: &Appearance, tab_color: Option<AnsiColorIdentifier>) -> TabStyles {
        let theme = appearance.theme();
        let active_tab_bar_color: Option<ThemeFill> =
            tab_color.map(|color| color.to_ansi_color(&theme.terminal_colors().normal).into());
        let error_color = theme.ui_error_color();
        let sharing_color = shared_session_indicator_color(appearance);
        let background = active_tab_bar_color.map(|color| {
            ThemeFill::VerticalGradient(VerticalGradient::new(
                theme.background().into(),
                color.into(),
            ))
        });
        TabStyles {
            background,
            error_color,
            sharing_color,
            synced_input_indicator_color: ColorU::from_u32(TAB_INDICATOR_SYNCED_COLOR),
            default: UiComponentStyles::default()
                .set_font_color(theme.nonactive_ui_text_color().into())
                .set_font_family_id(appearance.ui_builder().ui_font_family())
                .set_font_size(appearance.ui_builder().ui_font_size()),
            active: UiComponentStyles::default()
                .set_font_color(theme.active_ui_text_color().into())
                .set_font_weight(Weight::Medium)
                .set_border_color(theme.accent().into()),
        }
    }
}

impl<'a> TabComponent<'a> {
    pub fn new(
        tab_index: usize,
        tab_bar: TabBarState,
        tab: &TabData,
        editor: ViewHandle<EditorView>,
        close_button_position: TabCloseButtonPosition,
        is_drag_target: bool,
        ctx: &'a AppContext,
    ) -> Self {
        let appearance = Appearance::as_ref(ctx);
        let title = tab.pane_group.as_ref(ctx).display_title(ctx);

        let active_pane_is_ambient_agent_session = tab
            .pane_group
            .as_ref(ctx)
            .active_session_view(ctx)
            .map(|view| {
                let view = view.as_ref(ctx);
                view.is_ambient_agent_session(ctx) || {
                    let model = view.model.lock();
                    model.is_shared_ambient_agent_session()
                        || matches!(
                            model.conversation_transcript_viewer_status(),
                            Some(
                                crate::terminal::model::terminal_model::ConversationTranscriptViewerStatus::ViewingAmbientConversation(_)
                            )
                        )
                }
            })
            .unwrap_or(false);
        let active_pane_has_unsaved_code_changes = tab
            .pane_group
            .as_ref(ctx)
            .has_active_code_pane_with_unsaved_changes(ctx);
        let is_being_shared = tab
            .pane_group
            .as_ref(ctx)
            .is_terminal_pane_being_shared(ctx);
        let should_show_indicators = *TabSettings::as_ref(ctx).show_indicators.value();
        let are_inputs_synced = SyncedInputState::as_ref(ctx)
            .should_sync_this_pane_group(tab.pane_group.id(), tab.pane_group.window_id(ctx));

        let pane_state_indicator: Indicator = tab
            .pane_group
            .as_ref(ctx)
            .most_recent_pane_state(ctx)
            .into();
        let has_active_pane_state_indicator = !(matches!(pane_state_indicator, Indicator::None));
        let is_maximized = tab.pane_group.as_ref(ctx).is_focused_pane_maximized(ctx);
        let shell_indicator_type = tab.pane_group.as_ref(ctx).focused_shell_indicator_type(ctx);

        // If a session is being shared, we want to show that indicator in the tab bar above all else.
        // Otherwise, if the tab indicator setting is explicitly turned off, we don't want to show any indicator.
        // But if it's on, we want to show the synced indicator if this tab is being synced.
        // If we aren't showing the synced indicator (and we know the setting is on),
        // we will show long-running, error indicators, etc. as applicable.
        let indicator = if active_pane_is_ambient_agent_session {
            Indicator::AmbientAgent
        } else if active_pane_has_unsaved_code_changes {
            Indicator::UnsavedChanges
        } else if FeatureFlag::CreatingSharedSessions.is_enabled() && is_being_shared {
            Indicator::Shared
        } else if !should_show_indicators {
            Indicator::None
        } else if are_inputs_synced {
            Indicator::Synced
        } else if let Some(agent) = Self::agent_indicator(tab, ctx) {
            agent
        } else if let Some(shell_indicator_type) = shell_indicator_type {
            Indicator::Shell(shell_indicator_type)
        } else if has_active_pane_state_indicator {
            pane_state_indicator
        } else if is_maximized {
            Indicator::Maximized
        } else {
            Indicator::None
        };

        let tooltip_message = Self::get_tooltip_message(&indicator, tab, ctx);
        let tooltip_directory = Self::get_tooltip_directory(&indicator, tab, ctx);
        let tooltip_git_branch = Self::get_tooltip_git_branch(&indicator, tab, ctx);
        let window_id = tab.pane_group.window_id(ctx);
        let background_opacity = WindowSettings::as_ref(ctx)
            .background_opacity
            .effective_opacity(window_id, ctx)
            .clamp(20, 100);
        Self {
            tab: tab.clone(),
            tab_bar,
            editor,
            title,
            has_custom_title: tab.pane_group.as_ref(ctx).custom_title(ctx).is_some(),
            tab_index,
            styles: TabStyles::default(appearance, tab.color()),
            ui_builder: appearance.ui_builder().clone(),
            indicator,
            close_button_position,
            appearance,
            tooltip_message,
            tooltip_directory,
            tooltip_git_branch,
            is_drag_target,
            background_opacity,
            for_drag_ghost: false,
        }
    }

    /// Marks this tab as being rendered inside the floating chip used by the
    /// cross-window tab drag overlay. See [`TabComponent::for_drag_ghost`].
    pub fn for_drag_ghost(mut self) -> Self {
        self.for_drag_ghost = true;
        self
    }

    /// Returns the agent indicator for the focused session's active conversation,
    /// or `None` if there is no non-empty, non-passive conversation to display.
    /// When a shell command is long-running the status is overridden to
    /// `InProgress`, matching vertical-tab behavior.
    fn agent_indicator(tab: &TabData, app: &AppContext) -> Option<Indicator> {
        let terminal_view = tab.pane_group.as_ref(app).focused_session_view(app)?;
        let terminal_view_ref = terminal_view.as_ref(app);
        let is_long_running = terminal_view_ref.is_long_running();
        let conversation =
            BlocklistAIHistoryModel::as_ref(app).active_conversation(terminal_view_ref.id())?;

        // Show in-progress indicator when a shell command is running in the AgentView.
        // This matches vertical-tab behavior.
        if is_long_running {
            return Some(Indicator::Agent {
                conversation_status: Some(ConversationStatus::InProgress),
            });
        }

        if conversation.is_empty() || conversation.is_entirely_passive() {
            return None;
        }

        let conversation_status = Some(conversation.status().clone());
        Some(Indicator::Agent {
            conversation_status,
        })
    }

    /// Determine if this tab is the active tab.
    fn is_active_tab(&self) -> bool {
        Some(self.tab_index) == self.tab_bar.active_tab_index
    }

    /// Determine if this tab is currently being renamed.
    ///
    /// Note: Only the active tab can be renamed, part of the rename logic activates the tab before
    /// starting the rename.
    fn is_tab_being_renamed(&self) -> bool {
        self.tab_bar.is_any_tab_renaming && self.is_active_tab()
    }

    /// Determine if this tab is being dragged
    fn is_tab_dragging(&self) -> bool {
        self.tab.draggable_state.is_dragging()
    }

    /// Whether the tab title comes from an agent conversation rather than the
    /// terminal (e.g. shell path). Derived from the already-computed indicator
    /// so the text-clipping direction matches the title content.
    fn has_ai_conversation_title(&self) -> bool {
        Self::is_agent_task_indicator(&self.indicator)
    }

    /// Get the tooltip message for tabs - handles both agent tasks and regular tab titles
    fn get_tooltip_message(
        indicator: &Indicator,
        tab: &TabData,
        ctx: &AppContext,
    ) -> Option<String> {
        if Self::is_agent_task_indicator(indicator) {
            return Self::get_agent_task_tooltip_message(tab, ctx);
        }

        // If we're not showing the conversation title in the tooltip,
        // use the original title from the terminal model.
        let original_title = tab
            .pane_group
            .as_ref(ctx)
            .original_title(ctx)
            .unwrap_or_default();

        let original_title_trimmed = original_title.trim();
        if !original_title_trimmed.is_empty() {
            return Some(original_title_trimmed.to_string());
        }

        None
    }

    /// Get the task description for the tooltip if this is an agent task
    /// and the tooltip content would be different from what's displayed in the tab
    fn get_agent_task_tooltip_message(tab: &TabData, ctx: &AppContext) -> Option<String> {
        let terminal_view_id = tab
            .pane_group
            .as_ref(ctx)
            .focused_session_view(ctx)
            .map(|view| view.id())?;
        let ai_history_model = BlocklistAIHistoryModel::as_ref(ctx);
        let conversation = ai_history_model.active_conversation(terminal_view_id)?;

        // Don't show tooltip for passive conversations
        if conversation.is_entirely_passive() {
            return None;
        }

        let conversation_title = conversation.title()?;
        let trimmed_title = conversation_title.trim().to_owned();

        // Truncate tooltip to prevent rendering issues
        let truncated_name = truncate_from_end(&trimmed_title, MAX_TOOLTIP_LENGTH);

        Some(truncated_name)
    }

    /// Check if the given indicator is an agent task indicator
    fn is_agent_task_indicator(indicator: &Indicator) -> bool {
        matches!(indicator, Indicator::Agent { .. } | Indicator::AmbientAgent)
    }

    /// Get the current working directory for the tooltip if this is an agent task
    fn get_tooltip_directory(
        indicator: &Indicator,
        tab: &TabData,
        ctx: &AppContext,
    ) -> Option<String> {
        if !Self::is_agent_task_indicator(indicator) {
            return None;
        }

        tab.pane_group
            .as_ref(ctx)
            .focused_session_view(ctx)
            .and_then(|view| {
                view.as_ref(ctx)
                    .model
                    .lock()
                    .block_list()
                    .active_block()
                    .metadata()
                    .current_working_directory()
                    .map(|s| s.to_string())
            })
    }

    /// Get the git branch for the tooltip if this is an agent task
    fn get_tooltip_git_branch(
        indicator: &Indicator,
        tab: &TabData,
        ctx: &AppContext,
    ) -> Option<String> {
        if !Self::is_agent_task_indicator(indicator) {
            return None;
        }

        tab.pane_group
            .as_ref(ctx)
            .focused_session_view(ctx)
            .and_then(|view| {
                view.as_ref(ctx)
                    .model
                    .lock()
                    .block_list()
                    .active_block()
                    .git_branch()
                    .cloned()
            })
    }

    /// Generate the SavePosition ID for the tab text content
    fn tab_text_position_id(&self) -> String {
        format!("tab_text_{}", self.tab_index)
    }

    fn render_tab_content(&self) -> Box<dyn Element> {
        let styles = if self.is_active_tab() {
            self.styles.default.merge(self.styles.active)
        } else {
            self.styles.default
        };
        let font_style = styles.font_properties();
        let font_color = styles.font_color.expect("Font color is set");

        if self.is_tab_being_renamed() {
            Align::new(
                TextInput::new(
                    self.editor.clone(),
                    UiComponentStyles::default()
                        .set_background(Fill::None)
                        .set_border_radius(CornerRadius::with_all(Radius::Pixels(0.)))
                        .set_border_width(0.),
                )
                .with_style(UiComponentStyles {
                    margin: Some(Coords::default().top(
                        if FeatureFlag::NewTabStyling.is_enabled() {
                            // With the larger tabs in the new ui, we need to give the editor some extra top margin
                            // to make it appear centered
                            8.
                        } else {
                            3.
                        },
                    )),
                    ..Default::default()
                })
                .build()
                .finish(),
            )
            .finish()
        } else {
            Text::new_inline(
                self.title.clone(),
                self.styles
                    .default
                    .font_family_id
                    .expect("Font family defined"),
                self.styles.default.font_size.expect("Font size defined"),
            )
            .with_clip(if self.should_clip_text_start() {
                ClipConfig::start()
            } else {
                ClipConfig::end()
            })
            .with_style(font_style)
            .with_color(font_color)
            .finish()
        }
    }

    fn render_close_tab_button(
        &self,
        background: Option<Fill>,
        is_hovered: bool,
    ) -> Box<dyn Element> {
        let should_render = {
            let is_last_tab = self.tab_bar.tab_count == 1;
            ContextFlag::CloseWindow.is_enabled() || !is_last_tab
        };
        let button = if is_hovered && should_render {
            let tab_index = self.tab_index;
            let close_mouse_state = self.tab.close_mouse_state.clone();
            let position_id = tab_position_id(tab_index);
            let is_last_tab = tab_index == self.tab_bar.tab_count - 1;

            let default_background = background.unwrap_or_else(|| {
                Fill::Solid(coloru_with_opacity(
                    self.appearance.theme().surface_3().into(),
                    TAB_CLOSE_BUTTON_OPACITY,
                ))
            });

            // Create 100% opacity background for hover state
            let hover_background = if let Some(custom_background) = self.styles.background {
                match custom_background {
                    ThemeFill::Solid(color) => Fill::Solid(color),
                    ThemeFill::VerticalGradient(gradient) => {
                        Fill::Solid(gradient.get_most_opaque())
                    }
                    ThemeFill::HorizontalGradient(gradient) => {
                        Fill::Solid(gradient.get_most_opaque())
                    }
                }
            } else {
                Fill::Solid(self.appearance.theme().surface_3().into())
            };

            let default_styles = UiComponentStyles {
                background: Some(default_background),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(2.0))),
                ..UiComponentStyles::default()
            };

            let hover_styles = UiComponentStyles {
                background: Some(hover_background),
                ..default_styles
            };

            icon_button(self.appearance, Icon::X, false, close_mouse_state)
                .with_style(default_styles)
                .with_active_styles(hover_styles)
                .with_hovered_styles(hover_styles)
                .build()
                .on_hover(move |is_hover, ctx, _, _| {
                    if is_hover {
                        // When hover starts, dispatch action with current width
                        if let Some(rect) = ctx.element_position_by_id(&position_id) {
                            ctx.dispatch_typed_action(WorkspaceAction::TabHoverWidthStart {
                                width: rect.width(),
                            });
                        }
                    } else {
                        // When hover ends, dispatch action to clear stored width
                        ctx.dispatch_typed_action(WorkspaceAction::TabHoverWidthEnd);
                    }
                })
                .on_click(move |ctx, _, _| {
                    if is_last_tab {
                        ctx.dispatch_typed_action(WorkspaceAction::TabHoverWidthEnd);
                    }
                    ctx.dispatch_typed_action(WorkspaceAction::CloseTab(tab_index))
                })
                .finish()
        } else {
            ConstrainedBox::new(Empty::new().finish())
                .with_width(ICON_DIMENSIONS)
                .with_height(ICON_DIMENSIONS)
                .finish()
        };

        Align::new(
            SavePosition::new(button, &format!("close_tab_button:{}", self.tab_index)).finish(),
        )
        .finish()
    }

    fn render_indicator(&self) -> Option<Box<dyn Element>> {
        let icon = match &self.indicator {
            Indicator::UnsavedChanges => Some(
                Container::new(
                    Rect::new()
                        .with_background_color(
                            self.appearance
                                .theme()
                                .main_text_color(self.appearance.theme().background())
                                .into(),
                        )
                        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                        .finish(),
                )
                .with_uniform_margin(3.)
                .finish(),
            ),
            Indicator::None => None,
            Indicator::Synced => Some(
                Icon::LinkHorizontal
                    .to_warpui_icon(self.styles.synced_input_indicator_color.into())
                    .finish(),
            ),
            Indicator::Error => Some(
                Icon::AlertTriangle
                    .to_warpui_icon(self.styles.error_color.into())
                    .finish(),
            ),
            Indicator::Shared => Some(
                Icon::Sharing
                    .to_warpui_icon(self.styles.sharing_color.into())
                    .finish(),
            ),
            Indicator::Maximized => Some(
                Icon::Maximize
                    .to_warpui_icon(
                        self.styles
                            .default
                            .font_color
                            .unwrap_or(ColorU::white())
                            .into(),
                    )
                    .finish(),
            ),
            Indicator::Shell(shell_indicator_type) => Some(
                shell_indicator_type
                    .to_icon()
                    .to_warpui_icon(internal_colors::neutral_5(self.appearance.theme()).into())
                    .finish(),
            ),
            Indicator::Agent {
                conversation_status,
            } => {
                if let Some(status) = conversation_status {
                    if FeatureFlag::NewTabStyling.is_enabled() {
                        let icon_size = 22.0 - STATUS_ELEMENT_PADDING * 2.;
                        Some(render_status_element(status, icon_size, self.appearance))
                    } else {
                        Some(status.render_icon(self.appearance).finish())
                    }
                } else {
                    let icon_color = self.appearance.theme().nonactive_ui_text_color();
                    Some(Icon::Oz.to_warpui_icon(icon_color).finish())
                }
            }
            Indicator::AmbientAgent => {
                // Always use the active tab font color for the ambient agent cloud icon, with a safe fallback.
                let active_styles = self.styles.default.merge(self.styles.active);
                let icon_color = active_styles
                    .font_color
                    .unwrap_or_else(|| self.appearance.theme().active_ui_text_color().into());

                let ui_builder = self.ui_builder.clone();
                let mouse_state = self.tab.indicator_hover_state.clone();
                Some(
                    Hoverable::new(mouse_state, move |state| {
                        let mut stack = Stack::new()
                            .with_child(Icon::OzCloud.to_warpui_icon(icon_color.into()).finish());

                        if state.is_hovered() {
                            let tooltip = ui_builder
                                .tool_tip("Cloud agent run".to_string())
                                .build()
                                .finish();
                            stack.add_positioned_overlay_child(
                                tooltip,
                                OffsetPositioning::offset_from_parent(
                                    vec2f(0., 3.),
                                    ParentOffsetBounds::WindowByPosition,
                                    ParentAnchor::BottomMiddle,
                                    ChildAnchor::TopMiddle,
                                ),
                            );
                        }

                        stack.finish()
                    })
                    .finish(),
                )
            }
        };

        icon.map(|icon| {
            Container::new(
                ConstrainedBox::new(icon)
                    .with_max_width(TAB_INDICATOR_HEIGHT)
                    .with_max_height(TAB_INDICATOR_HEIGHT)
                    .finish(),
            )
            .with_margin_right(4.)
            .finish()
        })
    }

    fn render_tab_container(&self, is_hovered: bool) -> Box<dyn Element> {
        let is_tab_dragging = self.is_tab_dragging();
        let is_hovered = is_hovered && !self.tab_bar.is_any_tab_dragging;

        self.render_tab_container_internal(is_hovered, is_tab_dragging)
    }

    fn should_clip_text_start(&self) -> bool {
        !self.has_custom_title && !self.has_ai_conversation_title()
    }

    fn render_tab_container_internal(
        &self,
        is_hovered: bool,
        is_tab_dragging: bool,
    ) -> Box<dyn Element> {
        let theme = self.appearance.theme();
        let is_active = self.is_active_tab();

        let (background_color, border_fill) = if FeatureFlag::NewTabStyling.is_enabled() {
            // If there is a custom tab background, we overlay it with varying opacities.
            let bg = if let Some(custom_background) = self.styles.background {
                let base_opacity = if is_active {
                    60
                } else if is_hovered {
                    40
                } else {
                    20
                };
                let opacity = (base_opacity as f32 * self.background_opacity as f32 / 100.) as u8;
                match custom_background {
                    ThemeFill::Solid(color) => coloru_with_opacity(color, opacity).into(),
                    ThemeFill::VerticalGradient(gradient) => {
                        coloru_with_opacity(gradient.get_most_opaque(), opacity).into()
                    }
                    ThemeFill::HorizontalGradient(gradient) => {
                        coloru_with_opacity(gradient.get_most_opaque(), opacity).into()
                    }
                }
            } else if is_active {
                internal_colors::fg_overlay_2(theme).into()
            } else if is_hovered {
                internal_colors::fg_overlay_1(theme).into()
            } else {
                Fill::None
            };

            let border = if is_active {
                internal_colors::fg_overlay_2(theme)
            } else {
                internal_colors::fg_overlay_1(theme)
            };

            (bg, border)
        } else {
            let tab_opacity = if is_active || is_hovered {
                WARP_2_HOVERED_TAB_COLOR_OPACITY
            } else {
                WARP_2_TAB_COLOR_OPACITY
            };

            let bg = if let Some(custom_background) = self.styles.background {
                match custom_background {
                    ThemeFill::Solid(color) => coloru_with_opacity(color, tab_opacity).into(),
                    ThemeFill::VerticalGradient(gradient) => {
                        coloru_with_opacity(gradient.get_most_opaque(), tab_opacity).into()
                    }
                    ThemeFill::HorizontalGradient(gradient) => {
                        coloru_with_opacity(gradient.get_most_opaque(), tab_opacity).into()
                    }
                }
            } else {
                coloru_with_opacity(theme.surface_3().into(), tab_opacity).into()
            };

            let border = if is_active || is_hovered {
                internal_colors::fg_overlay_2(theme)
            } else {
                internal_colors::fg_overlay_1(theme)
            };

            (bg, border)
        };

        let full_tab_content = {
            let mut flex_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(warpui::elements::CrossAxisAlignment::Center);
            if let Some(indicator) = self.render_indicator() {
                flex_row.add_child(indicator);
            }
            flex_row.add_child(
                Shrinkable::new(
                    1.0,
                    SavePosition::new(self.render_tab_content(), &self.tab_text_position_id())
                        .finish(),
                )
                .finish(),
            );
            Container::new(flex_row.finish())
                .with_horizontal_padding(8.)
                .finish()
        };

        let compact_icon = {
            if let Some(indicator) = self.render_indicator() {
                indicator
            } else {
                // Fallback to terminal icon if no indicator is present
                Icon::Terminal
                    .to_warpui_icon(
                        self.styles
                            .default
                            .font_color
                            .unwrap_or(ColorU::white())
                            .into(),
                    )
                    .finish()
            }
        };
        let compact_tab_content = Clipped::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(warpui::elements::CrossAxisAlignment::Center)
                .with_child(
                    ConstrainedBox::new(compact_icon)
                        .with_max_width(TAB_INDICATOR_HEIGHT)
                        .with_max_height(TAB_INDICATOR_HEIGHT)
                        .finish(),
                )
                .finish(),
        )
        .finish();

        let close_button_background = if let Some(custom_background) = self.styles.background {
            match custom_background {
                ThemeFill::Solid(color) => {
                    Fill::Solid(coloru_with_opacity(color, TAB_CLOSE_BUTTON_OPACITY))
                }
                ThemeFill::VerticalGradient(gradient) => Fill::Solid(coloru_with_opacity(
                    gradient.get_most_opaque(),
                    TAB_CLOSE_BUTTON_OPACITY,
                )),
                ThemeFill::HorizontalGradient(gradient) => Fill::Solid(coloru_with_opacity(
                    gradient.get_most_opaque(),
                    TAB_CLOSE_BUTTON_OPACITY,
                )),
            }
        } else {
            Fill::Solid(coloru_with_opacity(
                theme.surface_3().into(),
                TAB_CLOSE_BUTTON_OPACITY,
            ))
        };

        // The old code always used a negative offset, which I (Harry) think is wrong for the left-side case (pushes outward).
        // We preserve that behavior in the flag-OFF path out of an abundance of caution to avoid breaking existing functionality.
        let (parent_anchor, child_anchor, horizontal_inset) =
            if FeatureFlag::NewTabStyling.is_enabled() {
                if FeatureFlag::TabCloseButtonOnLeft.is_enabled()
                    && matches!(self.close_button_position, TabCloseButtonPosition::Left)
                {
                    (
                        ParentAnchor::MiddleLeft,
                        ChildAnchor::MiddleLeft,
                        TAB_CLOSE_BUTTON_HORIZONTAL_INSET + 4.0,
                    )
                } else {
                    (
                        ParentAnchor::MiddleRight,
                        ChildAnchor::MiddleRight,
                        -(TAB_CLOSE_BUTTON_HORIZONTAL_INSET + 4.0),
                    )
                }
            } else if FeatureFlag::TabCloseButtonOnLeft.is_enabled()
                && matches!(self.close_button_position, TabCloseButtonPosition::Left)
            {
                (
                    ParentAnchor::TopLeft,
                    ChildAnchor::TopLeft,
                    -TAB_CLOSE_BUTTON_HORIZONTAL_INSET,
                )
            } else {
                (
                    ParentAnchor::TopRight,
                    ChildAnchor::TopRight,
                    -TAB_CLOSE_BUTTON_HORIZONTAL_INSET,
                )
            };

        let build_close_button_overlay = |is_hovered: bool| {
            Container::new(
                ConstrainedBox::new(
                    self.render_close_tab_button(Some(close_button_background), is_hovered),
                )
                .with_width(TAB_CLOSE_BUTTON_WIDTH)
                .with_height(TAB_CLOSE_BUTTON_WIDTH)
                .finish(),
            )
            .finish()
        };

        let mut full_stack = Stack::new().with_child(full_tab_content);
        full_stack.add_positioned_child(
            build_close_button_overlay(is_hovered),
            OffsetPositioning::offset_from_parent(
                vec2f(horizontal_inset, 0.0),
                ParentOffsetBounds::ParentByPosition,
                parent_anchor,
                child_anchor,
            ),
        );

        let mut compact_stack = Stack::new().with_child(compact_tab_content);
        // Only show the close button on the active tab for narrow width
        // to prevent accidental clicks
        if self.is_active_tab() {
            compact_stack.add_positioned_child(
                build_close_button_overlay(is_hovered),
                OffsetPositioning::offset_from_parent(
                    vec2f(horizontal_inset, 0.0),
                    ParentOffsetBounds::ParentByPosition,
                    parent_anchor,
                    child_anchor,
                ),
            );
        }

        let stack = SizeConstraintSwitch::new(
            full_stack.finish(),
            vec![(
                SizeConstraintCondition::WidthLessThan(COMPACT_TAB_WIDTH_THRESHOLD),
                compact_stack.finish(),
            )],
        )
        .finish();

        let mut tab = Container::new(stack)
            .with_vertical_padding(2.)
            .with_background(background_color);
        if FeatureFlag::NewTabStyling.is_enabled() {
            let is_first_tab = self.tab_index == 0;
            tab = tab.with_border(
                Border::all(1.)
                    // We only include a left border on the very first tab to avoid double borders.
                    .with_sides(false, is_first_tab, false, true)
                    .with_border_fill(border_fill),
            );
        } else {
            tab = tab
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
                .with_border(Border::all(1.).with_border_fill(border_fill));
        }

        // If the tab is being dragged, add an opaque background behind it
        if is_tab_dragging {
            Container::new(tab.finish())
                .with_background_color(
                    self.ui_builder
                        .warp_theme()
                        .background()
                        .into_solid_bias_top_color(),
                )
                .finish()
        } else if self.for_drag_ghost {
            // The chip overlay is a purely visual snapshot of the source tab.
            // It must not act as a drop target — both because that's not
            // semantically meaningful for an element that follows the cursor,
            // and because the inner `DropTarget`'s position is unrelated to
            // any real tab in the target window.
            tab.finish()
        } else {
            DropTarget::new(
                tab.finish(),
                TabBarDropTargetData {
                    tab_bar_location: TabBarLocation::TabIndex(self.tab_index),
                },
            )
            .finish()
        }
    }
}

/// Determine the `SavePosition` ID for a draggable tab based on its index
pub fn tab_position_id(index: usize) -> String {
    format!("tab_position_{index}")
}

impl UiComponent for TabComponent<'_> {
    type ElementType = Shrinkable;

    fn build(self) -> Self::ElementType {
        let appearance = self.appearance;
        let tab_mouse_state = self.tab.tab_mouse_state.clone();
        let tab_index = self.tab_index;
        let is_tab_being_renamed = self.is_tab_being_renamed();
        let is_last_tab = self.tab_bar.tab_count == 1;
        let hover_fixed_width = self.tab_bar.hover_fixed_width;
        let is_any_tab_dragging = self.tab_bar.is_any_tab_dragging;
        let draggable_state = self.tab.draggable_state.clone();
        let mouse_close_state = self.tab.close_mouse_state.clone();
        // Capture before `self` is moved into the Hoverable closure below.
        let for_drag_ghost = self.for_drag_ghost;

        // Extract values before moving self into closure
        let tooltip_text = self.tooltip_message.clone();
        let tooltip_directory = self.tooltip_directory.clone();
        let tooltip_git_branch = self.tooltip_git_branch.clone();
        let tab_text_position_id = self.tab_text_position_id();
        let tooltip_mouse_state = self.tab.tooltip_mouse_state.clone();

        // Main tab hover (for close button, etc - no delay)
        let mut tab = Hoverable::new(tab_mouse_state, move |state| {
            let is_hovered = state.is_hovered() || self.is_drag_target;
            self.render_tab_container(is_hovered)
        });

        // Add tooltip hover on top with delay if we have a tooltip message
        if let Some(tooltip_text) = tooltip_text {
            let tooltip_text_clone = tooltip_text.clone();
            let tooltip_directory_clone = tooltip_directory.clone();
            let tooltip_git_branch_clone = tooltip_git_branch.clone();

            // Layer the tooltip hover on top
            tab = Hoverable::new(tooltip_mouse_state, move |tooltip_state| {
                let base_tab = tab.finish();

                if tooltip_state.is_hovered() && !is_tab_being_renamed && !is_any_tab_dragging {
                    let font_color = appearance.theme().background().into_solid();

                    let title_text = Text::new(
                        tooltip_text_clone.clone(),
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(font_color)
                    .finish();

                    let has_extra_info =
                        tooltip_directory_clone.is_some() || tooltip_git_branch_clone.is_some();

                    let tooltip_content: Box<dyn Element> = if has_extra_info {
                        let mut column = Flex::column().with_child(title_text);

                        if let Some(directory) = &tooltip_directory_clone {
                            let folder_icon = Icon::Folder
                                .to_warpui_icon(ThemeFill::Solid(font_color))
                                .finish();

                            let directory_row = Flex::row()
                                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                .with_child(
                                    ConstrainedBox::new(folder_icon)
                                        .with_height(appearance.ui_font_size())
                                        .with_width(appearance.ui_font_size())
                                        .finish(),
                                )
                                .with_child(
                                    Container::new(
                                        Text::new(
                                            directory.clone(),
                                            appearance.ui_font_family(),
                                            appearance.ui_font_size(),
                                        )
                                        .with_color(font_color)
                                        .finish(),
                                    )
                                    .with_margin_left(4.)
                                    .finish(),
                                )
                                .finish();

                            column.add_child(
                                Container::new(directory_row).with_margin_top(4.).finish(),
                            );
                        }

                        if let Some(branch) = &tooltip_git_branch_clone {
                            let branch_icon = Icon::GitBranch
                                .to_warpui_icon(ThemeFill::Solid(font_color))
                                .finish();

                            let branch_row = Flex::row()
                                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                .with_child(
                                    ConstrainedBox::new(branch_icon)
                                        .with_height(appearance.ui_font_size())
                                        .with_width(appearance.ui_font_size())
                                        .finish(),
                                )
                                .with_child(
                                    Container::new(
                                        Text::new(
                                            branch.clone(),
                                            appearance.ui_font_family(),
                                            appearance.ui_font_size(),
                                        )
                                        .with_color(font_color)
                                        .finish(),
                                    )
                                    .with_margin_left(4.)
                                    .finish(),
                                )
                                .finish();

                            column
                                .add_child(Container::new(branch_row).with_margin_top(4.).finish());
                        }

                        column.finish()
                    } else {
                        title_text
                    };

                    let tooltip = Container::new(tooltip_content)
                        .with_background(appearance.theme().tooltip_background())
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                        .with_padding(Padding::uniform(6.))
                        .finish();

                    let mut stack = Stack::new().with_child(base_tab);

                    stack.add_positioned_overlay_child(
                        tooltip,
                        OffsetPositioning::offset_from_save_position_element(
                            tab_text_position_id.clone(),
                            vec2f(0., 8.),
                            PositionedElementOffsetBounds::WindowByPosition,
                            PositionedElementAnchor::BottomLeft,
                            ChildAnchor::TopLeft,
                        ),
                    );
                    return stack.finish();
                }

                base_tab
            })
            .with_hover_in_delay(Duration::from_millis(500));
        }

        // Copy the values so they can be passed within the on_click and on_right_click handlers

        // We only want the on_click action to take effect on the tab, if it's not being renamed at a moment.
        // Note that clicking on other tabs is still ok.
        if !is_tab_being_renamed {
            tab = tab.on_mouse_down(move |ctx, _app, _| {
                let is_hovered = mouse_close_state
                    .lock()
                    .expect("lock acquired")
                    .is_hovered();
                if !is_hovered {
                    ctx.dispatch_typed_action(WorkspaceAction::ActivateTab(tab_index));
                }
            });

            tab = tab.on_double_click(move |ctx, _app, _| {
                ctx.dispatch_typed_action(WorkspaceAction::RenameTab(tab_index));
            });
        }

        tab = tab.on_right_click(move |ctx, _app, position| {
            ctx.dispatch_typed_action(WorkspaceAction::ToggleTabRightClickMenu {
                tab_index,
                anchor: TabContextMenuAnchor::Pointer(position),
            });
        });

        if ContextFlag::CloseWindow.is_enabled() || !is_last_tab {
            tab = tab.on_middle_click(move |ctx, _app, _position| {
                ctx.dispatch_typed_action(WorkspaceAction::CloseTab(tab_index));
            });
        }

        // Note: Tooltip delay is now handled separately in the tooltip overlay

        let constrained_tab = if let Some(fixed_width) = hover_fixed_width {
            // Use fixed width when hovering over a close button
            ConstrainedBox::new(tab.finish())
                .with_width(fixed_width)
                .finish()
        } else {
            // Use dynamic sizing when not hovering
            ConstrainedBox::new(tab.finish())
                .with_max_width(200.)
                .finish()
        };

        // Skip the `Draggable` and `SavePosition` wrappers when rendering
        // the tab inside the cross-window drag chip overlay. Wrapping the
        // chip in another `Draggable` would interfere with the in-flight
        // drag, and writing a `SavePosition` keyed by `tab_position_id(0)`
        // would clobber the target window's real tab 0 entry in the
        // position cache, breaking `tab_insertion_index_for_cursor`.
        let full_tab: Box<dyn Element> = if for_drag_ghost {
            constrained_tab
        } else {
            let draggable = Draggable::new(draggable_state, constrained_tab)
                .on_drag_start(|ctx, _, _| ctx.dispatch_typed_action(WorkspaceAction::StartTabDrag))
                .on_drag(move |ctx, _, rect, _| {
                    ctx.dispatch_typed_action(WorkspaceAction::DragTab {
                        tab_index,
                        tab_position: rect,
                    });
                })
                .on_drop(|ctx, _, _, _| ctx.dispatch_typed_action(WorkspaceAction::DropTab));
            let draggable = if FeatureFlag::DragTabsToWindows.is_enabled() {
                draggable
            } else {
                draggable.with_drag_axis(DragAxis::HorizontalOnly)
            };
            let tab_with_drag: Box<dyn Element> = draggable.finish();
            SavePosition::new(tab_with_drag, &tab_position_id(tab_index)).finish()
        };

        if FeatureFlag::NewTabStyling.is_enabled() {
            Shrinkable::new(1.0, full_tab)
        } else {
            Shrinkable::new(
                1.0,
                Container::new(full_tab)
                    .with_vertical_margin(4.)
                    .with_margin_left(8.)
                    .finish(),
            )
        }
    }

    fn with_style(self, style: UiComponentStyles) -> Self {
        Self {
            styles: self.styles.merge(style),
            ..self
        }
    }
}
