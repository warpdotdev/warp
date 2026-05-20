pub mod telemetry;

use crate::ai::agent::conversation::{ConversationStatus, StatusColorStyle};
use crate::ai::agent_management::AgentNotificationsModel;
use crate::ai::cloud_environments::CloudAmbientAgentEnvironment;
use crate::ai::conversation_status_ui::render_status_element;
use crate::cloud_object::model::generic_string_model::StringModel;
use crate::code::editor::{add_color, remove_color};
use crate::code::icon_from_file_path;
use crate::safe_triangle::SafeTriangle;
use crate::send_telemetry_from_app_ctx;
use crate::terminal::cli_agent_sessions::listener::agent_supports_rich_status;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::view::TerminalViewState;
use crate::terminal::CLIAgent;
use crate::ui_components::agent_icon::terminal_view_agent_icon_variant;
use crate::ui_components::icon_with_status::{render_icon_with_status, IconWithStatusVariant};
use crate::workspace::view::vertical_tabs::telemetry::{
    VerticalTabsChipEntrypoint, VerticalTabsTelemetryEvent,
};
use crate::FeatureFlag;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::appearance::Appearance;
use crate::context_chips::display_chip::GitLineChanges;
use crate::context_chips::github_pr_display_text_from_url;
use crate::drive::{cloud_object_styling::warp_drive_icon_color, DriveObjectType};
use crate::editor::EditorView;
use crate::pane_group::pane::IPaneType;
use crate::pane_group::TerminalPane;
use crate::pane_group::{
    CodePane, NotebookPane, PaneGroup, PaneId, TabBarHoverIndex, WorkflowPane,
};
use crate::tab::{tab_position_id, SelectedTabColor, TabData};
use crate::terminal::session_settings::SessionSettings;
use crate::terminal::TerminalView;
use crate::themes::theme::Fill as ThemeFill;
use crate::ui_components::buttons::combo_inner_button;
use crate::ui_components::icons::Icon as UiIcon;
use crate::util::bindings::keybinding_name_to_display_string;
use crate::util::color::Opacity;
use crate::workspace::action::WorkspaceAction;
use crate::workspace::cross_window_tab_drag::CrossWindowTabDrag;
use crate::workspace::hoa_onboarding::HoaOnboardingStep;
use crate::workspace::tab_settings::{
    TabSettings, VerticalTabsCompactSubtitle, VerticalTabsDisplayGranularity,
    VerticalTabsPrimaryInfo, VerticalTabsTabItemMode, VerticalTabsViewMode,
};
use crate::workspace::{
    PaneViewLocator, TabBarLocation, TabContextMenuAnchor, VerticalTabsPaneContextMenuTarget,
    VerticalTabsPaneDropTargetData, Workspace,
};
use languages::language_by_filename;

use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use settings::Setting as _;
use std::path::{Path, PathBuf};
use warp_core::context_flag::ContextFlag;
use warp_core::telemetry::TelemetryEvent as _;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::color::coloru_with_opacity;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::{AnsiColorIdentifier, Fill as WarpThemeFill, WarpTheme};
use warp_core::ui::Icon as WarpIcon;
use warpui::elements::DispatchEventResult;
use warpui::elements::{
    resizable_state_handle, Border, ChildAnchor, Clipped, ClippedScrollStateHandle,
    ClippedScrollable, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DragAxis,
    DragBarSide, Draggable, DropShadow, DropTarget, Element, Empty, EventHandler, Expanded,
    Fill as ElementFill, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    OffsetPositioning, Padding, ParentAnchor, ParentElement, ParentOffsetBounds,
    PositionedElementAnchor, PositionedElementOffsetBounds, Radius, Resizable,
    ResizableStateHandle, SavePosition, ScrollTarget, ScrollToPositionMode, ScrollbarWidth,
    Shrinkable, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::prelude::Align;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::ui_components::text_input::TextInput;
use warpui::{AppContext, EntityId, SingletonEntity, ViewHandle, WindowId};

const PANEL_WIDTH: f32 = 248.;
const MIN_PANEL_WIDTH: f32 = 200.;
const MAX_PANEL_WIDTH_RATIO: f32 = 0.5;
const DETAIL_SIDECAR_SECTION_PADDING: f32 = 12.;
const DETAIL_SIDECAR_SECTION_GAP: f32 = 4.;
const GROUP_HEADER_VERTICAL_PADDING: f32 = 4.;
const GROUP_HORIZONTAL_PADDING: f32 = 8.;
const GROUP_BODY_BOTTOM_PADDING: f32 = 8.;
const GROUP_ITEM_SPACING: f32 = 4.;
const TABS_MODE_ITEM_SPACING: f32 = 4.;
const GROUP_ACTION_BUTTON_ICON_SIZE: f32 = 12.;
const GROUP_ACTION_BUTTON_PADDING: f32 = 2.;
const GROUP_ACTION_BUTTON_GAP: f32 = 2.;
const ROW_CORNER_RADIUS: f32 = 4.;
const BADGE_ICON_SIZE: f32 = 12.;
const DETAIL_SIDECAR_DEFAULT_WIDTH: f32 = 320.;
const DETAIL_SIDECAR_MIN_WIDTH: f32 = 240.;
const DETAIL_SIDECAR_CORNER_RADIUS: f32 = 4.;
/// Fixed height of the metadata row (line 3 in expanded mode). Matches the passive badge height
/// so the row doesn't resize when badges are toggled.
const METADATA_ROW_HEIGHT: f32 = BADGE_ICON_SIZE + 2.;
const TAB_COLOR_OPACITY: Opacity = 15;
const TAB_COLOR_HOVER_OPACITY: Opacity = 50;

// Circular icon constants
const ICON_WITH_STATUS_GAP: f32 = 8.;
pub(super) const VERTICAL_TABS_DETAIL_SIDECAR_POSITION_ID: &str = "vertical_tabs:detail_sidecar";

/// Total size of the icon-with-status component rendered for each vertical-tabs row.
/// Sub-components (circle, badge, cloud) are derived inside `render_icon_with_status`.
const VERTICAL_TABS_ICON_SIZE: f32 = 24.;

/// Icon size for the per-line conversation status pill in Summary mode. Pairs with
/// `STATUS_ELEMENT_PADDING` (2px) for an overall ~14px element next to a 12pt title.
const VERTICAL_TABS_SUMMARY_STATUS_ICON_SIZE: f32 = 10.;

fn vtab_pane_row_position_id(pane_group_id: EntityId, pane_id: PaneId) -> String {
    format!("vertical_tabs:pane_row:{pane_group_id:?}:{pane_id}")
}

fn terminal_title_fallback_font(agent_text: &TerminalAgentText) -> TerminalPrimaryLineFont {
    if agent_text.cli_agent.is_some() {
        TerminalPrimaryLineFont::Ui
    } else {
        TerminalPrimaryLineFont::Monospace
    }
}

fn supports_vertical_tabs_detail_sidecar(typed: &TypedPane<'_>) -> bool {
    typed.supports_vertical_tabs_detail_sidecar()
}

fn detail_target_for_hovered_row(
    pane_group_id: EntityId,
    pane_id: PaneId,
    granularity: VerticalTabsDisplayGranularity,
) -> VerticalTabsDetailTarget {
    match granularity {
        VerticalTabsDisplayGranularity::Panes => VerticalTabsDetailTarget::Pane {
            pane_group_id,
            pane_id,
        },
        VerticalTabsDisplayGranularity::Tabs => VerticalTabsDetailTarget::Tab {
            pane_group_id,
            source_pane_id: pane_id,
        },
    }
}

fn detail_target_kind(target: VerticalTabsDetailTarget) -> VerticalTabsDetailTargetKind {
    match target {
        VerticalTabsDetailTarget::Pane { .. } => VerticalTabsDetailTargetKind::Pane,
        VerticalTabsDetailTarget::Tab { .. } => VerticalTabsDetailTargetKind::Tab,
    }
}

/// Returns whether the current pointer geometry still justifies keeping the vertical-tabs detail
/// sidecar visible, independent of potentially stale element-local hover state.
fn should_keep_detail_sidecar_visible_for_mouse_position(
    position: Vector2F,
    row_rect: Option<RectF>,
    sidecar_rect: Option<RectF>,
    safe_triangle: &mut SafeTriangle,
) -> bool {
    safe_triangle.set_target_rect(sidecar_rect);

    if row_rect.is_some_and(|rect| rect.contains_point(position)) {
        safe_triangle.update_position(position);
        return true;
    }

    let Some(sidecar_rect) = sidecar_rect else {
        safe_triangle.update_position(position);
        return true;
    };

    if sidecar_rect.contains_point(position) {
        safe_triangle.update_position(position);
        return true;
    }

    let suppress_hover = safe_triangle.should_suppress_hover(position);
    safe_triangle.update_position(position);
    suppress_hover
}

fn visible_pane_ids_for_detail_target<F>(
    visible_pane_ids: &[PaneId],
    source_pane_id: PaneId,
    target_kind: VerticalTabsDetailTargetKind,
    mut is_supported: F,
) -> Option<Vec<PaneId>>
where
    F: FnMut(PaneId) -> bool,
{
    if !visible_pane_ids.contains(&source_pane_id) {
        return None;
    }

    match target_kind {
        VerticalTabsDetailTargetKind::Pane => {
            is_supported(source_pane_id).then_some(vec![source_pane_id])
        }
        VerticalTabsDetailTargetKind::Tab => visible_pane_ids
            .iter()
            .copied()
            .all(&mut is_supported)
            .then(|| visible_pane_ids.to_vec()),
    }
}

fn pane_ids_for_detail_target(
    pane_group: &PaneGroup,
    target: VerticalTabsDetailTarget,
    app: &AppContext,
) -> Option<Vec<PaneId>> {
    let visible_pane_ids = pane_group.visible_pane_ids();
    visible_pane_ids_for_detail_target(
        &visible_pane_ids,
        target.source_pane_id(),
        detail_target_kind(target),
        |pane_id| {
            pane_group
                .pane_by_id(pane_id)
                .map(|_| {
                    supports_vertical_tabs_detail_sidecar(
                        &pane_group.resolve_pane_type(pane_id, app),
                    )
                })
                .unwrap_or(false)
        },
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TerminalPrimaryLineFont {
    Ui,
    Monospace,
}

fn oz_icon_fill(theme: &WarpTheme) -> WarpThemeFill {
    theme.main_text_color(theme.background())
}

fn render_pane_icon_with_status(
    variant: IconWithStatusVariant,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    render_icon_with_status(
        variant,
        VERTICAL_TABS_ICON_SIZE,
        0.,
        theme,
        theme.background(),
    )
}

#[derive(Clone, Default)]
struct PaneGroupStateHandles {
    group: MouseStateHandle,
    header: MouseStateHandle,
    kebab: MouseStateHandle,
    close: MouseStateHandle,
    action_buttons: MouseStateHandle,
}

fn pane_row_background(
    pane_color: Option<ThemeFill>,
    is_selected: bool,
    is_hovered: bool,
    is_being_dragged: bool,
    theme: &WarpTheme,
) -> Option<ThemeFill> {
    if let Some(color) = pane_color {
        let opacity = if is_selected || is_hovered {
            TAB_COLOR_HOVER_OPACITY
        } else {
            TAB_COLOR_OPACITY
        };
        Some(color.with_opacity(opacity))
    } else if is_selected {
        Some(internal_colors::fg_overlay_2(theme))
    } else if is_being_dragged || is_hovered {
        Some(internal_colors::fg_overlay_1(theme))
    } else {
        None
    }
}

fn render_pane_row_element(
    props: PaneProps<'_>,
    padding: Padding,
    defer_events_to_children: bool,
    content: Box<dyn Element>,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    let detail_target = supports_vertical_tabs_detail_sidecar(&props.typed).then(|| {
        detail_target_for_hovered_row(
            props.pane_group_id,
            props.pane_id,
            props.display_granularity,
        )
    });
    let row_position_id = vtab_pane_row_position_id(props.pane_group_id, props.pane_id);
    let PaneProps {
        pane_id,
        pane_group_id,
        is_active_tab,
        mouse_state,
        title_mouse_state: _,
        title: _,
        subtitle: _,
        custom_vertical_tabs_title: _,
        display_title_override: _,
        is_focused,
        typed: _,
        is_being_dragged,
        pane_color,
        badge_mouse_states: _,
        detail_hover_state,
        display_granularity: _,
        renamable_tab_index,
        pane_context_menu_tab_index,
        is_tab_being_renamed,
        rename_editor: _,
        is_pane_being_renamed,
        pane_rename_editor: _,
    } = props;
    let is_selected = is_active_tab && is_focused;
    let mut row = Hoverable::new(mouse_state, move |state| {
        let mut container = Container::new(Clipped::new(content).finish())
            .with_padding(padding)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_CORNER_RADIUS)));

        if let Some(background) = pane_row_background(
            pane_color,
            is_selected,
            state.is_hovered(),
            is_being_dragged,
            theme,
        ) {
            container = container.with_background(background);
        }

        container
            .with_border(Border::all(1.).with_border_fill(if is_selected {
                internal_colors::fg_overlay_3(theme).into()
            } else {
                ElementFill::None
            }))
            .finish()
    })
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(WorkspaceAction::FocusPane(PaneViewLocator {
            pane_group_id,
            pane_id,
        }));
    })
    .on_hover(move |is_hovered, ctx, app, position| {
        let show_details_on_hover = *TabSettings::as_ref(app)
            .vertical_tabs_show_details_on_hover
            .value();
        let mut overlay_state = detail_hover_state
            .overlay_state
            .lock()
            .expect("vertical tabs detail overlay lock poisoned");
        if !show_details_on_hover {
            if overlay_state.active_target.is_some() {
                overlay_state.active_target = None;
                overlay_state.safe_triangle.set_target_rect(None);
                ctx.notify();
            }
            return;
        }
        let sidecar_rect = app.element_position_by_id_at_last_frame(
            detail_hover_state.window_id,
            VERTICAL_TABS_DETAIL_SIDECAR_POSITION_ID,
        );
        let sidecar_hovered = detail_hover_state
            .sidecar_mouse_state
            .lock()
            .expect("detail sidecar hover state lock poisoned")
            .is_mouse_over_element();
        overlay_state.safe_triangle.set_target_rect(sidecar_rect);

        let suppress_hover = overlay_state.safe_triangle.should_suppress_hover(position);
        overlay_state.safe_triangle.update_position(position);

        let mut changed = false;
        if is_hovered {
            if !suppress_hover && overlay_state.active_target != detail_target {
                overlay_state.active_target = detail_target;
                if detail_target.is_none() {
                    overlay_state.safe_triangle.set_target_rect(None);
                }
                changed = true;
            }
        } else if !suppress_hover
            && !sidecar_hovered
            && overlay_state.active_target == detail_target
        {
            overlay_state.active_target = None;
            overlay_state.safe_triangle.set_target_rect(None);
            changed = true;
        }

        if changed {
            ctx.notify();
        }
    })
    .with_skip_synthetic_hover_out()
    .with_cursor(Cursor::PointingHand);

    if let Some(tab_index) = renamable_tab_index.filter(|_| !is_tab_being_renamed) {
        row = row.on_double_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::RenameTab(tab_index));
        });
    }
    let pane_locator = PaneViewLocator {
        pane_group_id,
        pane_id,
    };
    if pane_context_menu_tab_index.is_some() && !is_pane_being_renamed {
        row = row.on_double_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::RenamePane(pane_locator));
        });
    }
    if let Some(tab_index) = pane_context_menu_tab_index {
        row = row.on_right_click(move |ctx, _, position| {
            ctx.dispatch_typed_action(WorkspaceAction::ToggleVerticalTabsPaneContextMenu {
                tab_index,
                target: VerticalTabsPaneContextMenuTarget::ClickedPane(pane_locator),
                position,
            });
        });
    }

    if defer_events_to_children {
        row = row.with_defer_events_to_children();
    }
    SavePosition::new(row.finish(), &row_position_id).finish()
}

#[derive(Clone, Default)]
struct PaneRowBadgeMouseStates {
    diff_stats: MouseStateHandle,
    pull_request: MouseStateHandle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VerticalTabsDetailTarget {
    Pane {
        pane_group_id: EntityId,
        pane_id: PaneId,
    },
    Tab {
        pane_group_id: EntityId,
        source_pane_id: PaneId,
    },
}

impl VerticalTabsDetailTarget {
    fn pane_group_id(&self) -> EntityId {
        match self {
            Self::Pane { pane_group_id, .. } | Self::Tab { pane_group_id, .. } => *pane_group_id,
        }
    }

    fn source_pane_id(&self) -> PaneId {
        match self {
            Self::Pane { pane_id, .. } => *pane_id,
            Self::Tab { source_pane_id, .. } => *source_pane_id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VerticalTabsDetailTargetKind {
    Pane,
    Tab,
}

struct VerticalTabsDetailOverlayState {
    active_target: Option<VerticalTabsDetailTarget>,
    safe_triangle: SafeTriangle,
}

impl Default for VerticalTabsDetailOverlayState {
    fn default() -> Self {
        Self {
            active_target: None,
            safe_triangle: SafeTriangle::new(),
        }
    }
}

#[derive(Clone)]
pub(super) struct VerticalTabsDetailHoverState {
    overlay_state: Arc<Mutex<VerticalTabsDetailOverlayState>>,
    sidecar_mouse_state: MouseStateHandle,
    window_id: WindowId,
}

impl VerticalTabsDetailHoverState {
    pub(super) fn reconcile_visibility_for_mouse_position(
        &self,
        position: Vector2F,
        app: &AppContext,
    ) -> bool {
        let mut overlay_state = self
            .overlay_state
            .lock()
            .expect("vertical tabs detail overlay lock poisoned");
        let Some(active_target) = overlay_state.active_target else {
            return false;
        };

        let row_rect = app.element_position_by_id_at_last_frame(
            self.window_id,
            vtab_pane_row_position_id(
                active_target.pane_group_id(),
                active_target.source_pane_id(),
            ),
        );
        let sidecar_rect = app.element_position_by_id_at_last_frame(
            self.window_id,
            VERTICAL_TABS_DETAIL_SIDECAR_POSITION_ID,
        );
        if should_keep_detail_sidecar_visible_for_mouse_position(
            position,
            row_rect,
            sidecar_rect,
            &mut overlay_state.safe_triangle,
        ) {
            return false;
        }

        overlay_state.active_target = None;
        overlay_state.safe_triangle.set_target_rect(None);
        drop(overlay_state);

        if let Ok(mut mouse_state) = self.sidecar_mouse_state.lock() {
            mouse_state.reset_interaction_state();
        }

        true
    }
}

pub(super) struct VerticalTabsPanelState {
    scroll_state: ClippedScrollStateHandle,
    resizable_state: ResizableStateHandle,
    group_mouse_states: RefCell<HashMap<EntityId, PaneGroupStateHandles>>,
    pane_row_mouse_states: RefCell<HashMap<PaneId, MouseStateHandle>>,
    pane_title_mouse_states: RefCell<HashMap<PaneId, MouseStateHandle>>,
    pane_badge_mouse_states: RefCell<HashMap<PaneId, PaneRowBadgeMouseStates>>,
    detail_pane_badge_mouse_states: RefCell<HashMap<PaneId, PaneRowBadgeMouseStates>>,
    detail_scroll_state: ClippedScrollStateHandle,
    detail_sidecar_mouse_state: MouseStateHandle,
    detail_overlay_state: Arc<Mutex<VerticalTabsDetailOverlayState>>,
    new_tab_hover_state: MouseStateHandle,
    new_tab_button_state: MouseStateHandle,
    pub(super) search_query: String,
    settings_button_mouse_state: MouseStateHandle,
    panes_segment_mouse_state: MouseStateHandle,
    tabs_segment_mouse_state: MouseStateHandle,
    focused_session_option_mouse_state: MouseStateHandle,
    summary_option_mouse_state: MouseStateHandle,
    compact_segment_mouse_state: MouseStateHandle,
    expanded_segment_mouse_state: MouseStateHandle,
    command_option_mouse_state: MouseStateHandle,
    directory_option_mouse_state: MouseStateHandle,
    branch_option_mouse_state: MouseStateHandle,
    subtitle_option_1_mouse_state: MouseStateHandle,
    subtitle_option_2_mouse_state: MouseStateHandle,
    show_pr_link_mouse_state: MouseStateHandle,
    show_pr_link_info_tooltip_mouse_state: MouseStateHandle,
    show_diff_stats_mouse_state: MouseStateHandle,
    show_details_on_hover_mouse_state: MouseStateHandle,
    pub(super) show_settings_popup: bool,
}

impl Default for VerticalTabsPanelState {
    fn default() -> Self {
        Self {
            scroll_state: ClippedScrollStateHandle::default(),
            resizable_state: resizable_state_handle(PANEL_WIDTH),
            group_mouse_states: RefCell::default(),
            pane_row_mouse_states: RefCell::default(),
            pane_title_mouse_states: RefCell::default(),
            pane_badge_mouse_states: RefCell::default(),
            detail_pane_badge_mouse_states: RefCell::default(),
            detail_scroll_state: ClippedScrollStateHandle::default(),
            detail_sidecar_mouse_state: Default::default(),
            detail_overlay_state: Arc::new(Mutex::new(VerticalTabsDetailOverlayState::default())),
            new_tab_hover_state: Default::default(),
            new_tab_button_state: Default::default(),
            search_query: String::new(),
            settings_button_mouse_state: Default::default(),
            panes_segment_mouse_state: Default::default(),
            tabs_segment_mouse_state: Default::default(),
            focused_session_option_mouse_state: Default::default(),
            summary_option_mouse_state: Default::default(),
            compact_segment_mouse_state: Default::default(),
            expanded_segment_mouse_state: Default::default(),
            command_option_mouse_state: Default::default(),
            directory_option_mouse_state: Default::default(),
            branch_option_mouse_state: Default::default(),
            subtitle_option_1_mouse_state: Default::default(),
            subtitle_option_2_mouse_state: Default::default(),
            show_pr_link_mouse_state: Default::default(),
            show_pr_link_info_tooltip_mouse_state: Default::default(),
            show_diff_stats_mouse_state: Default::default(),
            show_details_on_hover_mouse_state: Default::default(),
            show_settings_popup: false,
        }
    }
}

impl VerticalTabsPanelState {
    /// Returns a lightweight handle bundle for workspace-level visibility reconciliation while the
    /// detail sidecar is active.
    pub(super) fn detail_hover_state(&self, window_id: WindowId) -> VerticalTabsDetailHoverState {
        VerticalTabsDetailHoverState {
            overlay_state: self.detail_overlay_state.clone(),
            sidecar_mouse_state: self.detail_sidecar_mouse_state.clone(),
            window_id,
        }
    }

    pub(super) fn has_active_detail_target(&self) -> bool {
        self.detail_overlay_state
            .lock()
            .map(|overlay_state| overlay_state.active_target.is_some())
            .unwrap_or(false)
    }

    pub(super) fn clear_detail_sidecar(&self) {
        if let Ok(mut overlay_state) = self.detail_overlay_state.lock() {
            overlay_state.active_target = None;
            overlay_state.safe_triangle.set_target_rect(None);
        }
        if let Ok(mut mouse_state) = self.detail_sidecar_mouse_state.lock() {
            mouse_state.reset_interaction_state();
        }
    }

    /// Clears the detail sidecar only if it is currently anchored to a pane row in the given
    /// pane group. Used when a tab (and therefore its pane rows) is about to go away so the
    /// sidecar doesn't try to position itself against a missing anchor on the next render.
    pub(super) fn clear_detail_sidecar_if_for_pane_group(&self, pane_group_id: EntityId) {
        let matches = self
            .detail_overlay_state
            .lock()
            .map(|overlay_state| {
                overlay_state
                    .active_target
                    .is_some_and(|target| target.pane_group_id() == pane_group_id)
            })
            .unwrap_or(false);
        if matches {
            self.clear_detail_sidecar();
        }
    }
}

struct PaneProps<'a> {
    pane_id: PaneId,
    pane_group_id: EntityId,
    is_active_tab: bool,
    mouse_state: MouseStateHandle,
    title_mouse_state: Option<MouseStateHandle>,
    title: String,
    subtitle: String,
    custom_vertical_tabs_title: Option<String>,
    display_title_override: Option<String>,
    is_focused: bool,
    typed: TypedPane<'a>,
    is_being_dragged: bool,
    pane_color: Option<ThemeFill>,
    badge_mouse_states: PaneRowBadgeMouseStates,
    detail_hover_state: VerticalTabsDetailHoverState,
    display_granularity: VerticalTabsDisplayGranularity,
    renamable_tab_index: Option<usize>,
    pane_context_menu_tab_index: Option<usize>,
    is_tab_being_renamed: bool,
    rename_editor: Option<ViewHandle<EditorView>>,
    is_pane_being_renamed: bool,
    pane_rename_editor: Option<ViewHandle<EditorView>>,
}

struct PaneRowState {
    mouse_state: MouseStateHandle,
    title_mouse_state: Option<MouseStateHandle>,
    pane_color: Option<ThemeFill>,
    badge_mouse_states: PaneRowBadgeMouseStates,
}

enum TerminalPrimaryLineData {
    StatusText {
        text: String,
    },
    Text {
        text: String,
        font: TerminalPrimaryLineFont,
    },
}

impl TerminalPrimaryLineData {
    fn text(&self) -> &str {
        match self {
            TerminalPrimaryLineData::StatusText { text, .. }
            | TerminalPrimaryLineData::Text { text, .. } => text,
        }
    }
}

enum TabGroupColorMode {
    Uniform(ThemeFill),
    PerPane(HashMap<PaneId, Option<ThemeFill>>),
    None,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VerticalTabsResolvedMode {
    Panes,
    FocusedSession,
    Summary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SummaryPaneKind {
    Terminal,
    OzAgent { is_ambient: bool },
    CLIAgent { agent: CLIAgent, is_ambient: bool },
    Code { title: String },
    CodeDiff,
    File,
    Notebook { is_plan: bool },
    Workflow { is_ai_prompt: bool },
    Settings,
    EnvVarCollection,
    EnvironmentManagement,
    AIFact,
    AIDocument,
    ExecutionProfileEditor,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SummaryPaneKindIcons {
    Single(SummaryPaneKind),
    Pair {
        primary: SummaryPaneKind,
        secondary: SummaryPaneKind,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct VerticalTabsSummaryBranchEntry {
    repo_path: PathBuf,
    branch_name: String,
    diff_stats: Option<GitLineChanges>,
    pull_request_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct VerticalTabsSummaryPrimaryLabel {
    text: String,
    /// Some when the contributing pane is a conversation with a known status. Drives the
    /// per-line status pill prefix in Summary mode.
    status: Option<ConversationStatus>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct VerticalTabsSummaryData {
    primary_labels: Vec<VerticalTabsSummaryPrimaryLabel>,
    working_directories: Vec<String>,
    branch_entries: Vec<VerticalTabsSummaryBranchEntry>,
}

impl TabGroupColorMode {
    fn into_per_pane_colors(
        self,
        visible_pane_ids: &[PaneId],
    ) -> Option<HashMap<PaneId, Option<ThemeFill>>> {
        match self {
            TabGroupColorMode::PerPane(map) => Some(map),
            TabGroupColorMode::Uniform(fill) => Some(
                visible_pane_ids
                    .iter()
                    .map(|&id| (id, Some(fill)))
                    .collect(),
            ),
            TabGroupColorMode::None => None,
        }
    }
}

struct GroupHeaderProps<'a> {
    tab_index: usize,
    pane_group: &'a PaneGroup,
    is_being_renamed: bool,
    rename_editor: ViewHandle<EditorView>,
    header_mouse_state: MouseStateHandle,
}

#[derive(Clone, Copy)]
struct TabGroupDragState {
    is_any_pane_dragging: bool,
    insert_before_index: usize,
    insert_after_index: Option<usize>,
}

fn resolve_vertical_tabs_mode(app: &AppContext) -> VerticalTabsResolvedMode {
    let settings = TabSettings::as_ref(app);
    match *settings.vertical_tabs_display_granularity.value() {
        VerticalTabsDisplayGranularity::Panes => VerticalTabsResolvedMode::Panes,
        VerticalTabsDisplayGranularity::Tabs => match *settings.vertical_tabs_tab_item_mode.value()
        {
            VerticalTabsTabItemMode::FocusedSession => VerticalTabsResolvedMode::FocusedSession,
            VerticalTabsTabItemMode::Summary => {
                if FeatureFlag::VerticalTabsSummaryMode.is_enabled() {
                    VerticalTabsResolvedMode::Summary
                } else {
                    VerticalTabsResolvedMode::FocusedSession
                }
            }
        },
    }
}

fn push_normalized_unique_summary_text(
    values: &mut Vec<String>,
    seen: &mut HashMap<String, ()>,
    text: &str,
) {
    let Some(normalized) = normalize_summary_text(text) else {
        return;
    };
    if seen.contains_key(&normalized) {
        return;
    }
    seen.insert(normalized.clone(), ());
    values.push(normalized);
}

/// Push a primary label, preserving the first-seen display text and conversation status
/// when the same normalized label is contributed by multiple panes.
fn push_normalized_unique_summary_label(
    values: &mut Vec<VerticalTabsSummaryPrimaryLabel>,
    seen: &mut HashMap<String, ()>,
    text: &str,
    status: Option<ConversationStatus>,
) {
    let Some(normalized) = normalize_summary_text(text) else {
        return;
    };
    if seen.contains_key(&normalized) {
        return;
    }
    seen.insert(normalized.clone(), ());
    values.push(VerticalTabsSummaryPrimaryLabel {
        text: normalized,
        status,
    });
}

/// Stable sort that moves labels with a known `ConversationStatus` ahead of labels without
/// one, while preserving the relative first-seen order within each group. Used in Summary
/// mode so the visible 3-line title region (and the `+ N more` overflow) prioritizes
/// conversation lines over plain terminal / non-conversation lines.
fn sort_summary_primary_labels_status_first(values: &mut [VerticalTabsSummaryPrimaryLabel]) {
    values.sort_by_key(|label| label.status.is_none());
}

fn normalize_summary_text(text: &str) -> Option<String> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

/// Returns the conversation status for a terminal pane, used to render the per-line status
/// pill prefix in Summary mode. Mirrors the status sources used by `render_detail_status_pill`
/// in the detail sidecar — CLI agent sessions with rich status, Oz agent conversations, or
/// ambient agent sessions. Returns `None` for plain terminals or conversations without status.
fn summary_conversation_status_for_terminal(
    terminal_view: &TerminalView,
    app: &AppContext,
) -> Option<ConversationStatus> {
    let cli_agent_session = CLIAgentSessionsModel::as_ref(app).session(terminal_view.id());
    if let Some(session) = cli_agent_session
        .filter(|s| s.listener.is_some())
        .filter(|s| !matches!(s.agent, CLIAgent::Unknown))
        .filter(|s| agent_supports_rich_status(&s.agent))
    {
        return Some(session.status.to_conversation_status());
    }

    let is_ambient = terminal_view.is_ambient_agent_session(app);
    let has_conversation = terminal_view
        .selected_conversation_display_title(app)
        .is_some();
    (has_conversation || is_ambient)
        .then(|| terminal_view.selected_conversation_status_for_display(app))
        .flatten()
}

fn coalesce_summary_branch_entries(
    entries: Vec<VerticalTabsSummaryBranchEntry>,
) -> Vec<VerticalTabsSummaryBranchEntry> {
    let mut coalesced: Vec<VerticalTabsSummaryBranchEntry> = Vec::new();
    let mut indices: HashMap<(PathBuf, String), usize> = HashMap::new();
    for entry in entries {
        let key = (entry.repo_path.clone(), entry.branch_name.clone());
        if let Some(index) = indices.get(&key).copied() {
            let existing = &mut coalesced[index];
            if existing.diff_stats.is_none() {
                existing.diff_stats = entry.diff_stats;
            }
            if existing.pull_request_label.is_none() {
                existing.pull_request_label = entry.pull_request_label;
            }
        } else {
            indices.insert(key, coalesced.len());
            coalesced.push(entry);
        }
    }
    coalesced
}

fn summary_overflow_count(total_count: usize, visible_limit: usize) -> usize {
    total_count.saturating_sub(visible_limit)
}

fn summary_search_text_fragments(
    summary: &VerticalTabsSummaryData,
    title_override: Option<&str>,
) -> Vec<String> {
    let mut fragments = Vec::new();
    if let Some(title_override) = title_override.and_then(normalize_summary_text) {
        fragments.push(title_override);
    }
    fragments.extend(
        summary
            .primary_labels
            .iter()
            .map(|label| label.text.clone()),
    );
    fragments.extend(summary.working_directories.iter().cloned());
    for entry in &summary.branch_entries {
        fragments.push(entry.branch_name.clone());
        if let Some(pull_request_label) = &entry.pull_request_label {
            fragments.push(pull_request_label.clone());
        }
        if let Some(diff_stats) = &entry.diff_stats {
            fragments.push(vtab_diff_stats_text(diff_stats));
        }
    }
    fragments
}

fn select_summary_pane_kind_icons(
    pane_kinds: impl IntoIterator<Item = (EntityId, SummaryPaneKind)>,
) -> Option<SummaryPaneKindIcons> {
    let mut pane_kinds: Vec<(EntityId, SummaryPaneKind)> = pane_kinds.into_iter().collect();
    pane_kinds.sort_by_key(|(creation_order_id, _)| *creation_order_id);

    let mut unique_kinds = Vec::new();
    for (_, pane_kind) in pane_kinds {
        if !unique_kinds.contains(&pane_kind) {
            unique_kinds.push(pane_kind);
        }
        if unique_kinds.len() == 2 {
            return Some(SummaryPaneKindIcons::Pair {
                primary: unique_kinds[0].clone(),
                secondary: unique_kinds[1].clone(),
            });
        }
    }

    unique_kinds
        .first()
        .cloned()
        .map(SummaryPaneKindIcons::Single)
}

fn resolve_summary_pane_kind_icons(
    pane_group: &PaneGroup,
    visible_pane_ids: &[PaneId],
    app: &AppContext,
) -> Option<SummaryPaneKindIcons> {
    select_summary_pane_kind_icons(visible_pane_ids.iter().filter_map(|pane_id| {
        pane_group.pane_by_id(*pane_id).map(|pane| {
            let pane_configuration = pane.pane_configuration();
            let pane_configuration = pane_configuration.as_ref(app);
            let typed = pane_group.resolve_pane_type(*pane_id, app);
            (
                pane_id.creation_order_id(),
                typed.summary_pane_kind(pane_configuration.title().trim(), app),
            )
        })
    }))
}

impl VerticalTabsPanelState {
    pub(super) fn scroll_to_tab(&self, tab_index: usize) {
        self.scroll_state.scroll_to_position(ScrollTarget {
            position_id: tab_position_id(tab_index),
            mode: ScrollToPositionMode::FullyIntoView,
        });
    }

    /// Returns the indices (in original order) of tab groups that have at least
    /// one pane matching the current search query. Returns all indices when the
    /// query is empty.
    pub(super) fn matching_tab_indices(
        &self,
        tabs: &[TabData],
        active_tab_index: usize,
        app: &AppContext,
    ) -> Vec<usize> {
        if self.search_query.is_empty() {
            return (0..tabs.len()).collect();
        }
        let query_lower = self.search_query.to_lowercase();
        let resolved_mode = resolve_vertical_tabs_mode(app);
        let display_granularity = match resolved_mode {
            VerticalTabsResolvedMode::Panes => VerticalTabsDisplayGranularity::Panes,
            VerticalTabsResolvedMode::FocusedSession | VerticalTabsResolvedMode::Summary => {
                VerticalTabsDisplayGranularity::Tabs
            }
        };
        tabs.iter()
            .enumerate()
            .filter(|(tab_index, tab)| {
                let pane_group = tab.pane_group.as_ref(app);
                let visible_pane_ids = pane_group.visible_pane_ids();
                match resolved_mode {
                    VerticalTabsResolvedMode::Summary => {
                        let summary =
                            build_vertical_tabs_summary_data(pane_group, &visible_pane_ids, app);
                        search_fragments_contain_query(
                            &summary_search_text_fragments(
                                &summary,
                                pane_group.custom_title(app).as_deref(),
                            ),
                            &query_lower,
                        )
                    }
                    VerticalTabsResolvedMode::Panes | VerticalTabsResolvedMode::FocusedSession => {
                        pane_ids_for_display_granularity(
                            &visible_pane_ids,
                            pane_group.focused_pane_id(app),
                            display_granularity,
                        )
                        .into_iter()
                        .any(|pane_id| {
                            let title_override = (!uses_outer_group_container(display_granularity))
                                .then(|| pane_group.custom_title(app))
                                .flatten();
                            let ms = MouseStateHandle::default();
                            PaneProps::new(
                                pane_group,
                                pane_id,
                                tab.pane_group.id(),
                                *tab_index == active_tab_index,
                                PaneRowState {
                                    mouse_state: ms,
                                    title_mouse_state: None,
                                    pane_color: None,
                                    badge_mouse_states: PaneRowBadgeMouseStates::default(),
                                },
                                self.detail_hover_state(tab.pane_group.window_id(app)),
                                display_granularity,
                                true,
                                title_override.clone(),
                                None,
                                None,
                                false,
                                None,
                                false,
                                None,
                                app,
                            )
                            .is_some_and(|props| pane_matches_query(&props, &query_lower, app))
                        })
                    }
                }
            })
            .map(|(i, _)| i)
            .collect()
    }
}

const CONTROL_BAR_VERTICAL_PADDING: f32 = 4.;
const CONTROL_BAR_SPACING: f32 = 4.;
const SEARCH_ICON_SIZE: f32 = 12.;
const SEARCH_BAR_HEIGHT: f32 = 24.;
const CONTROL_BAR_BUTTON_RADIUS: Radius = Radius::Pixels(4.);
const SPLIT_BUTTON_HEIGHT: f32 = SEARCH_BAR_HEIGHT;
pub(super) const VERTICAL_TABS_ADD_TAB_POSITION_ID: &str = "vertical_tabs_add_tab_button";
pub(super) const VERTICAL_TABS_SETTINGS_BUTTON_POSITION_ID: &str = "vertical_tabs_settings_button";

pub(super) fn vtab_action_buttons_position_id(tab_index: usize) -> String {
    format!("vtab_action_buttons_{tab_index}")
}
const COMPACT_ICON_SIZE: f32 = 16.;
const GROUP_INSERTION_TARGET_HEIGHT: f32 = 6.;
const GROUP_INSERTION_INDICATOR_HEIGHT: f32 = 3.;

fn any_workspace_pane_being_dragged(workspace: &Workspace, app: &AppContext) -> bool {
    workspace
        .tabs
        .iter()
        .any(|tab| tab.pane_group.as_ref(app).any_pane_being_dragged(app))
}

/// Renders an empty insertion slot for a cross-window ghost drag in the
/// vertical tabs panel. Shows a plain `fg_overlay_1` rectangle the same
/// height as a real tab row — the floating chip at the cursor carries all
/// visual content.
fn render_ghost_vertical_tab_slot(workspace: &Workspace, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    // Match the height of a real tab group row from the last rendered frame.
    // Falls back to 40px if no frame data is available yet.
    let height = workspace
        .tabs
        .first()
        .and_then(|_| {
            app.element_position_by_id_at_last_frame(workspace.window_id, tab_position_id(0))
        })
        .map(|rect| rect.height())
        .unwrap_or(40.);
    ConstrainedBox::new(
        Container::new(Empty::new().finish())
            .with_background(internal_colors::fg_overlay_1(theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_CORNER_RADIUS)))
            .finish(),
    )
    .with_height(height)
    .finish()
}

fn vertical_tabs_tab_bar_location(insert_index: usize, tab_count: usize) -> TabBarLocation {
    if insert_index == tab_count {
        TabBarLocation::AfterTabIndex(tab_count)
    } else {
        TabBarLocation::TabIndex(insert_index)
    }
}

fn render_vertical_tab_hover_indicator(theme: &WarpTheme) -> Box<dyn Element> {
    ConstrainedBox::new(
        Container::new(Empty::new().finish())
            .with_background(ThemeFill::Solid(theme.accent().into()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                GROUP_INSERTION_INDICATOR_HEIGHT / 2.,
            )))
            .finish(),
    )
    .with_height(GROUP_INSERTION_INDICATOR_HEIGHT)
    .finish()
}

fn render_vertical_tab_insertion_target_content(content: Box<dyn Element>) -> Box<dyn Element> {
    ConstrainedBox::new(
        Container::new(content)
            .with_padding(
                Padding::uniform(0.)
                    .with_left(GROUP_HORIZONTAL_PADDING)
                    .with_right(GROUP_HORIZONTAL_PADDING),
            )
            .finish(),
    )
    .with_height(GROUP_INSERTION_TARGET_HEIGHT)
    .finish()
}

fn render_vertical_tab_insertion_target(
    insert_index: usize,
    tab_count: usize,
    is_drag_target: bool,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    let content = if is_drag_target {
        render_vertical_tab_hover_indicator(theme)
    } else {
        Empty::new().finish()
    };

    DropTarget::new(
        render_vertical_tab_insertion_target_content(content),
        VerticalTabsPaneDropTargetData {
            tab_bar_location: vertical_tabs_tab_bar_location(insert_index, tab_count),
            tab_hover_index: TabBarHoverIndex::BeforeTab(insert_index),
        },
    )
    .finish()
}

fn add_vertical_tab_insertion_target_overlay(
    stack: &mut Stack,
    insert_index: usize,
    tab_count: usize,
    is_drag_target: bool,
    parent_anchor: ParentAnchor,
    child_anchor: ChildAnchor,
    theme: &WarpTheme,
) {
    stack.add_positioned_overlay_child(
        render_vertical_tab_insertion_target(insert_index, tab_count, is_drag_target, theme),
        OffsetPositioning::offset_from_parent(
            vec2f(0., 0.),
            ParentOffsetBounds::ParentBySize,
            parent_anchor,
            child_anchor,
        ),
    );
}

fn render_control_bar(
    state: &VerticalTabsPanelState,
    workspace: &Workspace,
    search_editor: &ViewHandle<EditorView>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let sub_text = theme.sub_text_color(theme.background());

    let search_icon = ConstrainedBox::new(WarpIcon::Search.to_warpui_icon(sub_text).finish())
        .with_width(SEARCH_ICON_SIZE)
        .with_height(SEARCH_ICON_SIZE)
        .finish();

    let text_input = TextInput::new(
        search_editor.clone(),
        UiComponentStyles::default()
            .set_background(ElementFill::None)
            .set_border_radius(CornerRadius::with_all(Radius::Pixels(0.)))
            .set_border_width(0.),
    )
    .build()
    .finish();

    let search_bar = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(6.)
        .with_child(search_icon)
        .with_child(Shrinkable::new(1., text_input).finish())
        .finish();

    let settings_button = render_settings_button(state, appearance);
    let new_tab_button = render_new_tab_button(state, workspace, appearance, app);

    Container::new(
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(CONTROL_BAR_SPACING)
            .with_child(Shrinkable::new(1., search_bar).finish())
            .with_child(settings_button)
            .with_child(new_tab_button)
            .finish(),
    )
    .with_padding(
        Padding::uniform(CONTROL_BAR_VERTICAL_PADDING)
            .with_left(GROUP_HORIZONTAL_PADDING)
            .with_right(GROUP_HORIZONTAL_PADDING),
    )
    .finish()
}

fn render_detail_kind_badge_icon(
    props: &PaneProps<'_>,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let sub_text = theme.sub_text_color(theme.background());
    let disabled_text = detail_sidecar_text_colors(theme).disabled;
    match &props.typed {
        TypedPane::Terminal(terminal_pane) => {
            let terminal_view = terminal_pane.terminal_view(app);
            let terminal_view = terminal_view.as_ref(app);
            let cli_agent_session = CLIAgentSessionsModel::as_ref(app).session(terminal_view.id());
            if let Some(icon) = cli_agent_session.and_then(|session| session.agent.icon()) {
                let color = cli_agent_session
                    .and_then(|session| session.agent.brand_color())
                    .map(WarpThemeFill::Solid)
                    .unwrap_or_else(|| theme.accent());
                return icon.to_warpui_icon(color).finish();
            }

            let icon = if terminal_view.is_ambient_agent_session(app) {
                WarpIcon::OzCloud
            } else if terminal_view
                .selected_conversation_display_title(app)
                .is_some()
            {
                WarpIcon::Oz
            } else {
                WarpIcon::Terminal
            };
            let color = match icon {
                WarpIcon::Oz | WarpIcon::OzCloud => oz_icon_fill(theme),
                WarpIcon::Terminal => disabled_text,
                _ => sub_text,
            };
            icon.to_warpui_icon(color).finish()
        }
        TypedPane::Code(_) => icon_from_file_path(&props.title, appearance)
            .unwrap_or_else(|| WarpIcon::Code2.to_warpui_icon(sub_text).finish()),
        typed => {
            let fill = typed
                .warp_drive_object_type()
                .map(|object_type| {
                    WarpThemeFill::Solid(warp_drive_icon_color(appearance, object_type))
                })
                .unwrap_or(sub_text);
            typed.icon().to_warpui_icon(fill).finish()
        }
    }
}

fn render_settings_button(
    state: &VerticalTabsPanelState,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let sub_text = theme.sub_text_color(theme.background());
    let main_text = theme.main_text_color(theme.background());
    let is_popup_open = state.show_settings_popup;
    let ui_builder = appearance.ui_builder().clone();

    let button = Hoverable::new(
        state.settings_button_mouse_state.clone(),
        move |hover_state| {
            let icon = ConstrainedBox::new(
                WarpIcon::Settings
                    .to_warpui_icon(if is_popup_open { main_text } else { sub_text })
                    .finish(),
            )
            .with_width(16.)
            .with_height(16.)
            .finish();

            let background = if is_popup_open {
                internal_colors::fg_overlay_3(theme)
            } else if hover_state.is_hovered() {
                internal_colors::fg_overlay_2(theme)
            } else {
                ThemeFill::Solid(ColorU::transparent_black())
            };

            let button_container = Container::new(icon)
                .with_padding(Padding::uniform(2.))
                .with_background(background)
                .with_corner_radius(CornerRadius::with_all(CONTROL_BAR_BUTTON_RADIUS))
                .finish();

            if hover_state.is_hovered() && !is_popup_open {
                let tooltip = ui_builder
                    .tool_tip("View options".to_string())
                    .build()
                    .finish();
                let mut stack = Stack::new().with_child(button_container);
                stack.add_positioned_overlay_child(
                    tooltip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., 4.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::BottomMiddle,
                        ChildAnchor::TopMiddle,
                    ),
                );
                stack.finish()
            } else {
                button_container
            }
        },
    )
    .on_click(|ctx, _, _| {
        ctx.dispatch_typed_action(WorkspaceAction::ToggleVerticalTabsSettingsPopup);
    })
    .with_cursor(Cursor::PointingHand)
    .finish();

    SavePosition::new(button, VERTICAL_TABS_SETTINGS_BUTTON_POSITION_ID).finish()
}

fn render_new_tab_button(
    state: &VerticalTabsPanelState,
    workspace: &Workspace,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let sub_text = theme.sub_text_color(theme.background());
    let main_text = theme.main_text_color(theme.background());
    let ui_builder = appearance.ui_builder().clone();
    let tab_configs_keybinding =
        keybinding_name_to_display_string(super::TOGGLE_TAB_CONFIGS_MENU_BINDING_NAME, app);
    let is_active = workspace.show_new_session_dropdown_menu.is_some()
        || workspace
            .hoa_onboarding_flow
            .as_ref()
            .is_some_and(|flow| flow.as_ref(app).step() == HoaOnboardingStep::TabConfig);

    Hoverable::new(state.new_tab_hover_state.clone(), move |hover_state| {
        let plus_button = combo_inner_button(
            appearance,
            UiIcon::Plus,
            is_active,
            state.new_tab_button_state.clone(),
        )
        .with_style(
            UiComponentStyles::default()
                .set_border_radius(CornerRadius::with_all(CONTROL_BAR_BUTTON_RADIUS))
                .set_font_color(if is_active { main_text } else { sub_text }.into()),
        )
        .with_active_styles(
            UiComponentStyles::default()
                .set_background(internal_colors::fg_overlay_3(theme).into()),
        )
        .build()
        .on_click(|ctx, _, position| {
            ctx.dispatch_typed_action(WorkspaceAction::ToggleNewSessionMenu { position });
        })
        .finish();

        let button = SavePosition::new(plus_button, VERTICAL_TABS_ADD_TAB_POSITION_ID).finish();

        let contents = if hover_state.is_hovered() {
            let tooltip = if let Some(sublabel) = tab_configs_keybinding.clone() {
                ui_builder
                    .tool_tip_with_sublabel("Tab configs".to_string(), sublabel)
                    .build()
                    .finish()
            } else {
                ui_builder
                    .tool_tip("Tab configs".to_string())
                    .build()
                    .finish()
            };
            let mut stack = Stack::new().with_child(button);
            stack.add_positioned_overlay_child(
                tooltip,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 4.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomMiddle,
                    ChildAnchor::TopMiddle,
                ),
            );
            stack.finish()
        } else {
            button
        };

        let mut container = Container::new(
            ConstrainedBox::new(contents)
                .with_height(SPLIT_BUTTON_HEIGHT)
                .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(CONTROL_BAR_BUTTON_RADIUS));

        if is_active {
            container = container.with_background(internal_colors::fg_overlay_3(theme));
        } else if hover_state.is_hovered() {
            container = container.with_background(internal_colors::neutral_1(theme));
        }
        container.finish()
    })
    .finish()
}

fn render_vertical_tabs_panel(
    state: &VerticalTabsPanelState,
    workspace: &Workspace,
    side: super::PanelPosition,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let scrollable_groups = ClippedScrollable::vertical(
        state.scroll_state.clone(),
        render_groups(state, workspace, app),
        ScrollbarWidth::Custom(4.),
        theme.nonactive_ui_detail().into(),
        theme.active_ui_detail().into(),
        ElementFill::None,
    )
    .with_overlayed_scrollbar()
    .finish();

    let panel_content = Flex::column()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(render_control_bar(
            state,
            workspace,
            &workspace.vertical_tabs_search_input,
            app,
        ))
        .with_child(Shrinkable::new(1., scrollable_groups).finish())
        .finish();

    // The settings popup is rendered at the workspace level (with Dismiss for click-outside-
    // to-close). Rendering it here again shares MouseStateHandle instances across two Hoverable
    // trees; click_count.take() is consumed by this copy first, leaving the workspace copy
    // with None and silently dropping all clicks on the popup items.
    let panel_with_popup: Box<dyn Element> = panel_content;

    let drag_side = match side {
        super::PanelPosition::Left => DragBarSide::Right,
        super::PanelPosition::Right => DragBarSide::Left,
    };
    let inner = Container::new(panel_with_popup)
        .with_background(internal_colors::fg_overlay_1(theme))
        .finish();

    Resizable::new(state.resizable_state.clone(), inner)
        .with_dragbar_side(drag_side)
        .on_resize(|ctx, _| {
            ctx.notify();
        })
        .with_bounds_callback(Box::new(|window_size| {
            let max_width = window_size.x() * MAX_PANEL_WIDTH_RATIO;
            (MIN_PANEL_WIDTH, max_width.max(MIN_PANEL_WIDTH))
        }))
        .finish()
}

fn render_groups(
    state: &VerticalTabsPanelState,
    workspace: &Workspace,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    if workspace.tabs.is_empty() {
        return Container::new(
            Text::new_inline("No tabs open", appearance.ui_font_family(), 12.)
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
        )
        .with_padding(Padding::uniform(12.))
        .finish();
    }

    let resolved_mode = resolve_vertical_tabs_mode(app);
    let display_granularity = match resolved_mode {
        VerticalTabsResolvedMode::Panes => VerticalTabsDisplayGranularity::Panes,
        VerticalTabsResolvedMode::FocusedSession | VerticalTabsResolvedMode::Summary => {
            VerticalTabsDisplayGranularity::Tabs
        }
    };
    let uses_outer_group_container = uses_outer_group_container(display_granularity);
    let query = state.search_query.as_str();
    let visible_tabs: Vec<(usize, Option<Vec<PaneId>>)> = if query.is_empty() {
        workspace
            .tabs
            .iter()
            .enumerate()
            .map(|(tab_index, _)| (tab_index, None))
            .collect()
    } else {
        let query_lower = query.to_lowercase();
        workspace
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(tab_index, tab)| {
                let pane_group = tab.pane_group.as_ref(app);
                let visible_pane_ids = pane_group.visible_pane_ids();
                match resolved_mode {
                    VerticalTabsResolvedMode::Summary => {
                        let summary =
                            build_vertical_tabs_summary_data(pane_group, &visible_pane_ids, app);
                        search_fragments_contain_query(
                            &summary_search_text_fragments(
                                &summary,
                                pane_group.custom_title(app).as_deref(),
                            ),
                            &query_lower,
                        )
                        .then_some((tab_index, None))
                    }
                    VerticalTabsResolvedMode::Panes | VerticalTabsResolvedMode::FocusedSession => {
                        let title_override = (!uses_outer_group_container)
                            .then(|| pane_group.custom_title(app))
                            .flatten();
                        let matching_ids: Vec<PaneId> = pane_ids_for_display_granularity(
                            &visible_pane_ids,
                            pane_group.focused_pane_id(app),
                            display_granularity,
                        )
                        .into_iter()
                        .filter(|&pane_id| {
                            let Some(mouse_state) =
                                state.pane_row_mouse_states.borrow().get(&pane_id).cloned()
                            else {
                                let ms = MouseStateHandle::default();
                                return PaneProps::new(
                                    pane_group,
                                    pane_id,
                                    tab.pane_group.id(),
                                    tab_index == workspace.active_tab_index,
                                    PaneRowState {
                                        mouse_state: ms,
                                        title_mouse_state: None,
                                        pane_color: None,
                                        badge_mouse_states: PaneRowBadgeMouseStates::default(),
                                    },
                                    state.detail_hover_state(workspace.window_id),
                                    display_granularity,
                                    true,
                                    title_override.clone(),
                                    None,
                                    None,
                                    false,
                                    None,
                                    false,
                                    None,
                                    app,
                                )
                                .is_some_and(|props| {
                                    pane_matches_query(&props, &query_lower, app)
                                });
                            };
                            PaneProps::new(
                                pane_group,
                                pane_id,
                                tab.pane_group.id(),
                                tab_index == workspace.active_tab_index,
                                PaneRowState {
                                    mouse_state,
                                    title_mouse_state: None,
                                    pane_color: None,
                                    badge_mouse_states: PaneRowBadgeMouseStates::default(),
                                },
                                state.detail_hover_state(workspace.window_id),
                                display_granularity,
                                true,
                                title_override.clone(),
                                None,
                                None,
                                false,
                                None,
                                false,
                                None,
                                app,
                            )
                            .is_some_and(|props| pane_matches_query(&props, &query_lower, app))
                        })
                        .collect();

                        (!matching_ids.is_empty()).then_some((tab_index, Some(matching_ids)))
                    }
                }
            })
            .collect()
    };

    if visible_tabs.is_empty() {
        if query.is_empty() {
            return Empty::new().finish();
        } else {
            return Container::new(
                Text::new_inline(
                    "No tabs match your search.",
                    appearance.ui_font_family(),
                    12.,
                )
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            )
            .with_padding(Padding::uniform(12.))
            .finish();
        }
    }

    let is_any_pane_dragging = any_workspace_pane_being_dragged(workspace, app);
    // Ghost state for cross-window drag hovering over this window's vertical tabs panel.
    let ghost_state = CrossWindowTabDrag::as_ref(app).ghost_state_for_window(workspace.window_id);
    let ghost_insertion_index = ghost_state.as_ref().map(|g| g.insertion_index);
    let mut groups = Flex::column()
        .with_main_axis_size(MainAxisSize::Min)
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    if !uses_outer_group_container {
        groups = groups.with_spacing(TABS_MODE_ITEM_SPACING);
    }

    for (visible_tab_index, (tab_index, filtered_pane_ids)) in visible_tabs.iter().enumerate() {
        // Insert ghost slot before this tab group if the drop would land here.
        if ghost_insertion_index == Some(*tab_index) {
            groups.add_child(render_ghost_vertical_tab_slot(workspace, app));
        }
        let insert_before_index = *tab_index;
        let insert_after_index =
            (visible_tab_index == visible_tabs.len() - 1).then_some(tab_index + 1);
        groups.add_child(render_tab_group(
            state,
            workspace,
            *tab_index,
            &workspace.tabs[*tab_index],
            filtered_pane_ids.as_deref(),
            TabGroupDragState {
                is_any_pane_dragging,
                insert_before_index,
                insert_after_index,
            },
            app,
        ));
    }
    // Ghost after all tab groups (fencepost).
    if ghost_insertion_index == Some(workspace.tabs.len()) {
        groups.add_child(render_ghost_vertical_tab_slot(workspace, app));
    }

    // Prune stale badge mouse states for panes that no longer exist.
    let all_pane_ids: std::collections::HashSet<PaneId> = workspace
        .tabs
        .iter()
        .flat_map(|tab| tab.pane_group.as_ref(app).visible_pane_ids())
        .collect();
    state
        .pane_badge_mouse_states
        .borrow_mut()
        .retain(|id, _| all_pane_ids.contains(id));
    state
        .pane_title_mouse_states
        .borrow_mut()
        .retain(|id, _| all_pane_ids.contains(id));
    state
        .detail_pane_badge_mouse_states
        .borrow_mut()
        .retain(|id, _| all_pane_ids.contains(id));

    let groups = groups.finish();
    if uses_outer_group_container {
        groups
    } else {
        Container::new(groups)
            .with_padding(Padding::uniform(8.).with_top(0.))
            .finish()
    }
}

fn render_tab_group(
    state: &VerticalTabsPanelState,
    workspace: &Workspace,
    tab_index: usize,
    tab: &TabData,
    filtered_pane_ids: Option<&[PaneId]>,
    drag_state: TabGroupDragState,
    app: &AppContext,
) -> Box<dyn Element> {
    render_tab_group_internal(
        state,
        workspace,
        tab_index,
        tab,
        filtered_pane_ids,
        drag_state,
        false,
        app,
    )
}

#[allow(clippy::too_many_arguments)]
fn render_tab_group_internal(
    state: &VerticalTabsPanelState,
    workspace: &Workspace,
    tab_index: usize,
    tab: &TabData,
    filtered_pane_ids: Option<&[PaneId]>,
    drag_state: TabGroupDragState,
    for_drag_ghost: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let pane_group = tab.pane_group.as_ref(app);
    let pane_group_id = tab.pane_group.id();
    let visible_pane_ids = pane_group.visible_pane_ids();
    let resolved_mode = resolve_vertical_tabs_mode(app);
    let display_granularity = match resolved_mode {
        VerticalTabsResolvedMode::Panes => VerticalTabsDisplayGranularity::Panes,
        VerticalTabsResolvedMode::FocusedSession | VerticalTabsResolvedMode::Summary => {
            VerticalTabsDisplayGranularity::Tabs
        }
    };
    let uses_outer_group_container = uses_outer_group_container(display_granularity);
    let representative_pane_ids = pane_ids_for_display_granularity(
        &visible_pane_ids,
        pane_group.focused_pane_id(app),
        display_granularity,
    );
    let pane_ids_to_render: &[PaneId] = filtered_pane_ids.unwrap_or(&representative_pane_ids);
    let PaneGroupStateHandles {
        group: group_mouse_state,
        header: group_header_mouse_state,
        kebab: kebab_mouse_state,
        close: close_mouse_state,
        action_buttons: action_buttons_mouse_state,
    } = state
        .group_mouse_states
        .borrow_mut()
        .entry(pane_group_id)
        .or_default()
        .clone();
    let action_buttons_mouse_over = action_buttons_mouse_state
        .lock()
        .expect("action buttons hover state lock poisoned")
        .is_mouse_over_element();
    let row_mouse_states: Vec<(PaneId, MouseStateHandle)> = pane_ids_to_render
        .iter()
        .map(|pane_id| {
            let ms = state
                .pane_row_mouse_states
                .borrow_mut()
                .entry(*pane_id)
                .or_default()
                .clone();
            (*pane_id, ms)
        })
        .collect();
    let title_mouse_states: HashMap<PaneId, MouseStateHandle> = pane_ids_to_render
        .iter()
        .map(|pane_id| {
            let ms = state
                .pane_title_mouse_states
                .borrow_mut()
                .entry(*pane_id)
                .or_default()
                .clone();
            (*pane_id, ms)
        })
        .collect();
    let is_active = tab_index == workspace.active_tab_index
        && !workspace
            .current_workspace_state
            .is_agent_management_view_open;
    let has_top_border = tab_index > 0;
    let is_first_tab = tab_index == 0;
    let is_last_tab = tab_index + 1 == workspace.tabs.len();
    let is_this_tab_dragging = tab.draggable_state.is_dragging();
    let color_mode = compute_tab_group_color_mode(tab, pane_group, &visible_pane_ids, theme, app);
    let per_pane_colors = color_mode.into_per_pane_colors(&visible_pane_ids);
    let is_being_renamed = is_active && workspace.current_workspace_state.is_tab_being_renamed();
    let rename_editor = workspace.tab_rename_editor.clone();
    let has_custom_title = pane_group.custom_title(app).is_some();
    let displayed_tab_title_override = (!uses_outer_group_container)
        .then(|| pane_group.custom_title(app))
        .flatten();
    let is_menu_open_for_tab = workspace
        .show_tab_right_click_menu
        .is_some_and(|(idx, _)| idx == tab_index);
    let is_drag_target = workspace.hovered_tab_index == Some(TabBarHoverIndex::OverTab(tab_index));
    let summary = matches!(resolved_mode, VerticalTabsResolvedMode::Summary)
        .then(|| build_vertical_tabs_summary_data(pane_group, &visible_pane_ids, app));
    let summary_pane_kind_icons = matches!(resolved_mode, VerticalTabsResolvedMode::Summary)
        .then(|| resolve_summary_pane_kind_icons(pane_group, &visible_pane_ids, app))
        .flatten();
    let active_pane_context_menu_target = PaneViewLocator {
        pane_group_id,
        pane_id: pane_group.focused_pane_id(app),
    };

    let mut group_element = Hoverable::new(group_mouse_state, move |group_state| {
        let build_rows = || {
            let mut rows = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(GROUP_ITEM_SPACING);
            if matches!(resolved_mode, VerticalTabsResolvedMode::Summary) {
                let Some((pane_id, row_mouse_state)) = row_mouse_states.first() else {
                    return Empty::new().finish();
                };
                let pane_color = per_pane_colors
                    .as_ref()
                    .and_then(|map| map.get(pane_id).copied())
                    .flatten();
                let badge_mouse_states = state
                    .pane_badge_mouse_states
                    .borrow_mut()
                    .entry(*pane_id)
                    .or_default()
                    .clone();
                let Some(pane_props) = PaneProps::new(
                    pane_group,
                    *pane_id,
                    pane_group_id,
                    is_active,
                    PaneRowState {
                        mouse_state: row_mouse_state.clone(),
                        title_mouse_state: None,
                        pane_color,
                        badge_mouse_states,
                    },
                    state.detail_hover_state(workspace.window_id),
                    display_granularity,
                    false,
                    displayed_tab_title_override.clone(),
                    (!uses_outer_group_container).then_some(tab_index),
                    None,
                    !uses_outer_group_container && is_being_renamed,
                    (!uses_outer_group_container).then_some(rename_editor.clone()),
                    false,
                    None,
                    app,
                ) else {
                    return Empty::new().finish();
                };
                rows.add_child(render_summary_tab_item(
                    pane_props,
                    summary
                        .as_ref()
                        .expect("summary data must exist in summary mode"),
                    summary_pane_kind_icons,
                    app,
                ));
                return rows.finish();
            }
            for (pane_id, row_mouse_state) in &row_mouse_states {
                let pane_color = per_pane_colors
                    .as_ref()
                    .and_then(|map| map.get(pane_id).copied())
                    .flatten();
                let badge_mouse_states = state
                    .pane_badge_mouse_states
                    .borrow_mut()
                    .entry(*pane_id)
                    .or_default()
                    .clone();
                let locator = PaneViewLocator {
                    pane_group_id,
                    pane_id: *pane_id,
                };
                let is_pane_being_renamed = workspace
                    .current_workspace_state
                    .is_pane_being_renamed(locator);
                let Some(pane_props) = PaneProps::new(
                    pane_group,
                    *pane_id,
                    pane_group_id,
                    is_active,
                    PaneRowState {
                        mouse_state: row_mouse_state.clone(),
                        title_mouse_state: title_mouse_states.get(pane_id).cloned(),
                        pane_color,
                        badge_mouse_states,
                    },
                    state.detail_hover_state(workspace.window_id),
                    display_granularity,
                    true,
                    displayed_tab_title_override.clone(),
                    (!uses_outer_group_container).then_some(tab_index),
                    uses_outer_group_container.then_some(tab_index),
                    !uses_outer_group_container && is_being_renamed,
                    (!uses_outer_group_container).then_some(rename_editor.clone()),
                    is_pane_being_renamed,
                    is_pane_being_renamed.then_some(workspace.pane_rename_editor.clone()),
                    app,
                ) else {
                    continue;
                };
                let view_mode = *TabSettings::as_ref(app).vertical_tabs_view_mode.value();
                let row = match view_mode {
                    VerticalTabsViewMode::Compact => render_compact_pane_row(pane_props, app),
                    VerticalTabsViewMode::Expanded => render_pane_row(pane_props, app),
                };
                rows.add_child(row);
            }
            rows.finish()
        };

        let group_content = if uses_outer_group_container {
            let mut group = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
            if has_custom_title || is_being_renamed {
                group.add_child(render_group_header(
                    GroupHeaderProps {
                        tab_index,
                        pane_group,
                        is_being_renamed,
                        rename_editor: rename_editor.clone(),
                        header_mouse_state: group_header_mouse_state.clone(),
                    },
                    app,
                ));
            }

            let show_header = has_custom_title || is_being_renamed;
            let mut body_padding = Padding::uniform(0.)
                .with_left(GROUP_HORIZONTAL_PADDING)
                .with_right(GROUP_HORIZONTAL_PADDING)
                .with_bottom(GROUP_BODY_BOTTOM_PADDING);
            if !show_header {
                body_padding = body_padding.with_top(GROUP_BODY_BOTTOM_PADDING);
            }
            group.add_child(
                Container::new(build_rows())
                    .with_padding(body_padding)
                    .finish(),
            );
            let background = if is_drag_target {
                internal_colors::fg_overlay_2(theme)
            } else if is_active || group_state.is_hovered() {
                internal_colors::fg_overlay_1(theme)
            } else {
                ThemeFill::Solid(ColorU::transparent_black())
            };
            let mut container = Container::new(group.finish()).with_background(background);
            if is_drag_target {
                container = container.with_border(
                    Border::all(1.).with_border_fill(ThemeFill::Solid(theme.accent().into())),
                );
            } else if has_top_border || is_first_tab || is_last_tab {
                container = container.with_border(
                    Border::new(1.)
                        .with_sides(has_top_border || is_first_tab, false, is_last_tab, false)
                        .with_border_fill(internal_colors::fg_overlay_1(theme)),
                );
            }
            container.finish()
        } else {
            let background = if is_drag_target {
                internal_colors::fg_overlay_2(theme)
            } else if is_active || group_state.is_hovered() {
                internal_colors::fg_overlay_1(theme)
            } else {
                ThemeFill::Solid(ColorU::transparent_black())
            };
            let mut container = Container::new(build_rows()).with_background(background);
            if is_drag_target {
                container = container.with_border(
                    Border::all(1.).with_border_fill(ThemeFill::Solid(theme.accent().into())),
                );
            }
            container.finish()
        };

        // Show the action buttons when the group OR the buttons themselves
        // are hovered, following the pattern from AgentManagementView.
        // This prevents flickering when the mouse moves from the group
        // to the overlay buttons (which may sit outside the group bounds).
        let should_show_action_buttons = !drag_state.is_any_pane_dragging
            && (group_state.is_hovered() || action_buttons_mouse_over || is_menu_open_for_tab);

        let action_buttons = if should_show_action_buttons {
            render_group_action_buttons(
                tab_index,
                is_menu_open_for_tab,
                action_buttons_mouse_state.clone(),
                kebab_mouse_state.clone(),
                close_mouse_state.clone(),
                theme,
            )
        } else {
            Empty::new().finish()
        };
        let mut stack = Stack::new().with_child(group_content);
        if drag_state.is_any_pane_dragging {
            add_vertical_tab_insertion_target_overlay(
                &mut stack,
                drag_state.insert_before_index,
                workspace.tabs.len(),
                workspace.hovered_tab_index
                    == Some(TabBarHoverIndex::BeforeTab(drag_state.insert_before_index)),
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
                theme,
            );
            if let Some(insert_after_index) = drag_state.insert_after_index {
                add_vertical_tab_insertion_target_overlay(
                    &mut stack,
                    insert_after_index,
                    workspace.tabs.len(),
                    workspace.hovered_tab_index
                        == Some(TabBarHoverIndex::BeforeTab(insert_after_index)),
                    ParentAnchor::BottomLeft,
                    ChildAnchor::BottomLeft,
                    theme,
                );
            }
        }
        stack.add_positioned_overlay_child(
            action_buttons,
            OffsetPositioning::offset_from_parent(
                vec2f(-4., GROUP_HEADER_VERTICAL_PADDING),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::TopRight,
                ChildAnchor::TopRight,
            ),
        );
        stack.finish()
    })
    .on_right_click(move |ctx, _, position| {
        ctx.dispatch_typed_action(WorkspaceAction::ToggleVerticalTabsPaneContextMenu {
            tab_index,
            target: VerticalTabsPaneContextMenuTarget::ActivePane(active_pane_context_menu_target),
            position,
        });
    });

    // Mirror the horizontal-tab behavior: middle-click closes the tab, except when it would
    // close the last tab in a context that doesn't allow closing the window.
    if ContextFlag::CloseWindow.is_enabled() || !is_last_tab {
        group_element = group_element.on_middle_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::CloseTab(tab_index));
        });
    }

    let group_element = group_element.with_defer_events_to_children().finish();

    let draggable = Draggable::new(tab.draggable_state.clone(), group_element)
        .on_drag_start(|ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::StartTabDrag);
        })
        .on_drag(move |ctx, _, rect, _| {
            ctx.dispatch_typed_action(WorkspaceAction::DragTab {
                tab_index,
                tab_position: rect,
            });
        })
        .on_drop(|ctx, _, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::DropTab);
        });
    // Only lock the drag to the vertical axis when cross-window tab drag is
    // disabled. When it is enabled, the user needs to be able to drag
    // horizontally out of the panel to detach the tab into a new window.
    let draggable = if FeatureFlag::DragTabsToWindows.is_enabled() {
        draggable
    } else {
        draggable.with_drag_axis(DragAxis::VerticalOnly)
    };
    let draggable = draggable.finish();

    let draggable: Box<dyn Element> = if is_this_tab_dragging {
        Container::new(draggable)
            .with_background(internal_colors::fg_overlay_1(theme))
            .finish()
    } else {
        draggable
    };
    // When rendering inside the cross-window drag chip overlay, skip the
    // outer `SavePosition` (it would clobber the target window's
    // `tab_position_<index>` cache entry and break
    // `tab_insertion_index_for_cursor`) and the `DropTarget` (the chip
    // shouldn't be a drop target since it follows the cursor).
    if for_drag_ghost {
        return draggable;
    }
    let draggable = SavePosition::new(draggable, &tab_position_id(tab_index)).finish();

    if is_this_tab_dragging {
        draggable
    } else {
        DropTarget::new(
            draggable,
            VerticalTabsPaneDropTargetData {
                tab_bar_location: TabBarLocation::TabIndex(tab_index),
                tab_hover_index: TabBarHoverIndex::OverTab(tab_index),
            },
        )
        .finish()
    }
}

fn render_group_action_buttons(
    tab_index: usize,
    is_menu_open: bool,
    action_buttons_mouse_state: MouseStateHandle,
    kebab_mouse_state: MouseStateHandle,
    close_mouse_state: MouseStateHandle,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    let meta_color = theme.sub_text_color(theme.background());

    let kebab_button = Hoverable::new(kebab_mouse_state, move |button_state| {
        let mut container = Container::new(
            ConstrainedBox::new(WarpIcon::DotsVertical.to_warpui_icon(meta_color).finish())
                .with_width(GROUP_ACTION_BUTTON_ICON_SIZE)
                .with_height(GROUP_ACTION_BUTTON_ICON_SIZE)
                .finish(),
        )
        .with_padding(Padding::uniform(GROUP_ACTION_BUTTON_PADDING))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
        if is_menu_open || button_state.is_hovered() {
            container = container.with_background(internal_colors::fg_overlay_2(theme));
        }
        container.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_mouse_down(move |ctx, _, _| {
        ctx.dispatch_typed_action(WorkspaceAction::ToggleTabRightClickMenu {
            tab_index,
            anchor: TabContextMenuAnchor::VerticalTabsKebab,
        });
    })
    .finish();

    let close_button = Hoverable::new(close_mouse_state, move |button_state| {
        let mut container = Container::new(
            ConstrainedBox::new(WarpIcon::X.to_warpui_icon(meta_color).finish())
                .with_width(GROUP_ACTION_BUTTON_ICON_SIZE)
                .with_height(GROUP_ACTION_BUTTON_ICON_SIZE)
                .finish(),
        )
        .with_padding(Padding::uniform(GROUP_ACTION_BUTTON_PADDING))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
        if button_state.is_hovered() {
            container = container.with_background(internal_colors::fg_overlay_3(theme));
        }
        container.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(WorkspaceAction::CloseTab(tab_index));
    })
    .finish();

    let button_row = Flex::row()
        .with_main_axis_size(MainAxisSize::Min)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(GROUP_ACTION_BUTTON_GAP)
        .with_child(kebab_button)
        .with_child(close_button)
        .finish();

    let belt_border_color = internal_colors::neutral_4(theme);
    let belt = Hoverable::new(action_buttons_mouse_state, move |_| {
        Container::new(button_row)
            .with_background(ThemeFill::Solid(internal_colors::neutral_3(theme)))
            .with_border(Border::all(1.).with_border_fill(ThemeFill::Solid(belt_border_color)))
            .with_padding(Padding::uniform(GROUP_ACTION_BUTTON_PADDING))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .finish()
    })
    .with_defer_events_to_children()
    .finish();

    SavePosition::new(belt, &vtab_action_buttons_position_id(tab_index)).finish()
}

/// Renders the same vertical tab group element used by the live vertical
/// tabs panel, so the floating chip during a cross-window tab drag matches
/// the source vertical-tabs row exactly. Constructed with neutral state
/// (no drag/hover indicators) since the snapshot doesn't represent an
/// in-progress local drag and isn't itself a drop target.
pub(crate) fn render_tab_group_for_drag_ghost(
    workspace: &Workspace,
    tab_index: usize,
    app: &AppContext,
) -> Box<dyn Element> {
    let Some(tab) = workspace.tabs.get(tab_index) else {
        return Empty::new().finish();
    };
    let drag_state = TabGroupDragState {
        is_any_pane_dragging: false,
        insert_before_index: 0,
        insert_after_index: None,
    };
    render_tab_group_internal(
        &workspace.vertical_tabs_panel,
        workspace,
        tab_index,
        tab,
        None,
        drag_state,
        true, // for_drag_ghost
        app,
    )
}

fn render_group_header(props: GroupHeaderProps<'_>, app: &AppContext) -> Box<dyn Element> {
    let GroupHeaderProps {
        tab_index,
        pane_group,
        is_being_renamed,
        rename_editor,
        header_mouse_state,
    } = props;
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let title = pane_group.display_title(app);
    let title = if title.is_empty() {
        "Untitled tab".to_string()
    } else {
        title
    };
    let font_family = appearance.ui_font_family();
    let title_color = theme.sub_text_color(theme.background());

    Hoverable::new(header_mouse_state, move |_header_state| {
        Container::new(if is_being_renamed {
            TextInput::new(
                rename_editor.clone(),
                UiComponentStyles::default()
                    .set_background(ElementFill::None)
                    .set_border_radius(CornerRadius::with_all(Radius::Pixels(0.)))
                    .set_border_width(0.),
            )
            .build()
            .finish()
        } else {
            Text::new_inline(title.clone(), font_family, 10.)
                .with_clip(ClipConfig::ellipsis())
                .with_color(title_color.into())
                .finish()
        })
        .with_padding(
            Padding::uniform(0.)
                .with_left(GROUP_HORIZONTAL_PADDING)
                .with_right(GROUP_HORIZONTAL_PADDING)
                .with_top(GROUP_HEADER_VERTICAL_PADDING)
                .with_bottom(GROUP_HEADER_VERTICAL_PADDING),
        )
        .finish()
    })
    .on_click(move |ctx, _, _| {
        if !is_being_renamed {
            ctx.dispatch_typed_action(WorkspaceAction::ActivateTab(tab_index));
        }
    })
    .on_double_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(WorkspaceAction::RenameTab(tab_index));
    })
    .with_cursor(Cursor::PointingHand)
    .finish()
}

fn render_passive_terminal_diff_stats_badge(
    git_line_changes: &GitLineChanges,
    appearance: &Appearance,
) -> Box<dyn Element> {
    render_badge_container(
        render_vtab_diff_stats_content(git_line_changes, appearance),
        internal_colors::fg_overlay_1(appearance.theme()),
    )
}

fn resolve_icon_with_status_variant(
    typed: &TypedPane<'_>,
    title: &str,
    appearance: &Appearance,
    app: &AppContext,
) -> IconWithStatusVariant {
    let theme = appearance.theme();
    let main_text = theme.main_text_color(theme.background());
    let sub_text = theme.sub_text_color(theme.background());

    let drive_color = |object_type: DriveObjectType| -> WarpThemeFill {
        WarpThemeFill::Solid(warp_drive_icon_color(appearance, object_type))
    };

    match typed {
        TypedPane::Terminal(terminal_pane) => {
            let terminal_view = terminal_pane.terminal_view(app);
            let terminal_view = terminal_view.as_ref(app);
            if let Some(variant) = terminal_view_agent_icon_variant(terminal_view, app) {
                variant
            } else {
                // Plain terminal: use foreground color per design spec
                IconWithStatusVariant::Neutral {
                    icon: WarpIcon::Terminal,
                    icon_color: main_text,
                }
            }
        }
        TypedPane::Code(_) => {
            if let Some(icon_element) = icon_from_file_path(title, appearance) {
                IconWithStatusVariant::NeutralElement { icon_element }
            } else {
                IconWithStatusVariant::Neutral {
                    icon: WarpIcon::Code2,
                    icon_color: sub_text,
                }
            }
        }
        // Settings and environment management use the foreground color per design spec
        TypedPane::Settings | TypedPane::EnvironmentManagement => IconWithStatusVariant::Neutral {
            icon: typed.icon(),
            icon_color: main_text,
        },
        // Warp Drive object types use their established index colors
        TypedPane::Notebook { is_plan } => IconWithStatusVariant::Neutral {
            icon: typed.icon(),
            icon_color: drive_color(DriveObjectType::Notebook {
                is_ai_document: *is_plan,
            }),
        },
        TypedPane::Workflow { is_ai_prompt: true } => IconWithStatusVariant::Neutral {
            icon: typed.icon(),
            icon_color: drive_color(DriveObjectType::AgentModeWorkflow),
        },
        TypedPane::Workflow {
            is_ai_prompt: false,
        } => IconWithStatusVariant::Neutral {
            icon: typed.icon(),
            icon_color: drive_color(DriveObjectType::Workflow),
        },
        TypedPane::EnvVarCollection => IconWithStatusVariant::Neutral {
            icon: typed.icon(),
            icon_color: drive_color(DriveObjectType::EnvVarCollection),
        },
        TypedPane::AIFact => IconWithStatusVariant::Neutral {
            icon: typed.icon(),
            icon_color: drive_color(DriveObjectType::AIFact),
        },
        // Other pane types use sub-text color
        other => IconWithStatusVariant::Neutral {
            icon: other.icon(),
            icon_color: sub_text,
        },
    }
}

fn has_unread_activity(typed: &TypedPane<'_>, app: &AppContext) -> bool {
    let TypedPane::Terminal(terminal_pane) = typed else {
        return false;
    };
    let terminal_view = terminal_pane.terminal_view(app);
    let terminal_view_id = terminal_view.as_ref(app).id();
    AgentNotificationsModel::as_ref(app)
        .notifications()
        .has_unread_for_terminal_view(terminal_view_id)
}

const INDICATOR_DOT_SIZE: f32 = 8.;

fn render_title_indicator(theme: &WarpTheme) -> Box<dyn Element> {
    ConstrainedBox::new(
        WarpIcon::CircleFilled
            .to_warpui_icon(theme.accent())
            .finish(),
    )
    .with_width(INDICATOR_DOT_SIZE)
    .with_height(INDICATOR_DOT_SIZE)
    .finish()
}

fn render_pane_row(props: PaneProps<'_>, app: &AppContext) -> Box<dyn Element> {
    let effective_subtitle = props.subtitle.clone();
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let font_family = appearance.ui_font_family();

    let icon = render_pane_icon_with_status(
        resolve_icon_with_status_variant(&props.typed, &props.title, appearance, app),
        theme,
    );

    // Top-align the icon when there are multiple lines of content so it sits next to
    // the first line; center it for single-line rows (Settings, Notebook with no subtitle, etc.).
    let icon_alignment =
        if matches!(props.typed, TypedPane::Terminal(_)) || !effective_subtitle.is_empty() {
            CrossAxisAlignment::Start
        } else {
            CrossAxisAlignment::Center
        };

    let text_content = if let TypedPane::Terminal(terminal_pane) = &props.typed {
        render_terminal_row_content(
            &props,
            terminal_pane.terminal_view(app).as_ref(app),
            appearance,
            app,
        )
    } else {
        let has_indicator =
            props.typed.badge(app).is_some() || has_unread_activity(&props.typed, app);
        let mut title_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        title_row.add_child(
            Shrinkable::new(
                1.,
                render_pane_title_slot(
                    &props,
                    || {
                        Text::new_inline(props.displayed_title().to_string(), font_family, 12.)
                            .with_clip(ClipConfig::ellipsis())
                            .with_color(theme.main_text_color(theme.background()).into())
                            .finish()
                    },
                    12.,
                    theme.main_text_color(theme.background()),
                    ClipConfig::ellipsis(),
                    appearance,
                    app,
                ),
            )
            .finish(),
        );
        if has_indicator {
            title_row.add_child(
                Container::new(render_title_indicator(theme))
                    .with_margin_left(4.)
                    .finish(),
            );
        }

        let mut content_col = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(2.)
            .with_child(title_row.finish());

        if !effective_subtitle.is_empty() {
            let subtitle_clip = if matches!(props.typed, TypedPane::Code(_)) {
                ClipConfig::start()
            } else {
                ClipConfig::ellipsis()
            };
            content_col.add_child(
                Text::new_inline(effective_subtitle, font_family, 12.)
                    .with_clip(subtitle_clip)
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
            );
        }

        content_col.finish()
    };

    let content = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(icon_alignment)
        .with_spacing(ICON_WITH_STATUS_GAP)
        .with_child(icon)
        .with_child(Shrinkable::new(1., text_content).finish())
        .finish();

    render_pane_row_element(props, Padding::uniform(8.), true, content, theme)
}

enum TypedPane<'a> {
    Terminal(&'a TerminalPane),
    Code(&'a CodePane),
    CodeDiff,
    File,
    Notebook { is_plan: bool },
    Workflow { is_ai_prompt: bool },
    Settings,
    EnvVarCollection,
    EnvironmentManagement,
    AIFact,
    AIDocument,
    ExecutionProfileEditor,
    Other,
}

impl TypedPane<'_> {
    fn summary_pane_kind(&self, title: &str, app: &AppContext) -> SummaryPaneKind {
        match self {
            TypedPane::Terminal(terminal_pane) => {
                let terminal_view = terminal_pane.terminal_view(app);
                let terminal_view = terminal_view.as_ref(app);
                // Route through the shared helper so summary mode agrees with
                // `resolve_icon_with_status_variant` on what the tab represents.
                match terminal_view_agent_icon_variant(terminal_view, app) {
                    Some(IconWithStatusVariant::OzAgent { is_ambient, .. }) => {
                        SummaryPaneKind::OzAgent { is_ambient }
                    }
                    Some(IconWithStatusVariant::CLIAgent {
                        agent, is_ambient, ..
                    }) => SummaryPaneKind::CLIAgent { agent, is_ambient },
                    Some(_) | None => SummaryPaneKind::Terminal,
                }
            }
            TypedPane::Code(_) => SummaryPaneKind::Code {
                title: title.to_string(),
            },
            TypedPane::CodeDiff => SummaryPaneKind::CodeDiff,
            TypedPane::File => SummaryPaneKind::File,
            TypedPane::Notebook { is_plan } => SummaryPaneKind::Notebook { is_plan: *is_plan },
            TypedPane::Workflow { is_ai_prompt } => SummaryPaneKind::Workflow {
                is_ai_prompt: *is_ai_prompt,
            },
            TypedPane::Settings => SummaryPaneKind::Settings,
            TypedPane::EnvVarCollection => SummaryPaneKind::EnvVarCollection,
            TypedPane::EnvironmentManagement => SummaryPaneKind::EnvironmentManagement,
            TypedPane::AIFact => SummaryPaneKind::AIFact,
            TypedPane::AIDocument => SummaryPaneKind::AIDocument,
            TypedPane::ExecutionProfileEditor => SummaryPaneKind::ExecutionProfileEditor,
            TypedPane::Other => SummaryPaneKind::Other,
        }
    }

    fn warp_drive_object_type(&self) -> Option<DriveObjectType> {
        typed_pane_warp_drive_object_type(self)
    }

    fn supports_vertical_tabs_detail_sidecar(&self) -> bool {
        matches!(self, TypedPane::Terminal(_) | TypedPane::Code(_))
            || self.warp_drive_object_type().is_some()
    }
    fn kind_label(&self) -> &'static str {
        match self {
            TypedPane::Terminal(_) => "Terminal",
            TypedPane::Code(_) => "Code",
            TypedPane::CodeDiff => "Code Diff",
            TypedPane::File => "File",
            TypedPane::Notebook { .. } => "Notebook",
            TypedPane::Workflow { .. } => "Workflow",
            TypedPane::Settings => "Settings",
            TypedPane::EnvVarCollection => "Environment Variables",
            TypedPane::EnvironmentManagement => "Environments",
            TypedPane::AIFact => "Rules",
            TypedPane::AIDocument => "Plan",
            TypedPane::ExecutionProfileEditor => "Execution Profile",
            TypedPane::Other => "Other",
        }
    }

    fn badge(&self, app: &AppContext) -> Option<String> {
        match self {
            TypedPane::Code(code_pane) => code_pane
                .file_view(app)
                .as_ref(app)
                .contains_unsaved_changes(app)
                .then(|| "Unsaved".to_string()),
            TypedPane::Terminal(_)
            | TypedPane::CodeDiff
            | TypedPane::File
            | TypedPane::Notebook { .. }
            | TypedPane::Workflow { .. }
            | TypedPane::Settings
            | TypedPane::EnvVarCollection
            | TypedPane::EnvironmentManagement
            | TypedPane::AIFact
            | TypedPane::AIDocument
            | TypedPane::ExecutionProfileEditor
            | TypedPane::Other => None,
        }
    }

    fn icon(&self) -> WarpIcon {
        match self {
            TypedPane::Terminal(_) => WarpIcon::Terminal,
            TypedPane::Code(_) => WarpIcon::Code2,
            TypedPane::CodeDiff => WarpIcon::Diff,
            TypedPane::File => WarpIcon::File,
            TypedPane::Notebook { is_plan: true } => WarpIcon::Compass,
            TypedPane::Notebook { is_plan: false } => WarpIcon::Notebook,
            TypedPane::Workflow { is_ai_prompt: true } => WarpIcon::Prompt,
            TypedPane::Workflow {
                is_ai_prompt: false,
            } => WarpIcon::Workflow,
            TypedPane::Settings | TypedPane::EnvironmentManagement => WarpIcon::Gear,
            TypedPane::EnvVarCollection => WarpIcon::EnvVarCollection,
            TypedPane::AIFact => WarpIcon::BookOpen,
            TypedPane::AIDocument => WarpIcon::Compass,
            TypedPane::ExecutionProfileEditor => WarpIcon::Lightning,
            TypedPane::Other => WarpIcon::File,
        }
    }
}

fn pane_display_title_and_subtitle(
    typed: &TypedPane<'_>,
    title: &str,
    secondary_title: &str,
) -> (String, String) {
    if matches!(typed, TypedPane::Code(_)) && !title.is_empty() {
        let path = Path::new(title);
        let filename = path
            .file_name()
            .map(|file_name| file_name.to_string_lossy().to_string())
            .unwrap_or_else(|| title.to_string());
        let parent_raw = path
            .parent()
            .map(|parent| parent.to_string_lossy().to_string())
            .unwrap_or_default();
        let home_dir = dirs::home_dir();
        let home_str = home_dir.as_ref().and_then(|path| path.to_str());
        let parent = warp_util::path::user_friendly_path(&parent_raw, home_str).to_string();
        (filename, parent)
    } else {
        (
            if title.is_empty() {
                typed.kind_label().to_string()
            } else {
                title.to_string()
            },
            secondary_title.to_string(),
        )
    }
}

fn build_vertical_tabs_summary_data(
    pane_group: &PaneGroup,
    visible_pane_ids: &[PaneId],
    app: &AppContext,
) -> VerticalTabsSummaryData {
    let mut primary_labels = Vec::new();
    let mut primary_seen = HashMap::new();
    let mut working_directories = Vec::new();
    let mut working_directory_seen = HashMap::new();
    let mut branch_entries = Vec::new();

    for pane_id in visible_pane_ids {
        let Some(pane) = pane_group.pane_by_id(*pane_id) else {
            continue;
        };
        let pane_configuration = pane.pane_configuration();
        let pane_configuration = pane_configuration.as_ref(app);
        let typed = pane_group.resolve_pane_type(*pane_id, app);
        let (pane_title, pane_subtitle) = pane_display_title_and_subtitle(
            &typed,
            pane_configuration.title().trim(),
            pane_configuration.title_secondary().trim(),
        );

        match typed {
            TypedPane::Terminal(terminal_pane) => {
                let terminal_view = terminal_pane.terminal_view(app);
                let terminal_view = terminal_view.as_ref(app);
                let title_text = terminal_view.terminal_title_from_shell();
                let working_directory = resolved_terminal_working_directory(terminal_view, app);
                let working_directory_text = working_directory
                    .clone()
                    .filter(|wd| !wd.trim().is_empty())
                    .unwrap_or_else(|| title_text.clone());
                let agent_text = terminal_agent_text(terminal_view, app);
                let (conversation_display_title, cli_agent_title) =
                    preferred_agent_tab_titles(&agent_text, agent_tab_text_preference(app));

                let primary_label = terminal_primary_line_data(
                    terminal_view.is_long_running_and_user_controlled(),
                    conversation_display_title,
                    cli_agent_title,
                    title_text.as_str(),
                    working_directory_text.as_str(),
                    terminal_title_fallback_font(&agent_text),
                    terminal_view.last_completed_command_text(),
                );
                let status = summary_conversation_status_for_terminal(terminal_view, app);
                push_normalized_unique_summary_label(
                    &mut primary_labels,
                    &mut primary_seen,
                    primary_label.text(),
                    status,
                );

                if let Some(working_directory) = working_directory {
                    push_normalized_unique_summary_text(
                        &mut working_directories,
                        &mut working_directory_seen,
                        &working_directory,
                    );
                }

                if let (Some(repo_path), Some(branch_name)) = (
                    terminal_view
                        .current_local_repo_path()
                        .map(Path::to_path_buf),
                    terminal_view
                        .current_git_branch(app)
                        .and_then(|branch| normalize_summary_text(&branch)),
                ) {
                    branch_entries.push(VerticalTabsSummaryBranchEntry {
                        repo_path,
                        branch_name,
                        diff_stats: terminal_view.current_diff_line_changes(app),
                        pull_request_label: terminal_view
                            .current_pull_request_url(app)
                            .as_deref()
                            .map(terminal_pull_request_badge_label)
                            .and_then(|label| normalize_summary_text(&label)),
                    });
                }
            }
            TypedPane::Code(_) => {
                push_normalized_unique_summary_label(
                    &mut primary_labels,
                    &mut primary_seen,
                    &pane_title,
                    None,
                );
                push_normalized_unique_summary_text(
                    &mut working_directories,
                    &mut working_directory_seen,
                    &pane_subtitle,
                );
            }
            TypedPane::CodeDiff
            | TypedPane::File
            | TypedPane::Notebook { .. }
            | TypedPane::Workflow { .. }
            | TypedPane::Settings
            | TypedPane::EnvVarCollection
            | TypedPane::EnvironmentManagement
            | TypedPane::AIFact
            | TypedPane::AIDocument
            | TypedPane::ExecutionProfileEditor
            | TypedPane::Other => {
                push_normalized_unique_summary_label(
                    &mut primary_labels,
                    &mut primary_seen,
                    &pane_title,
                    None,
                );
            }
        }
    }

    sort_summary_primary_labels_status_first(&mut primary_labels);

    VerticalTabsSummaryData {
        primary_labels,
        working_directories,
        branch_entries: coalesce_summary_branch_entries(branch_entries),
    }
}

impl<'a> PaneProps<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        pane_group: &'a PaneGroup,
        pane_id: PaneId,
        pane_group_id: EntityId,
        is_active_tab: bool,
        pane_row_state: PaneRowState,
        detail_hover_state: VerticalTabsDetailHoverState,
        display_granularity: VerticalTabsDisplayGranularity,
        include_custom_vertical_tabs_title: bool,
        display_title_override: Option<String>,
        renamable_tab_index: Option<usize>,
        pane_context_menu_tab_index: Option<usize>,
        is_tab_being_renamed: bool,
        rename_editor: Option<ViewHandle<EditorView>>,
        is_pane_being_renamed: bool,
        pane_rename_editor: Option<ViewHandle<EditorView>>,
        app: &AppContext,
    ) -> Option<Self> {
        let pane = pane_group.pane_by_id(pane_id)?;

        // When a pane is a temporary replacement (e.g. an expanded code diff),
        // resolve display properties from the original hidden pane so the
        // sidebar row keeps showing the original icon, title, and metadata.
        let display_pane_id = pane_group
            .original_pane_for_replacement(pane_id)
            .unwrap_or(pane_id);
        let display_pane = pane_group.pane_by_id(display_pane_id)?;
        let pane_configuration = display_pane.pane_configuration();
        let pane_configuration = pane_configuration.as_ref(app);
        let typed = pane_group.resolve_pane_type(display_pane_id, app);
        let (display_title, display_subtitle) = pane_display_title_and_subtitle(
            &typed,
            pane_configuration.title().trim(),
            pane_configuration.title_secondary().trim(),
        );

        Some(Self {
            pane_id,
            pane_group_id,
            is_active_tab,
            mouse_state: pane_row_state.mouse_state,
            title_mouse_state: pane_row_state.title_mouse_state,
            title: display_title,
            subtitle: display_subtitle,
            custom_vertical_tabs_title: include_custom_vertical_tabs_title
                .then(|| {
                    pane_configuration
                        .custom_vertical_tabs_title()
                        .map(str::to_owned)
                })
                .flatten(),
            display_title_override,
            is_focused: pane_group.focused_pane_id(app) == pane_id,
            typed,
            is_being_dragged: pane.is_pane_being_dragged(app),
            pane_color: pane_row_state.pane_color,
            badge_mouse_states: pane_row_state.badge_mouse_states,
            detail_hover_state,
            display_granularity,
            renamable_tab_index,
            pane_context_menu_tab_index,
            is_tab_being_renamed,
            rename_editor,
            is_pane_being_renamed,
            pane_rename_editor,
        })
    }

    fn displayed_title(&self) -> &str {
        self.custom_vertical_tabs_title
            .as_deref()
            .or(self.display_title_override.as_deref())
            .unwrap_or(self.title.as_str())
    }

    fn generated_or_tab_title(&self) -> &str {
        self.display_title_override
            .as_deref()
            .unwrap_or(self.title.as_str())
    }

    fn shows_inline_tab_rename_editor(&self) -> bool {
        (self.is_tab_being_renamed && self.rename_editor.is_some())
            || (self.is_pane_being_renamed && self.pane_rename_editor.is_some())
    }

    fn rendered_search_text_fragments(&self, app: &AppContext) -> Vec<String> {
        let generated_fragments = match &self.typed {
            TypedPane::Terminal(terminal_pane) => terminal_pane_search_text_fragments(
                terminal_pane,
                self.display_title_override.as_deref(),
                app,
            ),
            TypedPane::Code(_)
            | TypedPane::CodeDiff
            | TypedPane::File
            | TypedPane::Notebook { .. }
            | TypedPane::Workflow { .. }
            | TypedPane::Settings
            | TypedPane::EnvVarCollection
            | TypedPane::EnvironmentManagement
            | TypedPane::AIFact
            | TypedPane::AIDocument
            | TypedPane::ExecutionProfileEditor
            | TypedPane::Other => {
                non_terminal_search_text_fragments(self.generated_or_tab_title(), &self.subtitle)
            }
        };
        pane_search_text_fragments(
            self.custom_vertical_tabs_title.as_deref(),
            generated_fragments,
        )
    }
}

fn pane_matches_query(props: &PaneProps<'_>, query_lower: &str, app: &AppContext) -> bool {
    search_fragments_contain_query(&props.rendered_search_text_fragments(app), query_lower)
}

fn uses_outer_group_container(display_granularity: VerticalTabsDisplayGranularity) -> bool {
    matches!(display_granularity, VerticalTabsDisplayGranularity::Panes)
}

fn search_fragments_contain_query(fragments: &[String], query_lower: &str) -> bool {
    fragments
        .iter()
        .filter(|fragment| !fragment.trim().is_empty())
        .any(|fragment| fragment.to_lowercase().contains(query_lower))
}

fn pane_search_text_fragments(
    custom_title: Option<&str>,
    generated_fragments: Vec<String>,
) -> Vec<String> {
    let mut fragments = Vec::new();
    let mut seen = HashMap::new();
    if let Some(custom_title) = custom_title {
        push_normalized_unique_summary_text(&mut fragments, &mut seen, custom_title);
    }
    for fragment in generated_fragments {
        push_normalized_unique_summary_text(&mut fragments, &mut seen, &fragment);
    }
    fragments
}

fn non_terminal_search_text_fragments(title: &str, subtitle: &str) -> Vec<String> {
    let mut fragments = vec![title.to_string()];
    if !subtitle.trim().is_empty() {
        fragments.push(subtitle.to_string());
    }
    fragments
}

fn terminal_pane_search_text_fragments(
    terminal_pane: &TerminalPane,
    display_title_override: Option<&str>,
    app: &AppContext,
) -> Vec<String> {
    let terminal_view = terminal_pane.terminal_view(app);
    let terminal_view = terminal_view.as_ref(app);
    let title_text = terminal_view.terminal_title_from_shell();
    let working_directory = resolved_terminal_working_directory(terminal_view, app)
        .unwrap_or_else(|| title_text.clone());
    let agent_text = terminal_agent_text(terminal_view, app);
    let (conversation_display_title, cli_agent_title) =
        preferred_agent_tab_titles(&agent_text, agent_tab_text_preference(app));

    let primary_text = display_title_override
        .map(str::to_owned)
        .unwrap_or_else(|| {
            terminal_primary_line_data(
                terminal_view.is_long_running_and_user_controlled(),
                conversation_display_title,
                cli_agent_title,
                title_text.as_str(),
                working_directory.as_str(),
                terminal_title_fallback_font(&agent_text),
                terminal_view.last_completed_command_text(),
            )
            .text()
            .to_string()
        });
    let pull_request_label = terminal_view
        .current_pull_request_url(app)
        .as_deref()
        .map(terminal_pull_request_badge_label);

    terminal_search_text_fragments(
        primary_text,
        working_directory,
        terminal_view.current_git_branch(app),
        terminal_kind_badge_label(agent_text.is_oz_agent, agent_text.cli_agent),
        pull_request_label,
        terminal_view.current_diff_line_changes(app),
    )
}

fn terminal_search_text_fragments(
    primary_text: String,
    working_directory: String,
    git_branch: Option<String>,
    kind_badge_label: String,
    pull_request_label: Option<String>,
    diff_stats: Option<GitLineChanges>,
) -> Vec<String> {
    let mut fragments = vec![primary_text, working_directory, kind_badge_label];
    if let Some(git_branch) = git_branch.filter(|branch| !branch.trim().is_empty()) {
        fragments.push(git_branch);
    }
    if let Some(pull_request_label) = pull_request_label.filter(|label| !label.trim().is_empty()) {
        fragments.push(pull_request_label);
    }
    if let Some(diff_stats) = diff_stats {
        fragments.push(vtab_diff_stats_text(&diff_stats));
    }
    fragments
}

fn terminal_primary_line_data(
    is_long_running: bool,
    conversation_display_title: Option<String>,
    cli_agent_title: Option<String>,
    terminal_title: &str,
    working_directory: &str,
    terminal_title_font: TerminalPrimaryLineFont,
    last_completed_command: Option<String>,
) -> TerminalPrimaryLineData {
    let trimmed_title = terminal_title.trim();
    let trimmed_working_directory = working_directory.trim();
    if let Some(cli_agent_title) = cli_agent_title {
        return TerminalPrimaryLineData::StatusText {
            text: cli_agent_title,
        };
    }

    if is_long_running && !trimmed_title.is_empty() && trimmed_title != trimmed_working_directory {
        return TerminalPrimaryLineData::Text {
            text: trimmed_title.to_string(),
            font: TerminalPrimaryLineFont::Monospace,
        };
    }

    if let Some(conversation_title) = conversation_display_title {
        return TerminalPrimaryLineData::StatusText {
            text: conversation_title,
        };
    }
    if !trimmed_title.is_empty() && trimmed_title != trimmed_working_directory {
        return TerminalPrimaryLineData::Text {
            text: trimmed_title.to_string(),
            font: terminal_title_font,
        };
    }

    if let Some(last_completed_command) = last_completed_command {
        return TerminalPrimaryLineData::Text {
            text: last_completed_command,
            font: TerminalPrimaryLineFont::Monospace,
        };
    }

    TerminalPrimaryLineData::Text {
        text: "New session".to_string(),
        font: TerminalPrimaryLineFont::Ui,
    }
}

fn terminal_kind_badge_label(is_oz_agent: bool, cli_agent: Option<CLIAgent>) -> String {
    if let Some(cli_agent) = cli_agent {
        cli_agent.display_name().to_string()
    } else if is_oz_agent {
        "Oz".to_string()
    } else {
        "Terminal".to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentTabTextPreference {
    ConversationTitle,
    LatestUserPrompt,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TerminalAgentText {
    conversation_display_title: Option<String>,
    conversation_latest_user_prompt: Option<String>,
    cli_agent_title: Option<String>,
    cli_agent_latest_user_prompt: Option<String>,
    is_oz_agent: bool,
    cli_agent: Option<CLIAgent>,
}

fn agent_tab_text_preference(app: &AppContext) -> AgentTabTextPreference {
    if *TabSettings::as_ref(app).use_latest_user_prompt_as_conversation_title_in_tab_names {
        AgentTabTextPreference::LatestUserPrompt
    } else {
        AgentTabTextPreference::ConversationTitle
    }
}

fn preferred_agent_tab_titles(
    agent_text: &TerminalAgentText,
    preference: AgentTabTextPreference,
) -> (Option<String>, Option<String>) {
    let conversation_title = match preference {
        AgentTabTextPreference::ConversationTitle => agent_text
            .conversation_display_title
            .clone()
            .or_else(|| agent_text.conversation_latest_user_prompt.clone()),
        AgentTabTextPreference::LatestUserPrompt => agent_text
            .conversation_latest_user_prompt
            .clone()
            .or_else(|| agent_text.conversation_display_title.clone()),
    };
    let cli_agent_title = match preference {
        AgentTabTextPreference::ConversationTitle => agent_text.cli_agent_title.clone(),
        AgentTabTextPreference::LatestUserPrompt => agent_text
            .cli_agent_latest_user_prompt
            .clone()
            .or_else(|| agent_text.cli_agent_title.clone()),
    };

    (conversation_title, cli_agent_title)
}

fn terminal_agent_text(terminal_view: &TerminalView, app: &AppContext) -> TerminalAgentText {
    let cli_agent_session = CLIAgentSessionsModel::as_ref(app).session(terminal_view.id());
    let is_plugin_backed = cli_agent_session.is_some_and(|session| session.listener.is_some());
    let is_ambient_agent = terminal_view.is_ambient_agent_session(app);

    let mut agent_text = TerminalAgentText {
        is_oz_agent: is_ambient_agent,
        cli_agent: cli_agent_session.map(|session| session.agent),
        ..Default::default()
    };

    if cli_agent_session.is_some() && !is_plugin_backed {
        return agent_text;
    }

    agent_text.conversation_display_title = terminal_view.selected_conversation_display_title(app);
    agent_text.conversation_latest_user_prompt =
        terminal_view.selected_conversation_latest_user_prompt_for_tab_name(app);
    agent_text.is_oz_agent =
        agent_text.conversation_display_title.is_some() || agent_text.is_oz_agent;

    if let Some(session) = cli_agent_session {
        agent_text.cli_agent_title = session.session_context.title_like_text();
        agent_text.cli_agent_latest_user_prompt = session.session_context.latest_user_prompt();
    }

    agent_text
}

fn terminal_pull_request_badge_label(pull_request_url: &str) -> String {
    github_pr_display_text_from_url(pull_request_url)
        .map(|label| label.strip_prefix("PR ").unwrap_or(&label).to_string())
        .unwrap_or_else(|| pull_request_url.to_string())
}

fn vtab_diff_stats_tokens(line_changes: &GitLineChanges) -> Vec<String> {
    let mut tokens = Vec::new();
    if line_changes.lines_added > 0 {
        tokens.push(format!("+{}", line_changes.lines_added));
    }
    if line_changes.lines_removed > 0 {
        tokens.push(format!("-{}", line_changes.lines_removed));
    }
    if tokens.is_empty() {
        tokens.push("0".to_string());
    }
    tokens
}

fn vtab_diff_stats_text(line_changes: &GitLineChanges) -> String {
    vtab_diff_stats_tokens(line_changes).join(" ")
}

impl PaneGroup {
    fn resolve_pane_type(&self, pane_id: PaneId, app: &AppContext) -> TypedPane<'_> {
        match pane_id.pane_type() {
            IPaneType::Terminal => TypedPane::Terminal(
                self.downcast_pane_by_id::<TerminalPane>(pane_id)
                    .expect("IPaneType::Terminal must correspond to a TerminalPane"),
            ),
            IPaneType::Code => TypedPane::Code(
                self.downcast_pane_by_id::<CodePane>(pane_id)
                    .expect("IPaneType::Code must correspond to a CodePane"),
            ),
            IPaneType::CodeDiff => TypedPane::CodeDiff,
            IPaneType::File => TypedPane::File,
            IPaneType::Notebook => {
                let is_plan = self
                    .downcast_pane_by_id::<NotebookPane>(pane_id)
                    .map(|np| np.notebook_view(app).as_ref(app).is_plan(app))
                    .unwrap_or(false);
                TypedPane::Notebook { is_plan }
            }
            IPaneType::Workflow => {
                let is_ai_prompt = self
                    .downcast_pane_by_id::<WorkflowPane>(pane_id)
                    .map(|wp| {
                        let wv = wp.get_view(app);
                        wv.as_ref(app).is_agent_mode_workflow()
                    })
                    .unwrap_or(false);
                TypedPane::Workflow { is_ai_prompt }
            }
            IPaneType::Settings => TypedPane::Settings,
            IPaneType::EnvVarCollection => TypedPane::EnvVarCollection,
            IPaneType::EnvironmentManagement => TypedPane::EnvironmentManagement,
            IPaneType::AIFact => TypedPane::AIFact,
            IPaneType::AIDocument => TypedPane::AIDocument,
            IPaneType::ExecutionProfileEditor => TypedPane::ExecutionProfileEditor,
            IPaneType::GetStarted
            | IPaneType::NetworkLog
            | IPaneType::Welcome
            | IPaneType::DeferredPlaceholder => TypedPane::Other,
            #[cfg(test)]
            IPaneType::Dummy => TypedPane::Other,
        }
    }
}

/// Returns the best available working-directory string for a terminal pane,
/// incorporating cloud environment name and setup status for ambient agent sessions.
fn resolved_terminal_working_directory(
    terminal_view: &TerminalView,
    app: &AppContext,
) -> Option<String> {
    let working_directory = terminal_view
        .display_working_directory(app)
        .filter(|wd| !wd.trim().is_empty());
    cloud_agent_working_directory_and_env(terminal_view, working_directory.as_deref(), app)
        .or(working_directory)
}

/// For cloud agent panes, builds a composite string from the environment name,
/// setup status, and/or working directory. Returns `None` for non-cloud sessions.
fn cloud_agent_working_directory_and_env(
    terminal_view: &TerminalView,
    working_directory: Option<&str>,
    app: &AppContext,
) -> Option<String> {
    if !terminal_view.is_ambient_agent_session(app) {
        return None;
    }
    let model_ref = terminal_view.ambient_agent_view_model()?.as_ref(app);

    let env_name = model_ref
        .selected_environment_id()
        .and_then(|id| CloudAmbientAgentEnvironment::get_by_id(id, app))
        .map(|env| env.model().string_model.display_name());

    let setup_status: Option<&str> = model_ref.agent_progress().map(|p| p.setup_status_text());

    match (env_name, setup_status, working_directory) {
        (Some(env), Some(status), _) => Some(format!("{env} · {status}")),
        (Some(env), None, Some(wd)) => Some(format!("{env} · {wd}")),
        (Some(env), None, None) => Some(env),
        (None, Some(status), _) => Some(status.to_string()),
        (None, None, _) => None,
    }
}

fn render_terminal_row_content(
    props: &PaneProps<'_>,
    terminal_view: &TerminalView,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let main_text_color = theme.main_text_color(theme.background());
    let sub_text_color = theme.sub_text_color(theme.background());
    let primary_info = *TabSettings::as_ref(app).vertical_tabs_primary_info.value();

    let title_text = terminal_view.terminal_title_from_shell();
    let working_directory = resolved_terminal_working_directory(terminal_view, app)
        .unwrap_or_else(|| title_text.clone());

    let git_branch = terminal_view.current_git_branch(app);

    // Line 1 and line 2 depend on the "Pane title as" setting.
    // Line 3 (metadata) shows context data on the left + badges on the right.
    //
    // | Setting          | Line 1 (title)       | Line 2 (description)   | Line 3 left          |
    // |------------------|----------------------|------------------------|----------------------|
    // | Command          | command/conversation | working directory       | git branch           |
    // | WorkingDirectory | working directory    | command/conversation    | git branch           |
    // | Branch           | git branch           | command/conversation    | working directory    |
    let (first_line, second_line, metadata_left) = match primary_info {
        VerticalTabsPrimaryInfo::Command => (
            render_pane_title_slot(
                props,
                || {
                    render_terminal_primary_line_for_view(
                        terminal_view,
                        appearance,
                        main_text_color,
                        app,
                    )
                },
                12.,
                main_text_color,
                ClipConfig::ellipsis(),
                appearance,
                app,
            ),
            render_text_line(
                &working_directory,
                sub_text_color,
                ClipConfig::start(),
                appearance,
            ),
            MetadataLeftContent::GitBranch(git_branch),
        ),
        VerticalTabsPrimaryInfo::WorkingDirectory => (
            render_pane_title_slot(
                props,
                || {
                    render_text_line(
                        &working_directory,
                        main_text_color,
                        ClipConfig::start(),
                        appearance,
                    )
                },
                12.,
                main_text_color,
                ClipConfig::ellipsis(),
                appearance,
                app,
            ),
            render_terminal_primary_line_for_view(terminal_view, appearance, sub_text_color, app),
            MetadataLeftContent::GitBranch(git_branch),
        ),
        VerticalTabsPrimaryInfo::Branch => {
            let (branch_text, show_branch_icon) =
                branch_label_display(git_branch.as_deref(), working_directory.as_str());
            (
                render_pane_title_slot(
                    props,
                    || {
                        if show_branch_icon {
                            render_git_branch_text(&branch_text, main_text_color, 12., appearance)
                        } else {
                            render_text_line(
                                &branch_text,
                                main_text_color,
                                ClipConfig::start(),
                                appearance,
                            )
                        }
                    },
                    12.,
                    main_text_color,
                    ClipConfig::ellipsis(),
                    appearance,
                    app,
                ),
                render_terminal_primary_line_for_view(
                    terminal_view,
                    appearance,
                    sub_text_color,
                    app,
                ),
                MetadataLeftContent::WorkingDirectory(working_directory),
            )
        }
    };

    let first_line_element = if has_unread_activity(&props.typed, app) {
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(Shrinkable::new(1., first_line).finish())
            .with_child(
                Container::new(render_title_indicator(theme))
                    .with_margin_left(4.)
                    .finish(),
            )
            .finish()
    } else {
        first_line
    };

    let mut content = Flex::column()
        .with_main_axis_size(MainAxisSize::Min)
        .with_cross_axis_alignment(CrossAxisAlignment::Start);
    content.add_child(first_line_element);
    content.add_child(Container::new(second_line).with_margin_top(2.).finish());
    content.add_child(
        Container::new(render_terminal_metadata_line(
            terminal_view,
            props.pane_group_id,
            props.pane_id,
            metadata_left,
            chip_entrypoint_for_granularity(props.display_granularity),
            &props.badge_mouse_states,
            appearance,
            app,
        ))
        .with_margin_top(2.)
        .finish(),
    );
    content.finish()
}

fn chip_entrypoint_for_granularity(
    granularity: VerticalTabsDisplayGranularity,
) -> VerticalTabsChipEntrypoint {
    match granularity {
        VerticalTabsDisplayGranularity::Panes => VerticalTabsChipEntrypoint::Pane,
        VerticalTabsDisplayGranularity::Tabs => VerticalTabsChipEntrypoint::Tab,
    }
}

fn branch_label_display(git_branch: Option<&str>, fallback: &str) -> (String, bool) {
    match git_branch.filter(|branch| !branch.trim().is_empty()) {
        Some(branch) => (branch.to_string(), true),
        None => (fallback.to_string(), false),
    }
}

fn compact_branch_subtitle_display(
    git_branch: Option<&str>,
    working_directory: Option<&str>,
) -> Option<(String, bool)> {
    git_branch
        .filter(|branch| !branch.trim().is_empty())
        .map(|branch| (branch.to_string(), true))
        .or_else(|| {
            working_directory
                .filter(|wd| !wd.trim().is_empty())
                .map(|wd| (wd.to_string(), false))
        })
}

fn render_git_branch_text(
    branch: &str,
    text_color: WarpThemeFill,
    font_size: f32,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(2.)
        .with_child(
            ConstrainedBox::new(UiIcon::GitBranch.to_warpui_icon(text_color).finish())
                .with_width(font_size - 2.)
                .with_height(font_size - 2.)
                .finish(),
        )
        .with_child(
            Shrinkable::new(
                1.,
                Text::new_inline(branch.to_string(), appearance.ui_font_family(), font_size)
                    .with_clip(ClipConfig::ellipsis())
                    .with_color(text_color.into())
                    .finish(),
            )
            .finish(),
        )
        .finish()
}

enum MetadataLeftContent {
    GitBranch(Option<String>),
    WorkingDirectory(String),
}

fn render_text_line(
    text: &str,
    text_color: WarpThemeFill,
    clip: ClipConfig,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Text::new_inline(text.to_string(), appearance.ui_font_family(), 12.)
        .with_clip(clip)
        .with_color(text_color.into())
        .finish()
}

fn render_inline_tab_rename_editor(
    rename_editor: &ViewHandle<EditorView>,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let editor_line_height = rename_editor
        .as_ref(app)
        .line_height(app.font_cache(), appearance);
    TextInput::new(
        rename_editor.clone(),
        UiComponentStyles::default()
            .set_height(editor_line_height)
            .set_background(ElementFill::None)
            .set_border_radius(CornerRadius::with_all(Radius::Pixels(0.)))
            .set_border_width(0.),
    )
    .build()
    .finish()
}

fn render_title_override(
    props: &PaneProps<'_>,
    font_size: f32,
    text_color: WarpThemeFill,
    clip: ClipConfig,
    appearance: &Appearance,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    if props.is_tab_being_renamed {
        return props
            .rename_editor
            .as_ref()
            .map(|rename_editor| render_inline_tab_rename_editor(rename_editor, appearance, app));
    }
    if props.is_pane_being_renamed {
        return props
            .pane_rename_editor
            .as_ref()
            .map(|rename_editor| render_inline_tab_rename_editor(rename_editor, appearance, app));
    }

    props
        .custom_vertical_tabs_title
        .as_ref()
        .or(props.display_title_override.as_ref())
        .map(|title| {
            Text::new_inline(title.clone(), appearance.ui_font_family(), font_size)
                .with_clip(clip)
                .with_color(text_color.into())
                .finish()
        })
}

fn render_pane_title_slot(
    props: &PaneProps<'_>,
    generated_title: impl FnOnce() -> Box<dyn Element>,
    font_size: f32,
    text_color: WarpThemeFill,
    clip: ClipConfig,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let title = render_title_override(props, font_size, text_color, clip, appearance, app)
        .unwrap_or_else(generated_title);

    if !matches!(
        props.display_granularity,
        VerticalTabsDisplayGranularity::Panes
    ) || props.shows_inline_tab_rename_editor()
    {
        return title;
    }

    let Some(title_mouse_state) = props.title_mouse_state.clone() else {
        return title;
    };
    let locator = PaneViewLocator {
        pane_group_id: props.pane_group_id,
        pane_id: props.pane_id,
    };
    Hoverable::new(title_mouse_state, move |_| title)
        .on_double_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::RenamePane(locator));
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
}

fn render_summary_tab_item(
    props: PaneProps<'_>,
    summary: &VerticalTabsSummaryData,
    summary_pane_kind_icons: Option<SummaryPaneKindIcons>,
    app: &AppContext,
) -> Box<dyn Element> {
    // Region caps for v2 per-line rendering: each region shows at most 3 visible lines
    // before collapsing the rest into a `+ N more` overflow line.
    const MAX_VISIBLE_PRIMARY_LABELS: usize = 3;
    const MAX_VISIBLE_WORKING_DIRECTORIES: usize = 3;
    const MAX_VISIBLE_BRANCH_LINES: usize = 3;
    const REGION_GAP: f32 = 4.;
    const INTRA_REGION_GAP: f32 = 2.;

    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let main_text_color = theme.main_text_color(theme.background());
    let sub_text_color = theme.sub_text_color(theme.background());
    let icon = summary_pane_kind_icons
        .map(|icons| render_summary_pane_kind_icons(icons, appearance))
        .unwrap_or_else(|| {
            render_pane_icon_with_status(
                resolve_icon_with_status_variant(&props.typed, &props.title, appearance, app),
                theme,
            )
        });

    let mut text_col = Flex::column()
        .with_main_axis_size(MainAxisSize::Min)
        .with_cross_axis_alignment(CrossAxisAlignment::Start);

    // Title region. A custom-title or rename override short-circuits the per-label list and
    // renders as a single line (no prefix slot, no overflow line).
    if let Some(title_override) = render_title_override(
        &props,
        12.,
        main_text_color,
        ClipConfig::end(),
        appearance,
        app,
    ) {
        text_col.add_child(title_override);
    } else if summary.primary_labels.is_empty() {
        text_col.add_child(render_text_line(
            &props.title,
            main_text_color,
            ClipConfig::end(),
            appearance,
        ));
    } else {
        let visible_labels: Vec<&VerticalTabsSummaryPrimaryLabel> = summary
            .primary_labels
            .iter()
            .take(MAX_VISIBLE_PRIMARY_LABELS)
            .collect();
        let reserve_prefix_slot = visible_labels.iter().any(|label| label.status.is_some());

        for (idx, label) in visible_labels.iter().enumerate() {
            let line = render_summary_primary_label_line(
                label,
                reserve_prefix_slot,
                main_text_color,
                appearance,
            );
            text_col.add_child(if idx == 0 {
                line
            } else {
                Container::new(line)
                    .with_margin_top(INTRA_REGION_GAP)
                    .finish()
            });
        }

        let hidden_label_count =
            summary_overflow_count(summary.primary_labels.len(), MAX_VISIBLE_PRIMARY_LABELS);
        if hidden_label_count > 0 {
            text_col.add_child(
                Container::new(render_summary_overflow_line(
                    hidden_label_count,
                    sub_text_color,
                    appearance,
                ))
                .with_margin_top(INTRA_REGION_GAP)
                .finish(),
            );
        }
    }

    // Working-directory region.
    let visible_directory_count = summary
        .working_directories
        .len()
        .min(MAX_VISIBLE_WORKING_DIRECTORIES);
    for (idx, working_dir) in summary
        .working_directories
        .iter()
        .take(MAX_VISIBLE_WORKING_DIRECTORIES)
        .enumerate()
    {
        let margin = if idx == 0 {
            REGION_GAP
        } else {
            INTRA_REGION_GAP
        };
        text_col.add_child(
            Container::new(render_text_line(
                working_dir,
                sub_text_color,
                ClipConfig::start(),
                appearance,
            ))
            .with_margin_top(margin)
            .finish(),
        );
    }
    let hidden_directory_count = summary_overflow_count(
        summary.working_directories.len(),
        MAX_VISIBLE_WORKING_DIRECTORIES,
    );
    if hidden_directory_count > 0 {
        let margin = if visible_directory_count == 0 {
            REGION_GAP
        } else {
            INTRA_REGION_GAP
        };
        text_col.add_child(
            Container::new(render_summary_overflow_line(
                hidden_directory_count,
                sub_text_color,
                appearance,
            ))
            .with_margin_top(margin)
            .finish(),
        );
    }

    // Branch region. Each branch line gets the existing 4px top margin from APP-3875.
    for branch_entry in summary.branch_entries.iter().take(MAX_VISIBLE_BRANCH_LINES) {
        text_col.add_child(
            Container::new(render_summary_branch_line(branch_entry, appearance))
                .with_margin_top(REGION_GAP)
                .finish(),
        );
    }

    let hidden_branch_count =
        summary_overflow_count(summary.branch_entries.len(), MAX_VISIBLE_BRANCH_LINES);
    if hidden_branch_count > 0 {
        text_col.add_child(
            Container::new(render_summary_overflow_line(
                hidden_branch_count,
                sub_text_color,
                appearance,
            ))
            .with_margin_top(REGION_GAP)
            .finish(),
        );
    }

    let content = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_spacing(ICON_WITH_STATUS_GAP)
        .with_child(icon)
        .with_child(Shrinkable::new(1., text_col.finish()).finish())
        .finish();

    render_pane_row_element(props, Padding::uniform(8.), true, content, theme)
}

fn render_summary_primary_label_line(
    label: &VerticalTabsSummaryPrimaryLabel,
    reserve_prefix_slot: bool,
    text_color: WarpThemeFill,
    appearance: &Appearance,
) -> Box<dyn Element> {
    // Reserve a slot wide enough for the status pill so non-conversation lines align with
    // conversation lines in the same region. STATUS_ELEMENT_PADDING is the 2px padding inside
    // the pill from `render_status_element`.
    const STATUS_ELEMENT_PADDING: f32 = 2.;
    let prefix_slot_size = VERTICAL_TABS_SUMMARY_STATUS_ICON_SIZE + STATUS_ELEMENT_PADDING * 2.;
    let text = render_text_line(&label.text, text_color, ClipConfig::end(), appearance);

    let prefix: Option<Box<dyn Element>> = match (label.status.as_ref(), reserve_prefix_slot) {
        (Some(status), _) => Some(render_status_element(
            status,
            VERTICAL_TABS_SUMMARY_STATUS_ICON_SIZE,
            appearance,
        )),
        (None, true) => Some(
            ConstrainedBox::new(Empty::new().finish())
                .with_width(prefix_slot_size)
                .with_height(prefix_slot_size)
                .finish(),
        ),
        (None, false) => None,
    };

    let Some(prefix) = prefix else {
        return text;
    };
    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(4.)
        .with_child(prefix)
        .with_child(Shrinkable::new(1., text).finish())
        .finish()
}

fn render_summary_overflow_line(
    hidden_count: usize,
    text_color: WarpThemeFill,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Text::new_inline(
        format!("+ {hidden_count} more"),
        appearance.ui_font_family(),
        10.,
    )
    .with_clip(ClipConfig::end())
    .with_color(text_color.into())
    .finish()
}

fn render_summary_pane_kind_icons(
    icons: SummaryPaneKindIcons,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    match icons {
        SummaryPaneKindIcons::Single(kind) => {
            render_summary_pane_kind_icon_circle(kind, VERTICAL_TABS_ICON_SIZE, appearance)
        }
        SummaryPaneKindIcons::Pair { primary, secondary } => {
            // The secondary icon sits at the BR of the primary at roughly badge
            // proportions, with a small cutout ring separating it from the primary.
            let primary_total = VERTICAL_TABS_ICON_SIZE;
            let secondary_total = VERTICAL_TABS_ICON_SIZE * 0.5;
            let ring_padding = secondary_total * 0.1;
            let primary_icon =
                render_summary_pane_kind_icon_circle(primary, primary_total, appearance);
            let secondary_icon =
                render_summary_pane_kind_icon_circle(secondary, secondary_total, appearance);
            let secondary_with_ring = Container::new(secondary_icon)
                .with_uniform_padding(ring_padding)
                .with_background(theme.background())
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                .finish();

            // Same 45° placement as `render_with_optional_status_badge`: secondary's
            // center sits on the primary circle's edge.
            let primary_radius = primary_total / 2.;
            let secondary_outer = secondary_total + ring_padding * 2.;
            let secondary_radius = secondary_outer / 2.;
            let secondary_corner_offset = primary_radius * std::f32::consts::FRAC_1_SQRT_2
                + secondary_radius
                - primary_total / 2.;

            let mut stack = Stack::new().with_child(
                ConstrainedBox::new(primary_icon)
                    .with_width(primary_total)
                    .with_height(primary_total)
                    .finish(),
            );
            stack.add_positioned_child(
                secondary_with_ring,
                OffsetPositioning::offset_from_parent(
                    vec2f(secondary_corner_offset, secondary_corner_offset),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::BottomRight,
                    ChildAnchor::BottomRight,
                ),
            );
            ConstrainedBox::new(stack.finish())
                .with_width(primary_total)
                .with_height(primary_total)
                .finish()
        }
    }
}

// Inline rendering for non-agent summary kinds — for an icon (e.g. Terminal, Code,
// Notebook) sized to fill its `total_size` bounding box.
const SUMMARY_INLINE_ICON_RATIO: f32 = 2. / 3.;
const SUMMARY_INLINE_PADDING_RATIO: f32 = (1. - SUMMARY_INLINE_ICON_RATIO) / 2.;

fn render_summary_pane_kind_icon_circle(
    kind: SummaryPaneKind,
    total_size: f32,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    // For ambient Oz / CLI agent kinds, delegate to `render_icon_with_status` so the
    // brand-color circle is overlaid with the white cloud badge (status-less in summary
    // mode). Non-ambient agent kinds and all other pane kinds fall through to the inline
    // circle rendering below.
    if let Some(variant) = ambient_agent_variant(&kind) {
        return render_icon_with_status(variant, total_size, 0., theme, theme.background());
    }
    let icon_size = total_size * SUMMARY_INLINE_ICON_RATIO;
    let padding = total_size * SUMMARY_INLINE_PADDING_RATIO;
    let (icon_element, background): (Box<dyn Element>, ElementFill) = match kind {
        SummaryPaneKind::OzAgent { .. } => (
            WarpIcon::Oz.to_warpui_icon(oz_icon_fill(theme)).finish(),
            theme.background().into(),
        ),
        SummaryPaneKind::CLIAgent { agent, .. } => {
            let icon_color = agent.brand_icon_color();
            let icon_element = agent
                .icon()
                .map(|icon| {
                    icon.to_warpui_icon(WarpThemeFill::Solid(icon_color))
                        .finish()
                })
                .unwrap_or_else(|| {
                    WarpIcon::Terminal
                        .to_warpui_icon(theme.sub_text_color(theme.background()))
                        .finish()
                });
            (
                icon_element,
                ThemeFill::Solid(
                    agent
                        .brand_color()
                        .unwrap_or(ColorU::new(100, 100, 100, 255)),
                )
                .into(),
            )
        }
        SummaryPaneKind::Code { title } => (
            icon_from_file_path(&title, appearance).unwrap_or_else(|| {
                WarpIcon::Code2
                    .to_warpui_icon(theme.sub_text_color(theme.background()))
                    .finish()
            }),
            internal_colors::fg_overlay_2(theme).into(),
        ),
        SummaryPaneKind::Terminal
        | SummaryPaneKind::CodeDiff
        | SummaryPaneKind::File
        | SummaryPaneKind::Notebook { .. }
        | SummaryPaneKind::Workflow { .. }
        | SummaryPaneKind::Settings
        | SummaryPaneKind::EnvVarCollection
        | SummaryPaneKind::EnvironmentManagement
        | SummaryPaneKind::AIFact
        | SummaryPaneKind::AIDocument
        | SummaryPaneKind::ExecutionProfileEditor
        | SummaryPaneKind::Other => {
            let (icon, icon_color) = summary_pane_kind_icon(kind, appearance);
            (
                icon.to_warpui_icon(icon_color).finish(),
                internal_colors::fg_overlay_2(theme).into(),
            )
        }
    };
    Container::new(
        ConstrainedBox::new(icon_element)
            .with_width(icon_size)
            .with_height(icon_size)
            .finish(),
    )
    .with_uniform_padding(padding)
    .with_background(background)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
        (icon_size + padding * 2.) / 2.,
    )))
    .finish()
}

/// Maps an ambient Oz / CLI agent summary-pane kind to the `IconWithStatusVariant` used to
/// render the brand-color circle with the white cloud badge. Non-ambient kinds (and all
/// other pane kinds) return `None` so the caller falls back to its inline rendering.
fn ambient_agent_variant(kind: &SummaryPaneKind) -> Option<IconWithStatusVariant> {
    match kind {
        SummaryPaneKind::OzAgent { is_ambient: true } => Some(IconWithStatusVariant::OzAgent {
            status: None,
            is_ambient: true,
        }),
        SummaryPaneKind::CLIAgent {
            agent,
            is_ambient: true,
        } => Some(IconWithStatusVariant::CLIAgent {
            agent: *agent,
            status: None,
            is_ambient: true,
        }),
        _ => None,
    }
}

fn summary_pane_kind_icon(
    kind: SummaryPaneKind,
    appearance: &Appearance,
) -> (WarpIcon, WarpThemeFill) {
    let theme = appearance.theme();
    let main_text = theme.main_text_color(theme.background());
    let sub_text = theme.sub_text_color(theme.background());
    let drive_color = |object_type: DriveObjectType| -> WarpThemeFill {
        WarpThemeFill::Solid(warp_drive_icon_color(appearance, object_type))
    };

    match kind {
        SummaryPaneKind::Terminal => (WarpIcon::Terminal, main_text),
        SummaryPaneKind::OzAgent { .. } => (WarpIcon::Oz, main_text),
        SummaryPaneKind::CLIAgent { agent, .. } => (
            agent.icon().unwrap_or(WarpIcon::Terminal),
            WarpThemeFill::Solid(agent.brand_icon_color()),
        ),
        SummaryPaneKind::Code { .. } => (WarpIcon::Code2, sub_text),
        SummaryPaneKind::CodeDiff => (WarpIcon::Diff, sub_text),
        SummaryPaneKind::File => (WarpIcon::File, sub_text),
        SummaryPaneKind::Notebook { is_plan } => (
            if is_plan {
                WarpIcon::Compass
            } else {
                WarpIcon::Notebook
            },
            drive_color(DriveObjectType::Notebook {
                is_ai_document: is_plan,
            }),
        ),
        SummaryPaneKind::Workflow { is_ai_prompt } => (
            if is_ai_prompt {
                WarpIcon::Prompt
            } else {
                WarpIcon::Workflow
            },
            if is_ai_prompt {
                drive_color(DriveObjectType::AgentModeWorkflow)
            } else {
                drive_color(DriveObjectType::Workflow)
            },
        ),
        SummaryPaneKind::Settings | SummaryPaneKind::EnvironmentManagement => {
            (WarpIcon::Gear, main_text)
        }
        SummaryPaneKind::EnvVarCollection => (
            WarpIcon::EnvVarCollection,
            drive_color(DriveObjectType::EnvVarCollection),
        ),
        SummaryPaneKind::AIFact => (WarpIcon::BookOpen, drive_color(DriveObjectType::AIFact)),
        SummaryPaneKind::AIDocument => (WarpIcon::Compass, sub_text),
        SummaryPaneKind::ExecutionProfileEditor => (WarpIcon::Lightning, sub_text),
        SummaryPaneKind::Other => (WarpIcon::File, sub_text),
    }
}

fn render_summary_branch_line(
    entry: &VerticalTabsSummaryBranchEntry,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let sub_text_color = theme.sub_text_color(theme.background());
    let mut row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Shrinkable::new(
                1.,
                render_git_branch_text(&entry.branch_name, sub_text_color, 10., appearance),
            )
            .finish(),
        );

    let mut right_badges = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(4.);
    let mut has_right_badges = false;
    if let Some(diff_stats) = &entry.diff_stats {
        right_badges.add_child(render_passive_terminal_diff_stats_badge(
            diff_stats, appearance,
        ));
        has_right_badges = true;
    }
    if let Some(pull_request_label) = &entry.pull_request_label {
        right_badges.add_child(render_passive_terminal_pull_request_badge(
            pull_request_label,
            appearance,
        ));
        has_right_badges = true;
    }
    if has_right_badges {
        row.add_child(
            Container::new(right_badges.finish())
                .with_padding_left(4.)
                .finish(),
        );
    }

    ConstrainedBox::new(row.finish())
        .with_height(METADATA_ROW_HEIGHT)
        .finish()
}

fn render_terminal_primary_line_for_view(
    terminal_view: &TerminalView,
    appearance: &Appearance,
    text_color: WarpThemeFill,
    app: &AppContext,
) -> Box<dyn Element> {
    let title_text = terminal_view.terminal_title_from_shell();
    let working_directory = resolved_terminal_working_directory(terminal_view, app)
        .unwrap_or_else(|| title_text.clone());
    let agent_text = terminal_agent_text(terminal_view, app);
    let (conversation_display_title, cli_agent_title) =
        preferred_agent_tab_titles(&agent_text, agent_tab_text_preference(app));

    render_terminal_primary_line(
        terminal_primary_line_data(
            terminal_view.is_long_running_and_user_controlled(),
            conversation_display_title,
            cli_agent_title,
            title_text.as_str(),
            working_directory.as_str(),
            terminal_title_fallback_font(&agent_text),
            terminal_view.last_completed_command_text(),
        ),
        terminal_view,
        appearance,
        text_color,
    )
}

/// Primary line for terminal pane rows. Precedence:
/// 1. CLI agent session with plugin data (query/summary) + status
/// 2. Oz agent conversation title + status
/// 3. Terminal title
fn render_terminal_primary_line(
    primary_line: TerminalPrimaryLineData,
    terminal_view: &TerminalView,
    appearance: &Appearance,
    text_color: WarpThemeFill,
) -> Box<dyn Element> {
    let theme = appearance.theme();

    let is_errored = terminal_view.current_state().state == TerminalViewState::Errored;
    let error_color = theme.ui_error_color();

    let wrap_with_error_indicator = |title_element: Box<dyn Element>| -> Box<dyn Element> {
        if !is_errored {
            return title_element;
        }
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.)
            .with_child(
                ConstrainedBox::new(
                    UiIcon::AlertTriangle
                        .to_warpui_icon(error_color.into())
                        .finish(),
                )
                .with_width(BADGE_ICON_SIZE)
                .with_height(BADGE_ICON_SIZE)
                .finish(),
            )
            .with_child(Shrinkable::new(1., title_element).finish())
            .finish()
    };
    match primary_line {
        TerminalPrimaryLineData::StatusText { text, .. } => {
            Text::new_inline(text, appearance.ui_font_family(), 12.)
                .with_clip(ClipConfig::ellipsis())
                .with_color(text_color.into())
                .finish()
        }
        TerminalPrimaryLineData::Text { text, font } => {
            let font_family = match font {
                TerminalPrimaryLineFont::Ui => appearance.ui_font_family(),
                TerminalPrimaryLineFont::Monospace => appearance.monospace_font_family(),
            };
            let title_el = Text::new_inline(text, font_family, 12.)
                .with_clip(ClipConfig::ellipsis())
                .with_color(text_color.into())
                .finish();
            wrap_with_error_indicator(title_el)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_terminal_metadata_line(
    terminal_view: &TerminalView,
    pane_group_id: EntityId,
    pane_id: PaneId,
    left_content: MetadataLeftContent,
    row_entrypoint: VerticalTabsChipEntrypoint,
    badge_mouse_states: &PaneRowBadgeMouseStates,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let sub_text_color = theme.sub_text_color(theme.background());

    let mut meta = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);

    // Left: Shrinkable so it clips before reaching the right-side badges.
    let left_element: Box<dyn Element> = match left_content {
        MetadataLeftContent::GitBranch(Some(branch)) if !branch.trim().is_empty() => {
            Shrinkable::new(
                1.,
                render_git_branch_text(&branch, sub_text_color, 10., appearance),
            )
            .finish()
        }
        MetadataLeftContent::WorkingDirectory(wd) if !wd.trim().is_empty() => Shrinkable::new(
            1.,
            Text::new_inline(wd, appearance.ui_font_family(), 10.)
                .with_clip(ClipConfig::start())
                .with_color(sub_text_color.into())
                .finish(),
        )
        .finish(),
        _ => Empty::new().finish(),
    };
    meta.add_child(left_element);

    // Right: wrap badges in a container with left padding equal to the inter-chip gap (4px).
    // SpaceBetween treats this padding as part of the right element's natural width, so when the
    // panel is narrow and the Shrinkable text has fully collapsed, there is still a guaranteed
    // 4px gap between the text and the first chip — matching the spacing between chips.
    if let Some(right_badges) = render_terminal_right_badges(
        terminal_view,
        pane_group_id,
        pane_id,
        row_entrypoint,
        badge_mouse_states,
        appearance,
        app,
    ) {
        meta.add_child(Container::new(right_badges).with_padding_left(4.).finish());
    }

    // Constrain to a fixed height so toggling badges on/off doesn't change the row height.
    ConstrainedBox::new(meta.finish())
        .with_height(METADATA_ROW_HEIGHT)
        .finish()
}

fn render_terminal_right_badges(
    terminal_view: &TerminalView,
    pane_group_id: EntityId,
    pane_id: PaneId,
    entrypoint: VerticalTabsChipEntrypoint,
    badge_mouse_states: &PaneRowBadgeMouseStates,
    appearance: &Appearance,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let show_diff_stats = *TabSettings::as_ref(app)
        .vertical_tabs_show_diff_stats
        .value();
    let show_pr_link = *TabSettings::as_ref(app).vertical_tabs_show_pr_link.value();

    let mut right_badges = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(4.);
    let mut has_badges = false;

    if show_diff_stats {
        if let Some(git_line_changes) = terminal_view.current_diff_line_changes(app) {
            right_badges.add_child(render_terminal_diff_stats_badge(
                &git_line_changes,
                pane_group_id,
                pane_id,
                entrypoint,
                badge_mouse_states.diff_stats.clone(),
                appearance,
            ));
            has_badges = true;
        }
    }

    if show_pr_link {
        if let Some(pull_request_url) = terminal_view.current_pull_request_url(app) {
            let label = terminal_pull_request_badge_label(&pull_request_url);
            right_badges.add_child(render_terminal_pull_request_badge(
                label,
                pull_request_url,
                entrypoint,
                badge_mouse_states.pull_request.clone(),
                appearance,
            ));
            has_badges = true;
        }
    }

    has_badges.then(|| right_badges.finish())
}

fn render_terminal_diff_stats_badge(
    git_line_changes: &GitLineChanges,
    pane_group_id: EntityId,
    pane_id: PaneId,
    entrypoint: VerticalTabsChipEntrypoint,
    mouse_state: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();

    Hoverable::new(mouse_state, move |state| {
        let bg = if state.is_hovered() {
            internal_colors::fg_overlay_2(theme)
        } else {
            internal_colors::fg_overlay_1(theme)
        };
        render_badge_container(
            render_vtab_diff_stats_content(git_line_changes, appearance),
            bg,
        )
    })
    .on_click(move |ctx, app, _| {
        send_telemetry_from_app_ctx!(
            VerticalTabsTelemetryEvent::DiffStatsChipClicked { entrypoint },
            app
        );
        let locator = PaneViewLocator {
            pane_group_id,
            pane_id,
        };
        ctx.dispatch_typed_action(WorkspaceAction::FocusPane(locator));
        ctx.dispatch_typed_action(WorkspaceAction::OpenCodeReviewPanel(locator));
    })
    .with_cursor(Cursor::PointingHand)
    .finish()
}

fn render_terminal_pull_request_badge(
    label: String,
    url: String,
    entrypoint: VerticalTabsChipEntrypoint,
    mouse_state: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();

    Hoverable::new(mouse_state, move |state| {
        let bg = if state.is_hovered() {
            internal_colors::fg_overlay_2(theme)
        } else {
            internal_colors::fg_overlay_1(theme)
        };
        render_badge_container(render_pull_request_badge_content(&label, appearance), bg)
    })
    .on_click(move |ctx, app, _| {
        send_telemetry_from_app_ctx!(
            VerticalTabsTelemetryEvent::PrChipClicked { entrypoint },
            app
        );
        ctx.dispatch_typed_action(WorkspaceAction::OpenLink(url.clone()));
    })
    .with_cursor(Cursor::PointingHand)
    .finish()
}

fn render_passive_terminal_pull_request_badge(
    label: &str,
    appearance: &Appearance,
) -> Box<dyn Element> {
    render_badge_container(
        render_pull_request_badge_content(label, appearance),
        internal_colors::fg_overlay_1(appearance.theme()),
    )
}

fn render_compact_non_terminal_title(
    title: &str,
    typed: &TypedPane<'_>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let clip_config = if matches!(typed, TypedPane::Code(_)) {
        ClipConfig::start()
    } else {
        ClipConfig::ellipsis()
    };
    Text::new_inline(title.to_string(), appearance.ui_font_family(), 12.)
        .with_clip(clip_config)
        .with_color(theme.main_text_color(theme.background()).into())
        .finish()
}

fn render_vtab_diff_stats_content(
    line_changes: &GitLineChanges,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let font_family = appearance.ui_font_family();
    let font_size = 10.;
    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

    for (index, token) in vtab_diff_stats_tokens(line_changes).iter().enumerate() {
        if index > 0 {
            row.add_child(Text::new_inline(" ", font_family, font_size).finish());
        }

        let color = if token.starts_with('+') {
            add_color(appearance)
        } else if token.starts_with('-') {
            remove_color(appearance)
        } else {
            internal_colors::neutral_6(appearance.theme())
        };

        row.add_child(
            Text::new_inline(token.clone(), font_family, font_size)
                .with_color(color)
                .with_style(Properties::default().weight(Weight::Semibold))
                .finish(),
        );
    }

    row.finish()
}

fn render_badge_container(content: Box<dyn Element>, background: ThemeFill) -> Box<dyn Element> {
    Container::new(content)
        .with_padding(Padding::uniform(1.).with_left(4.).with_right(4.))
        .with_background(background)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
        .finish()
}

fn render_pull_request_badge_content(label: &str, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let main_text_color = theme.main_text_color(theme.background());
    let sub_text_color = theme.sub_text_color(theme.background());
    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(4.)
        .with_child(
            ConstrainedBox::new(UiIcon::Github.to_warpui_icon(main_text_color).finish())
                .with_width(BADGE_ICON_SIZE)
                .with_height(BADGE_ICON_SIZE)
                .finish(),
        )
        .with_child(
            Text::new_inline(label.to_string(), appearance.ui_font_family(), 10.)
                .with_color(sub_text_color.into())
                .finish(),
        )
        .finish()
}

fn compute_tab_group_color_mode(
    tab: &TabData,
    pane_group: &PaneGroup,
    visible_pane_ids: &[PaneId],
    theme: &WarpTheme,
    app: &AppContext,
) -> TabGroupColorMode {
    // Manual override applies to the whole group.
    if !matches!(tab.selected_color, SelectedTabColor::Unset) {
        return match tab.color() {
            Some(color) => TabGroupColorMode::Uniform(
                color.to_ansi_color(&theme.terminal_colors().normal).into(),
            ),
            None => TabGroupColorMode::None,
        };
    }

    let dir_colors = TabSettings::as_ref(app)
        .directory_tab_colors
        .value()
        .clone();
    let per_pane: HashMap<PaneId, Option<AnsiColorIdentifier>> = visible_pane_ids
        .iter()
        .map(|&pane_id| {
            let color = if let Some(tv) = pane_group.terminal_view_from_pane_id(pane_id, app) {
                // Terminal pane: determine color from CWD.
                tv.as_ref(app).pwd_if_local(app).and_then(|cwd| {
                    dir_colors
                        .color_for_directory(Path::new(&cwd))
                        .and_then(|c| c.ansi_color())
                })
            } else if let Some(code_view) = pane_group.code_view_from_pane_id(pane_id, app) {
                // Code pane: determine color from the open file path using longest-prefix
                // matching against configured directories, so e.g. warp-internal/code.rs
                // inherits the color assigned to warp-internal.
                code_view
                    .as_ref(app)
                    .local_path(app)
                    .as_deref()
                    .and_then(|file_path| {
                        dir_colors
                            .color_for_directory(file_path)
                            .and_then(|c| c.ansi_color())
                    })
            } else {
                // Other non-terminal panes (notebook, workflow, etc.): fall back to the
                // cached directory color from the tab's last active terminal.
                tab.default_directory_color
            };
            (pane_id, color)
        })
        .collect();

    let has_uncolored = per_pane.values().any(|c| c.is_none());
    let mut distinct_colors: Vec<AnsiColorIdentifier> = Vec::new();
    for color in per_pane.values().flatten() {
        if !distinct_colors.contains(color) {
            distinct_colors.push(*color);
        }
    }

    // Uniform only when every pane has a color and they all match.
    let is_uniform = !has_uncolored && distinct_colors.len() == 1;

    if distinct_colors.is_empty() {
        TabGroupColorMode::None
    } else if is_uniform {
        let color = distinct_colors[0];
        TabGroupColorMode::Uniform(color.to_ansi_color(&theme.terminal_colors().normal).into())
    } else {
        let theme_map = per_pane
            .into_iter()
            .map(|(id, c)| {
                let fill = c.map(|c| c.to_ansi_color(&theme.terminal_colors().normal).into());
                (id, fill)
            })
            .collect();
        TabGroupColorMode::PerPane(theme_map)
    }
}

fn resolve_compact_subtitle(
    primary: VerticalTabsPrimaryInfo,
    subtitle_pref: VerticalTabsCompactSubtitle,
) -> VerticalTabsCompactSubtitle {
    let is_conflict = matches!(
        (primary, subtitle_pref),
        (
            VerticalTabsPrimaryInfo::Command,
            VerticalTabsCompactSubtitle::Command
        ) | (
            VerticalTabsPrimaryInfo::WorkingDirectory,
            VerticalTabsCompactSubtitle::WorkingDirectory
        ) | (
            VerticalTabsPrimaryInfo::Branch,
            VerticalTabsCompactSubtitle::Branch
        )
    );
    if is_conflict {
        default_compact_subtitle(primary)
    } else {
        subtitle_pref
    }
}

fn default_compact_subtitle(primary: VerticalTabsPrimaryInfo) -> VerticalTabsCompactSubtitle {
    match primary {
        VerticalTabsPrimaryInfo::Command => VerticalTabsCompactSubtitle::Branch,
        VerticalTabsPrimaryInfo::WorkingDirectory => VerticalTabsCompactSubtitle::Branch,
        VerticalTabsPrimaryInfo::Branch => VerticalTabsCompactSubtitle::Command,
    }
}

fn subtitle_options_for_primary(
    primary: VerticalTabsPrimaryInfo,
) -> [(VerticalTabsCompactSubtitle, &'static str); 2] {
    match primary {
        VerticalTabsPrimaryInfo::Command => [
            (VerticalTabsCompactSubtitle::Branch, "Branch"),
            (
                VerticalTabsCompactSubtitle::WorkingDirectory,
                "Working Directory",
            ),
        ],
        VerticalTabsPrimaryInfo::WorkingDirectory => [
            (VerticalTabsCompactSubtitle::Branch, "Branch"),
            (
                VerticalTabsCompactSubtitle::Command,
                "Command / Conversation",
            ),
        ],
        VerticalTabsPrimaryInfo::Branch => [
            (
                VerticalTabsCompactSubtitle::Command,
                "Command / Conversation",
            ),
            (
                VerticalTabsCompactSubtitle::WorkingDirectory,
                "Working Directory",
            ),
        ],
    }
}

pub(super) fn render_settings_popup(
    state: &VerticalTabsPanelState,
    app: &AppContext,
) -> Box<dyn Element> {
    const SETTINGS_POPUP_CORNER_RADIUS: f32 = 6.;
    const SETTINGS_POPUP_MENU_ITEM_FONT_SIZE: f32 = 12.;

    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let current_granularity = *TabSettings::as_ref(app)
        .vertical_tabs_display_granularity
        .value();
    let current_tab_item_mode = *TabSettings::as_ref(app).vertical_tabs_tab_item_mode.value();
    let current_mode = *TabSettings::as_ref(app).vertical_tabs_view_mode.value();
    let current_primary_info = *TabSettings::as_ref(app).vertical_tabs_primary_info.value();
    let current_subtitle = resolve_compact_subtitle(
        current_primary_info,
        *TabSettings::as_ref(app)
            .vertical_tabs_compact_subtitle
            .value(),
    );
    let show_pr_link = *TabSettings::as_ref(app).vertical_tabs_show_pr_link.value();
    let show_diff_stats = *TabSettings::as_ref(app)
        .vertical_tabs_show_diff_stats
        .value();
    let show_details_on_hover = *TabSettings::as_ref(app)
        .vertical_tabs_show_details_on_hover
        .value();
    let show_tab_item_section = matches!(current_granularity, VerticalTabsDisplayGranularity::Tabs)
        && FeatureFlag::VerticalTabsSummaryMode.is_enabled();
    let show_focused_session_controls = !matches!(
        resolve_vertical_tabs_mode(app),
        VerticalTabsResolvedMode::Summary
    );
    let sub_text = theme.sub_text_color(theme.background());
    let view_as_header = Container::new(
        Text::new_inline(
            "View as".to_string(),
            appearance.ui_font_family(),
            SETTINGS_POPUP_MENU_ITEM_FONT_SIZE,
        )
        .with_color(sub_text.into())
        .finish(),
    )
    .with_horizontal_padding(16.)
    .with_margin_bottom(4.)
    .finish();

    let view_as_segmented_control = Container::new(
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Expanded::new(
                    1.,
                    render_popup_text_segment(
                        "Panes",
                        matches!(current_granularity, VerticalTabsDisplayGranularity::Panes),
                        state.panes_segment_mouse_state.clone(),
                        VerticalTabsDisplayGranularity::Panes,
                        appearance,
                        theme,
                    ),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    render_popup_text_segment(
                        "Tabs",
                        matches!(current_granularity, VerticalTabsDisplayGranularity::Tabs),
                        state.tabs_segment_mouse_state.clone(),
                        VerticalTabsDisplayGranularity::Tabs,
                        appearance,
                        theme,
                    ),
                )
                .finish(),
            )
            .finish(),
    )
    .with_uniform_padding(4.)
    .with_background(internal_colors::fg_overlay_2(theme))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
        SETTINGS_POPUP_CORNER_RADIUS,
    )))
    .finish();

    let view_as_segmented_control_row = Container::new(view_as_segmented_control)
        .with_horizontal_padding(16.)
        .with_padding_bottom(4.)
        .finish();

    let tab_item_header = Container::new(
        Text::new_inline(
            "Tab item".to_string(),
            appearance.ui_font_family(),
            SETTINGS_POPUP_MENU_ITEM_FONT_SIZE,
        )
        .with_color(sub_text.into())
        .finish(),
    )
    .with_horizontal_padding(16.)
    .with_margin_bottom(4.)
    .finish();

    let focused_session_option = render_tab_item_mode_option(
        "Focused session",
        matches!(
            current_tab_item_mode,
            VerticalTabsTabItemMode::FocusedSession
        ),
        state.focused_session_option_mouse_state.clone(),
        VerticalTabsTabItemMode::FocusedSession,
        appearance,
        theme,
    );

    let summary_option = if FeatureFlag::VerticalTabsSummaryMode.is_enabled() {
        Some(render_tab_item_mode_option(
            "Summary",
            matches!(current_tab_item_mode, VerticalTabsTabItemMode::Summary),
            state.summary_option_mouse_state.clone(),
            VerticalTabsTabItemMode::Summary,
            appearance,
            theme,
        ))
    } else {
        None
    };

    let density_header = Container::new(
        Text::new_inline(
            "Density".to_string(),
            appearance.ui_font_family(),
            SETTINGS_POPUP_MENU_ITEM_FONT_SIZE,
        )
        .with_color(sub_text.into())
        .finish(),
    )
    .with_horizontal_padding(16.)
    .with_margin_bottom(4.)
    .finish();

    // Segmented control row (compact/expanded toggle)
    // Segmented control row (compact/expanded toggle) — always at the top
    let segmented_control = Container::new(
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Expanded::new(
                    1.,
                    render_popup_segment(
                        WarpIcon::Menu01,
                        matches!(current_mode, VerticalTabsViewMode::Compact),
                        state.compact_segment_mouse_state.clone(),
                        VerticalTabsViewMode::Compact,
                        theme,
                        sub_text,
                    ),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    render_popup_segment(
                        WarpIcon::Grid,
                        matches!(current_mode, VerticalTabsViewMode::Expanded),
                        state.expanded_segment_mouse_state.clone(),
                        VerticalTabsViewMode::Expanded,
                        theme,
                        sub_text,
                    ),
                )
                .finish(),
            )
            .finish(),
    )
    .with_uniform_padding(4.)
    .with_background(internal_colors::fg_overlay_2(theme))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
        SETTINGS_POPUP_CORNER_RADIUS,
    )))
    .finish();

    let segmented_control_row = Container::new(segmented_control)
        .with_horizontal_padding(16.)
        .with_padding_bottom(4.)
        .finish();

    // Divider between toggle and "Pane title as" section
    let make_divider = |theme: &WarpTheme| {
        Container::new(
            ConstrainedBox::new(
                Container::new(Empty::new().finish())
                    .with_background(internal_colors::fg_overlay_2(theme))
                    .finish(),
            )
            .with_height(1.)
            .finish(),
        )
        .with_margin_top(8.)
        .with_margin_bottom(8.)
        .finish()
    };

    let pane_title_header = Container::new(
        Text::new_inline(
            "Pane title as".to_string(),
            appearance.ui_font_family(),
            SETTINGS_POPUP_MENU_ITEM_FONT_SIZE,
        )
        .with_color(sub_text.into())
        .finish(),
    )
    .with_horizontal_padding(16.)
    .with_margin_bottom(4.)
    .finish();

    let command_option = render_primary_info_option(
        "Command / Conversation",
        matches!(current_primary_info, VerticalTabsPrimaryInfo::Command),
        state.command_option_mouse_state.clone(),
        VerticalTabsPrimaryInfo::Command,
        appearance,
        theme,
    );

    let directory_option = render_primary_info_option(
        "Working Directory",
        matches!(
            current_primary_info,
            VerticalTabsPrimaryInfo::WorkingDirectory
        ),
        state.directory_option_mouse_state.clone(),
        VerticalTabsPrimaryInfo::WorkingDirectory,
        appearance,
        theme,
    );

    let branch_option = render_primary_info_option(
        "Branch",
        matches!(current_primary_info, VerticalTabsPrimaryInfo::Branch),
        state.branch_option_mouse_state.clone(),
        VerticalTabsPrimaryInfo::Branch,
        appearance,
        theme,
    );

    // Assemble popup — top-level display granularity first, then density and pane-row sections.
    let mut popup_col = Flex::column()
        .with_main_axis_size(MainAxisSize::Min)
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    popup_col.add_child(view_as_header);
    popup_col.add_child(view_as_segmented_control_row);
    if show_tab_item_section {
        popup_col.add_child(make_divider(theme));
        popup_col.add_child(tab_item_header);
        popup_col.add_child(focused_session_option);
        if let Some(summary_option) = summary_option {
            popup_col.add_child(summary_option);
        }
    }

    if show_focused_session_controls {
        popup_col.add_child(make_divider(theme));
        popup_col.add_child(density_header);
        popup_col.add_child(segmented_control_row);
        popup_col.add_child(make_divider(theme));
        popup_col.add_child(pane_title_header);
        popup_col.add_child(command_option);
        popup_col.add_child(directory_option);
        popup_col.add_child(branch_option);

        if matches!(current_mode, VerticalTabsViewMode::Compact) {
            popup_col.add_child(make_divider(theme));

            let subtitle_header = Container::new(
                Text::new_inline(
                    "Additional metadata".to_string(),
                    appearance.ui_font_family(),
                    SETTINGS_POPUP_MENU_ITEM_FONT_SIZE,
                )
                .with_color(sub_text.into())
                .finish(),
            )
            .with_horizontal_padding(16.)
            .with_margin_bottom(4.)
            .finish();
            popup_col.add_child(subtitle_header);

            let options = subtitle_options_for_primary(current_primary_info);
            let mouse_states = [
                state.subtitle_option_1_mouse_state.clone(),
                state.subtitle_option_2_mouse_state.clone(),
            ];
            for (i, (value, label)) in options.iter().enumerate() {
                popup_col.add_child(render_compact_subtitle_option(
                    label,
                    current_subtitle == *value,
                    mouse_states[i].clone(),
                    *value,
                    appearance,
                    theme,
                ));
            }
        }

        if matches!(current_mode, VerticalTabsViewMode::Expanded) {
            popup_col.add_child(make_divider(theme));

            let show_header = Container::new(
                Text::new_inline(
                    "Show".to_string(),
                    appearance.ui_font_family(),
                    SETTINGS_POPUP_MENU_ITEM_FONT_SIZE,
                )
                .with_color(sub_text.into())
                .finish(),
            )
            .with_horizontal_padding(16.)
            .with_margin_bottom(4.)
            .finish();
            popup_col.add_child(show_header);
            let pr_validation_suppressed = SessionSettings::as_ref(app)
                .github_pr_chip_default_validation
                .is_suppressed();
            let pr_link_info_tooltip = if show_pr_link && pr_validation_suppressed {
                Some(ShowToggleInfoTooltip {
                    mouse_state: state.show_pr_link_info_tooltip_mouse_state.clone(),
                    tooltip_text: "Requires the GitHub CLI to be installed and authenticated",
                })
            } else {
                None
            };

            popup_col.add_child(render_show_toggle_option(
                "PR link",
                show_pr_link,
                state.show_pr_link_mouse_state.clone(),
                WorkspaceAction::ToggleVerticalTabsShowPrLink,
                pr_link_info_tooltip,
                appearance,
                theme,
            ));
            popup_col.add_child(render_show_toggle_option(
                "Diff stats",
                show_diff_stats,
                state.show_diff_stats_mouse_state.clone(),
                WorkspaceAction::ToggleVerticalTabsShowDiffStats,
                None,
                appearance,
                theme,
            ));
        }
    }
    popup_col.add_child(make_divider(theme));

    popup_col.add_child(render_show_toggle_option(
        "Show details on hover",
        show_details_on_hover,
        state.show_details_on_hover_mouse_state.clone(),
        WorkspaceAction::ToggleVerticalTabsShowDetailsOnHover,
        None,
        appearance,
        theme,
    ));
    EventHandler::new(
        ConstrainedBox::new(
            Container::new(popup_col.finish())
                .with_vertical_padding(8.)
                .with_background(internal_colors::neutral_1(theme))
                .with_border(Border::all(1.).with_border_fill(internal_colors::fg_overlay_1(theme)))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                    SETTINGS_POPUP_CORNER_RADIUS,
                )))
                .with_drop_shadow(DropShadow::default())
                .finish(),
        )
        .with_width(200.)
        .finish(),
    )
    .on_left_mouse_down(|_, _, _| DispatchEventResult::StopPropagation)
    .finish()
}

fn render_compact_subtitle_option(
    label: &str,
    is_selected: bool,
    mouse_state: MouseStateHandle,
    value: VerticalTabsCompactSubtitle,
    appearance: &Appearance,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    const ICON_SIZE: f32 = 16.;
    const FONT_SIZE: f32 = 12.;
    const GAP: f32 = 8.;

    let label = label.to_string();
    let main_text = theme.main_text_color(theme.background());
    Hoverable::new(mouse_state, move |hover_state| {
        let check_icon: Box<dyn Element> = if is_selected {
            ConstrainedBox::new(WarpIcon::Check.to_warpui_icon(main_text).finish())
                .with_width(ICON_SIZE)
                .with_height(ICON_SIZE)
                .finish()
        } else {
            ConstrainedBox::new(Empty::new().finish())
                .with_width(ICON_SIZE)
                .with_height(ICON_SIZE)
                .finish()
        };

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(GAP)
            .with_child(check_icon)
            .with_child(
                Text::new_inline(label.clone(), appearance.ui_font_family(), FONT_SIZE)
                    .with_color(main_text.into())
                    .finish(),
            )
            .finish();

        let mut container = Container::new(row)
            .with_horizontal_padding(16.)
            .with_vertical_padding(2.);
        if hover_state.is_hovered() {
            container = container.with_background(internal_colors::fg_overlay_1(theme));
        }
        container.finish()
    })
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(WorkspaceAction::SetVerticalTabsCompactSubtitle(value));
    })
    .with_cursor(Cursor::PointingHand)
    .finish()
}

fn render_tab_item_mode_option(
    label: &str,
    is_selected: bool,
    mouse_state: MouseStateHandle,
    value: VerticalTabsTabItemMode,
    appearance: &Appearance,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    const ICON_SIZE: f32 = 16.;
    const FONT_SIZE: f32 = 12.;
    const GAP: f32 = 8.;

    let label = label.to_string();
    let main_text = theme.main_text_color(theme.background());
    Hoverable::new(mouse_state, move |hover_state| {
        let check_icon: Box<dyn Element> = if is_selected {
            ConstrainedBox::new(WarpIcon::Check.to_warpui_icon(main_text).finish())
                .with_width(ICON_SIZE)
                .with_height(ICON_SIZE)
                .finish()
        } else {
            ConstrainedBox::new(Empty::new().finish())
                .with_width(ICON_SIZE)
                .with_height(ICON_SIZE)
                .finish()
        };

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(GAP)
            .with_child(check_icon)
            .with_child(
                Text::new_inline(label.clone(), appearance.ui_font_family(), FONT_SIZE)
                    .with_color(main_text.into())
                    .finish(),
            )
            .finish();

        let mut container = Container::new(row)
            .with_horizontal_padding(16.)
            .with_vertical_padding(2.);
        if hover_state.is_hovered() {
            container = container.with_background(internal_colors::fg_overlay_1(theme));
        }
        container.finish()
    })
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(WorkspaceAction::SetVerticalTabsTabItemMode(value));
    })
    .with_cursor(Cursor::PointingHand)
    .finish()
}

fn render_primary_info_option(
    label: &str,
    is_selected: bool,
    mouse_state: MouseStateHandle,
    value: VerticalTabsPrimaryInfo,
    appearance: &Appearance,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    const ICON_SIZE: f32 = 16.;
    const FONT_SIZE: f32 = 12.;
    const GAP: f32 = 8.;

    let label = label.to_string();
    let main_text = theme.main_text_color(theme.background());
    Hoverable::new(mouse_state, move |hover_state| {
        let check_icon: Box<dyn Element> = if is_selected {
            ConstrainedBox::new(WarpIcon::Check.to_warpui_icon(main_text).finish())
                .with_width(ICON_SIZE)
                .with_height(ICON_SIZE)
                .finish()
        } else {
            ConstrainedBox::new(Empty::new().finish())
                .with_width(ICON_SIZE)
                .with_height(ICON_SIZE)
                .finish()
        };

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(GAP)
            .with_child(check_icon)
            .with_child(
                Text::new_inline(label.clone(), appearance.ui_font_family(), FONT_SIZE)
                    .with_color(main_text.into())
                    .finish(),
            )
            .finish();

        let mut container = Container::new(row)
            .with_horizontal_padding(16.)
            .with_vertical_padding(2.);
        if hover_state.is_hovered() {
            container = container.with_background(internal_colors::fg_overlay_1(theme));
        }
        container.finish()
    })
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(WorkspaceAction::SetVerticalTabsPrimaryInfo(value));
    })
    .with_cursor(Cursor::PointingHand)
    .finish()
}

struct ShowToggleInfoTooltip {
    mouse_state: MouseStateHandle,
    tooltip_text: &'static str,
}

fn render_show_toggle_option(
    label: &str,
    is_enabled: bool,
    mouse_state: MouseStateHandle,
    action: WorkspaceAction,
    info_tooltip: Option<ShowToggleInfoTooltip>,
    appearance: &Appearance,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    const ICON_SIZE: f32 = 16.;
    const FONT_SIZE: f32 = 12.;
    const GAP: f32 = 8.;
    const INFO_ICON_SIZE: f32 = 12.;
    const INFO_GAP: f32 = 4.;

    let label = label.to_string();
    let main_text = theme.main_text_color(theme.background());
    let info_color = theme.sub_text_color(theme.background());
    let ui_builder = appearance.ui_builder().clone();

    let info_mouse_state = info_tooltip.as_ref().map(|t| t.mouse_state.clone());
    let info_tooltip_text = info_tooltip.as_ref().map(|t| t.tooltip_text.to_string());

    Hoverable::new(mouse_state, move |hover_state| {
        let check_icon: Box<dyn Element> = if is_enabled {
            ConstrainedBox::new(WarpIcon::Check.to_warpui_icon(main_text).finish())
                .with_width(ICON_SIZE)
                .with_height(ICON_SIZE)
                .finish()
        } else {
            ConstrainedBox::new(Empty::new().finish())
                .with_width(ICON_SIZE)
                .with_height(ICON_SIZE)
                .finish()
        };

        let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        row.add_child(Container::new(check_icon).with_margin_right(GAP).finish());
        row.add_child(
            Text::new_inline(label.clone(), appearance.ui_font_family(), FONT_SIZE)
                .with_color(main_text.into())
                .finish(),
        );
        if let (Some(info_ms), Some(info_text)) =
            (info_mouse_state.clone(), info_tooltip_text.clone())
        {
            let builder = ui_builder.clone();
            let info_icon = Hoverable::new(info_ms, move |info_hover| {
                let icon = ConstrainedBox::new(UiIcon::Info.to_warpui_icon(info_color).finish())
                    .with_width(INFO_ICON_SIZE)
                    .with_height(INFO_ICON_SIZE)
                    .finish();

                if info_hover.is_hovered() {
                    let tooltip = builder.tool_tip(info_text.clone()).build().finish();
                    let mut stack = Stack::new().with_child(icon);
                    stack.add_positioned_overlay_child(
                        tooltip,
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., -4.),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::TopMiddle,
                            ChildAnchor::BottomMiddle,
                        ),
                    );
                    stack.finish()
                } else {
                    icon
                }
            })
            .finish();
            row.add_child(
                Container::new(info_icon)
                    .with_padding_left(INFO_GAP)
                    .finish(),
            );
        }
        let row = row.finish();

        let mut container = Container::new(row)
            .with_horizontal_padding(16.)
            .with_vertical_padding(2.);
        if hover_state.is_hovered() {
            container = container.with_background(internal_colors::fg_overlay_1(theme));
        }
        container.finish()
    })
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(action.clone());
    })
    .with_cursor(Cursor::PointingHand)
    .finish()
}

fn render_popup_segment(
    icon: WarpIcon,
    is_selected: bool,
    mouse_state: MouseStateHandle,
    mode: VerticalTabsViewMode,
    theme: &WarpTheme,
    icon_color: WarpThemeFill,
) -> Box<dyn Element> {
    Hoverable::new(mouse_state, move |hover_state| {
        let background = if is_selected {
            internal_colors::fg_overlay_3(theme)
        } else if hover_state.is_hovered() {
            internal_colors::fg_overlay_1(theme)
        } else {
            ThemeFill::Solid(ColorU::transparent_black())
        };

        Container::new(
            Align::new(
                ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
                    .with_width(COMPACT_ICON_SIZE)
                    .with_height(COMPACT_ICON_SIZE)
                    .finish(),
            )
            .finish(),
        )
        .with_background(background)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_vertical_padding(2.)
        .finish()
    })
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(WorkspaceAction::SetVerticalTabsViewMode(mode));
    })
    .with_cursor(Cursor::PointingHand)
    .finish()
}

fn render_popup_text_segment(
    label: &str,
    is_selected: bool,
    mouse_state: MouseStateHandle,
    granularity: VerticalTabsDisplayGranularity,
    appearance: &Appearance,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    let label = label.to_string();
    let main_text = theme.main_text_color(theme.background());
    let sub_text = theme.sub_text_color(theme.background());
    Hoverable::new(mouse_state, move |hover_state| {
        let background = if is_selected {
            internal_colors::fg_overlay_3(theme)
        } else if hover_state.is_hovered() {
            internal_colors::fg_overlay_1(theme)
        } else {
            ThemeFill::Solid(ColorU::transparent_black())
        };

        Container::new(
            Align::new(
                Text::new_inline(label.clone(), appearance.ui_font_family(), 14.)
                    .with_color(if is_selected { main_text } else { sub_text }.into())
                    .finish(),
            )
            .finish(),
        )
        .with_background(background)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_vertical_padding(2.)
        .finish()
    })
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(WorkspaceAction::SetVerticalTabsDisplayGranularity(
            granularity,
        ));
    })
    .with_cursor(Cursor::PointingHand)
    .finish()
}

fn pane_ids_for_display_granularity(
    visible_pane_ids: &[PaneId],
    focused_pane_id: PaneId,
    granularity: VerticalTabsDisplayGranularity,
) -> Vec<PaneId> {
    match granularity {
        VerticalTabsDisplayGranularity::Panes => visible_pane_ids.to_vec(),
        VerticalTabsDisplayGranularity::Tabs => visible_pane_ids
            .iter()
            .copied()
            .find(|pane_id| *pane_id == focused_pane_id)
            .or_else(|| visible_pane_ids.first().copied())
            .into_iter()
            .collect(),
    }
}

fn detail_sidecar_offset_and_max_height(
    anchor_position_id: &str,
    side: super::PanelPosition,
    window_id: WindowId,
    app: &AppContext,
) -> (
    pathfinder_geometry::vector::Vector2F,
    f32,
    f32,
    PositionedElementOffsetBounds,
    PositionedElementAnchor,
    ChildAnchor,
) {
    const DETAIL_SIDECAR_MAX_HEIGHT: f32 = 420.;
    const DETAIL_SIDECAR_HORIZONTAL_GAP: f32 = 12.;
    const DETAIL_SIDECAR_WINDOW_MARGIN: f32 = 16.;

    // When the panel is on the left, the sidecar opens to the right and vice versa.
    let (top_anchors, bottom_anchors, gap_x) = match side {
        super::PanelPosition::Left => (
            (PositionedElementAnchor::TopRight, ChildAnchor::TopLeft),
            (
                PositionedElementAnchor::BottomRight,
                ChildAnchor::BottomLeft,
            ),
            DETAIL_SIDECAR_HORIZONTAL_GAP,
        ),
        super::PanelPosition::Right => (
            (PositionedElementAnchor::TopLeft, ChildAnchor::TopRight),
            (
                PositionedElementAnchor::BottomLeft,
                ChildAnchor::BottomRight,
            ),
            -DETAIL_SIDECAR_HORIZONTAL_GAP,
        ),
    };

    let Some(window) = app.windows().platform_window(window_id) else {
        return (
            vec2f(gap_x, 0.),
            DETAIL_SIDECAR_MAX_HEIGHT,
            DETAIL_SIDECAR_DEFAULT_WIDTH,
            PositionedElementOffsetBounds::WindowBySize,
            top_anchors.0,
            top_anchors.1,
        );
    };
    let max_height = (window.size().y() - DETAIL_SIDECAR_WINDOW_MARGIN * 2.)
        .clamp(0., DETAIL_SIDECAR_MAX_HEIGHT);
    let previous_sidecar_height = app
        .element_position_by_id_at_last_frame(window_id, VERTICAL_TABS_DETAIL_SIDECAR_POSITION_ID)
        .map(|sidecar_rect| sidecar_rect.height())
        .unwrap_or(max_height)
        .min(max_height);

    let Some(anchor_rect) = app.element_position_by_id_at_last_frame(window_id, anchor_position_id)
    else {
        return (
            vec2f(gap_x, 0.),
            max_height,
            DETAIL_SIDECAR_DEFAULT_WIDTH,
            PositionedElementOffsetBounds::WindowBySize,
            top_anchors.0,
            top_anchors.1,
        );
    };
    let window_width = window.size().x();
    let available_width =
        (window_width - anchor_rect.max_x() - DETAIL_SIDECAR_HORIZONTAL_GAP).max(0.);
    let (width, positioned_bounds) = detail_sidecar_width_and_bounds(available_width);

    let window_bottom = window.size().y() - DETAIL_SIDECAR_WINDOW_MARGIN;
    let should_anchor_to_bottom = anchor_rect.min_y() + previous_sidecar_height > window_bottom;

    if should_anchor_to_bottom {
        let min_bottom = DETAIL_SIDECAR_WINDOW_MARGIN + previous_sidecar_height;
        let offset_y = (min_bottom - anchor_rect.max_y()).max(0.);
        (
            vec2f(gap_x, offset_y),
            max_height,
            width,
            positioned_bounds,
            bottom_anchors.0,
            bottom_anchors.1,
        )
    } else {
        let offset_y = (DETAIL_SIDECAR_WINDOW_MARGIN - anchor_rect.min_y()).max(0.);
        (
            vec2f(gap_x, offset_y),
            max_height,
            width,
            positioned_bounds,
            top_anchors.0,
            top_anchors.1,
        )
    }
}

fn detail_sidecar_width_and_bounds(available_width: f32) -> (f32, PositionedElementOffsetBounds) {
    if available_width >= DETAIL_SIDECAR_DEFAULT_WIDTH {
        (
            DETAIL_SIDECAR_DEFAULT_WIDTH,
            PositionedElementOffsetBounds::WindowBySize,
        )
    } else if available_width >= DETAIL_SIDECAR_MIN_WIDTH {
        (available_width, PositionedElementOffsetBounds::WindowBySize)
    } else {
        (
            DETAIL_SIDECAR_MIN_WIDTH,
            PositionedElementOffsetBounds::Unbounded,
        )
    }
}

struct DetailSidecarTextColors {
    main: WarpThemeFill,
    sub: WarpThemeFill,
    disabled: WarpThemeFill,
}

fn detail_sidecar_background(theme: &WarpTheme) -> ColorU {
    theme
        .background()
        .blend(&internal_colors::fg_overlay_2(theme))
        .into_solid()
}

fn detail_sidecar_border_fill(theme: &WarpTheme) -> ThemeFill {
    theme
        .background()
        .blend(&internal_colors::fg_overlay_4(theme))
}

fn detail_sidecar_text_colors(theme: &WarpTheme) -> DetailSidecarTextColors {
    let bg = ThemeFill::Solid(detail_sidecar_background(theme));
    DetailSidecarTextColors {
        main: theme.main_text_color(bg),
        sub: theme.sub_text_color(bg),
        disabled: theme.disabled_text_color(bg),
    }
}

fn render_detail_badge(
    label: impl Into<String>,
    icon: Option<Box<dyn Element>>,
    background: Option<ThemeFill>,
    text_color: WarpThemeFill,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut content = Flex::row()
        .with_main_axis_size(MainAxisSize::Min)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(4.);
    if let Some(icon) = icon {
        content.add_child(
            ConstrainedBox::new(icon)
                .with_width(12.)
                .with_height(12.)
                .finish(),
        );
    }
    content.add_child(
        Text::new_inline(label.into(), appearance.ui_font_family(), 10.)
            .with_color(text_color.into())
            .finish(),
    );

    let mut badge = Container::new(content.finish())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)));
    if let Some(background) = background {
        badge = badge
            .with_padding(Padding::uniform(2.).with_left(6.).with_right(6.))
            .with_background(background);
    }
    badge.finish()
}

fn render_detail_status_pill(
    status: &ConversationStatus,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let (icon, color) = status.status_icon_and_color(theme, StatusColorStyle::Standard);
    Container::new(
        Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.)
            .with_child(
                ConstrainedBox::new(icon.to_warpui_icon(WarpThemeFill::Solid(color)).finish())
                    .with_width(12.)
                    .with_height(12.)
                    .finish(),
            )
            .with_child(
                Text::new_inline(status.to_string(), appearance.ui_font_family(), 10.)
                    .with_color(WarpThemeFill::Solid(color).into())
                    .finish(),
            )
            .finish(),
    )
    .with_padding(Padding::uniform(2.).with_left(4.).with_right(4.))
    .with_background(ThemeFill::Solid(coloru_with_opacity(color, 10)))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
    .finish()
}

fn render_detail_wrapping_text(
    text: impl Into<String>,
    font_size: f32,
    color: WarpThemeFill,
    style: Option<Properties>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut text = Text::new(text.into(), appearance.ui_font_family(), font_size)
        .soft_wrap(true)
        .with_color(color.into());
    if let Some(style) = style {
        text = text.with_style(style);
    }
    text.finish()
}

fn render_terminal_detail_primary_line(
    primary_line: &TerminalPrimaryLineData,
    color: WarpThemeFill,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let font_family = match primary_line {
        TerminalPrimaryLineData::StatusText { .. } => appearance.ui_font_family(),
        TerminalPrimaryLineData::Text { font, .. } => match font {
            TerminalPrimaryLineFont::Ui => appearance.ui_font_family(),
            TerminalPrimaryLineFont::Monospace => appearance.monospace_font_family(),
        },
    };

    Text::new(primary_line.text().to_string(), font_family, 12.)
        .soft_wrap(true)
        .with_color(color.into())
        .finish()
}

fn detail_pane_props<'a>(
    state: &VerticalTabsPanelState,
    workspace: &Workspace,
    pane_group: &'a PaneGroup,
    pane_group_id: EntityId,
    pane_id: PaneId,
    app: &AppContext,
) -> Option<PaneProps<'a>> {
    let badge_mouse_states = state
        .detail_pane_badge_mouse_states
        .borrow_mut()
        .entry(pane_id)
        .or_default()
        .clone();
    PaneProps::new(
        pane_group,
        pane_id,
        pane_group_id,
        false,
        PaneRowState {
            mouse_state: MouseStateHandle::default(),
            title_mouse_state: None,
            pane_color: None,
            badge_mouse_states,
        },
        state.detail_hover_state(workspace.window_id),
        *TabSettings::as_ref(app)
            .vertical_tabs_display_granularity
            .value(),
        false,
        None,
        None,
        None,
        false,
        None,
        false,
        None,
        app,
    )
}

fn render_terminal_detail_section(
    props: &PaneProps<'_>,
    terminal_view: &TerminalView,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let text_colors = detail_sidecar_text_colors(theme);
    let working_directory = resolved_terminal_working_directory(terminal_view, app);
    let git_branch = terminal_view.current_git_branch(app);
    let cli_agent_session = CLIAgentSessionsModel::as_ref(app).session(terminal_view.id());
    let agent_text = terminal_agent_text(terminal_view, app);
    let (conversation_display_title, cli_agent_title) =
        preferred_agent_tab_titles(&agent_text, agent_tab_text_preference(app));
    let kind_label = terminal_kind_badge_label(agent_text.is_oz_agent, agent_text.cli_agent);
    let status = if let Some(session) =
        cli_agent_session.filter(|s| s.listener.is_some() && agent_supports_rich_status(&s.agent))
    {
        Some(session.status.to_conversation_status())
    } else if agent_text.is_oz_agent {
        terminal_view.selected_conversation_status_for_display(app)
    } else {
        None
    };

    let title_text = terminal_view.terminal_title_from_shell();
    let primary_line = terminal_primary_line_data(
        terminal_view.is_long_running_and_user_controlled(),
        conversation_display_title,
        cli_agent_title,
        title_text.as_str(),
        working_directory.as_deref().unwrap_or(title_text.as_str()),
        terminal_title_fallback_font(&agent_text),
        terminal_view.last_completed_command_text(),
    );

    let mut section = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_spacing(DETAIL_SIDECAR_SECTION_GAP);

    if let Some(status) = status.as_ref() {
        section.add_child(render_detail_status_pill(status, appearance));
    }
    if let Some(working_directory) = working_directory.filter(|wd| !wd.trim().is_empty()) {
        section.add_child(render_detail_wrapping_text(
            working_directory,
            12.,
            text_colors.main,
            None,
            appearance,
        ));
    }
    if let Some(branch) = git_branch.filter(|branch| !branch.trim().is_empty()) {
        section.add_child(render_git_branch_text(
            &branch,
            text_colors.main,
            12.,
            appearance,
        ));
    }
    section.add_child(render_terminal_detail_primary_line(
        &primary_line,
        text_colors.sub,
        appearance,
    ));

    let mut metadata_row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);
    metadata_row.add_child(render_detail_badge(
        kind_label,
        Some(render_detail_kind_badge_icon(props, appearance, app)),
        None,
        text_colors.disabled,
        appearance,
    ));

    let mut right_badges = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(4.);
    let mut has_right_badges = false;
    if let Some(git_line_changes) = terminal_view.current_diff_line_changes(app) {
        right_badges.add_child(render_terminal_diff_stats_badge(
            &git_line_changes,
            props.pane_group_id,
            props.pane_id,
            VerticalTabsChipEntrypoint::DetailsSidecar,
            props.badge_mouse_states.diff_stats.clone(),
            appearance,
        ));
        has_right_badges = true;
    }
    if let Some(pull_request_url) = terminal_view.current_pull_request_url(app) {
        right_badges.add_child(render_terminal_pull_request_badge(
            terminal_pull_request_badge_label(&pull_request_url),
            pull_request_url,
            VerticalTabsChipEntrypoint::DetailsSidecar,
            props.badge_mouse_states.pull_request.clone(),
            appearance,
        ));
        has_right_badges = true;
    }
    if has_right_badges {
        metadata_row.add_child(right_badges.finish());
    }
    section.add_child(metadata_row.finish());

    Container::new(section.finish())
        .with_padding(Padding::uniform(DETAIL_SIDECAR_SECTION_PADDING))
        .finish()
}

fn render_code_detail_section(
    props: &PaneProps<'_>,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let text_colors = detail_sidecar_text_colors(theme);
    let TypedPane::Code(code_pane) = &props.typed else {
        return Empty::new().finish();
    };
    let code_view = code_pane.file_view(app);
    let code_view = code_view.as_ref(app);
    let extra_open_tabs = code_view.tab_count().saturating_sub(1);

    let mut section = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_spacing(DETAIL_SIDECAR_SECTION_GAP);
    section.add_child(render_detail_wrapping_text(
        props.title.clone(),
        12.,
        text_colors.main,
        None,
        appearance,
    ));

    if !props.subtitle.trim().is_empty() {
        section.add_child(render_detail_wrapping_text(
            props.subtitle.clone(),
            12.,
            text_colors.sub,
            None,
            appearance,
        ));
    }

    if extra_open_tabs > 0 {
        section.add_child(render_detail_wrapping_text(
            format!("and {extra_open_tabs} more"),
            12.,
            text_colors.sub,
            None,
            appearance,
        ));
    }

    if let Some(language_name) = code_detail_kind_label(&props.title) {
        let mut metadata_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        metadata_row.add_child(render_detail_badge(
            language_name,
            Some(render_detail_kind_badge_icon(props, appearance, app)),
            None,
            text_colors.disabled,
            appearance,
        ));
        if let Some(badge) = props.typed.badge(app) {
            metadata_row.add_child(render_detail_badge(
                badge,
                None,
                Some(internal_colors::fg_overlay_1(theme)),
                text_colors.sub,
                appearance,
            ));
        }
        section.add_child(metadata_row.finish());
    }

    Container::new(section.finish())
        .with_padding(Padding::uniform(DETAIL_SIDECAR_SECTION_PADDING))
        .finish()
}

fn render_warp_drive_object_detail_section(
    props: &PaneProps<'_>,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let text_colors = detail_sidecar_text_colors(theme);

    let mut section = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_spacing(DETAIL_SIDECAR_SECTION_GAP);
    section.add_child(render_detail_wrapping_text(
        props.title.clone(),
        12.,
        text_colors.main,
        None,
        appearance,
    ));
    section.add_child(render_detail_badge(
        props.typed.kind_label(),
        Some(render_detail_kind_badge_icon(props, appearance, app)),
        None,
        text_colors.disabled,
        appearance,
    ));

    Container::new(section.finish())
        .with_padding(Padding::uniform(DETAIL_SIDECAR_SECTION_PADDING))
        .finish()
}

fn code_detail_kind_label(file_name: &str) -> Option<String> {
    language_by_filename(Path::new(file_name)).map(|language| language.display_name().to_string())
}

fn typed_pane_warp_drive_object_type(typed: &TypedPane<'_>) -> Option<DriveObjectType> {
    match typed {
        TypedPane::Notebook { is_plan } => Some(DriveObjectType::Notebook {
            is_ai_document: *is_plan,
        }),
        TypedPane::Workflow { is_ai_prompt: true } => Some(DriveObjectType::AgentModeWorkflow),
        TypedPane::Workflow {
            is_ai_prompt: false,
        } => Some(DriveObjectType::Workflow),
        TypedPane::EnvVarCollection => Some(DriveObjectType::EnvVarCollection),
        TypedPane::AIFact => Some(DriveObjectType::AIFact),
        TypedPane::AIDocument => Some(DriveObjectType::Notebook {
            is_ai_document: true,
        }),
        TypedPane::Terminal(_)
        | TypedPane::Code(_)
        | TypedPane::CodeDiff
        | TypedPane::File
        | TypedPane::Settings
        | TypedPane::EnvironmentManagement
        | TypedPane::ExecutionProfileEditor
        | TypedPane::Other => None,
    }
}

fn render_detail_section(
    props: &PaneProps<'_>,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    match &props.typed {
        TypedPane::Terminal(terminal_pane) => render_terminal_detail_section(
            props,
            terminal_pane.terminal_view(app).as_ref(app),
            appearance,
            app,
        ),
        TypedPane::Code(_) => render_code_detail_section(props, appearance, app),
        TypedPane::Notebook { .. }
        | TypedPane::Workflow { .. }
        | TypedPane::EnvVarCollection
        | TypedPane::AIFact
        | TypedPane::AIDocument => render_warp_drive_object_detail_section(props, appearance, app),
        TypedPane::CodeDiff
        | TypedPane::File
        | TypedPane::Settings
        | TypedPane::EnvironmentManagement
        | TypedPane::ExecutionProfileEditor
        | TypedPane::Other => Empty::new().finish(),
    }
}
pub(super) struct DetailSidecarOverlay {
    pub(super) anchor_position_id: String,
    pub(super) offset: pathfinder_geometry::vector::Vector2F,
    pub(super) bounds: PositionedElementOffsetBounds,
    pub(super) parent_anchor: PositionedElementAnchor,
    pub(super) child_anchor: ChildAnchor,
    pub(super) sidecar: Box<dyn Element>,
}

pub(super) fn render_detail_sidecar(
    state: &VerticalTabsPanelState,
    workspace: &Workspace,
    side: super::PanelPosition,
    app: &AppContext,
) -> Option<DetailSidecarOverlay> {
    if !*TabSettings::as_ref(app)
        .vertical_tabs_show_details_on_hover
        .value()
    {
        state.clear_detail_sidecar();
        return None;
    }
    let active_target = state
        .detail_overlay_state
        .lock()
        .ok()
        .and_then(|overlay_state| overlay_state.active_target)?;
    let Some(tab) = workspace
        .tabs
        .iter()
        .find(|tab| tab.pane_group.id() == active_target.pane_group_id())
    else {
        state.clear_detail_sidecar();
        return None;
    };
    let context_menu_open_for_tab = workspace
        .tabs
        .iter()
        .position(|tab| tab.pane_group.id() == active_target.pane_group_id())
        .and_then(|tab_index| {
            workspace
                .show_tab_right_click_menu
                .map(|(open_tab_index, _)| open_tab_index == tab_index)
        })
        .unwrap_or(false);
    if context_menu_open_for_tab {
        state.clear_detail_sidecar();
        return None;
    }
    let pane_group = tab.pane_group.as_ref(app);
    let Some(pane_ids) = pane_ids_for_detail_target(pane_group, active_target, app) else {
        state.clear_detail_sidecar();
        return None;
    };
    if pane_ids.is_empty() {
        state.clear_detail_sidecar();
        return None;
    }

    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let sidecar_background = detail_sidecar_background(theme);
    let anchor_position_id = vtab_pane_row_position_id(
        active_target.pane_group_id(),
        active_target.source_pane_id(),
    );
    let (offset, max_height, width, bounds, parent_anchor, child_anchor) =
        detail_sidecar_offset_and_max_height(&anchor_position_id, side, workspace.window_id, app);
    let source_row_mouse_state = state
        .pane_row_mouse_states
        .borrow()
        .get(&active_target.source_pane_id())
        .cloned();
    let detail_overlay_state = state.detail_overlay_state.clone();
    let window_id = workspace.window_id;

    let mut sections = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    for (index, pane_id) in pane_ids.iter().enumerate() {
        let Some(props) = detail_pane_props(
            state,
            workspace,
            pane_group,
            active_target.pane_group_id(),
            *pane_id,
            app,
        ) else {
            state.clear_detail_sidecar();
            return None;
        };
        if index > 0 {
            sections.add_child(
                ConstrainedBox::new(
                    Container::new(Empty::new().finish())
                        .with_background(detail_sidecar_border_fill(theme))
                        .finish(),
                )
                .with_height(1.)
                .finish(),
            );
        }
        sections.add_child(render_detail_section(&props, appearance, app));
    }

    let scrollable = ConstrainedBox::new(
        ClippedScrollable::vertical(
            state.detail_scroll_state.clone(),
            sections.finish(),
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            ElementFill::None,
        )
        .with_overlayed_scrollbar()
        .finish(),
    )
    .with_max_height(max_height)
    .finish();

    let sidecar = Hoverable::new(state.detail_sidecar_mouse_state.clone(), move |_| {
        SavePosition::new(
            Container::new(scrollable)
                .with_background(sidecar_background)
                .with_border(Border::all(1.).with_border_fill(detail_sidecar_border_fill(theme)))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                    DETAIL_SIDECAR_CORNER_RADIUS,
                )))
                .with_drop_shadow(DropShadow::default())
                .finish(),
            VERTICAL_TABS_DETAIL_SIDECAR_POSITION_ID,
        )
        .finish()
    })
    .on_hover(move |is_hovered, ctx, app, position| {
        let mut overlay_state = detail_overlay_state
            .lock()
            .expect("vertical tabs detail overlay lock poisoned");
        overlay_state
            .safe_triangle
            .set_target_rect(app.element_position_by_id_at_last_frame(
                window_id,
                VERTICAL_TABS_DETAIL_SIDECAR_POSITION_ID,
            ));
        overlay_state.safe_triangle.update_position(position);

        if !is_hovered {
            let row_hovered = source_row_mouse_state.as_ref().is_some_and(|mouse_state| {
                mouse_state
                    .lock()
                    .expect("vertical tabs source row hover lock poisoned")
                    .is_mouse_over_element()
            });
            if !row_hovered && overlay_state.active_target == Some(active_target) {
                overlay_state.active_target = None;
                overlay_state.safe_triangle.set_target_rect(None);
                ctx.notify();
            }
        }
    })
    .finish();

    Some(DetailSidecarOverlay {
        anchor_position_id,
        offset,
        bounds,
        parent_anchor,
        child_anchor,
        sidecar: ConstrainedBox::new(sidecar).with_width(width).finish(),
    })
}

fn render_compact_pane_row(props: PaneProps<'_>, app: &AppContext) -> Box<dyn Element> {
    let effective_subtitle = props.subtitle.clone();
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let main_text_color = theme.main_text_color(theme.background());
    let sub_text_color = theme.sub_text_color(theme.background());
    let font_family = appearance.ui_font_family();
    let has_indicator = props.typed.badge(app).is_some() || has_unread_activity(&props.typed, app);

    let icon = render_pane_icon_with_status(
        resolve_icon_with_status_variant(&props.typed, &props.title, appearance, app),
        theme,
    );

    let primary_info = *TabSettings::as_ref(app).vertical_tabs_primary_info.value();
    let compact_subtitle = resolve_compact_subtitle(
        primary_info,
        *TabSettings::as_ref(app)
            .vertical_tabs_compact_subtitle
            .value(),
    );

    // Build title (line 1) based on "Pane title as" and subtitle (line 2) based on
    // "Additional metadata" setting.
    let (title_element, subtitle_element): (Box<dyn Element>, Option<Box<dyn Element>>) =
        if let TypedPane::Terminal(terminal_pane) = &props.typed {
            let terminal_view = terminal_pane.terminal_view(app).as_ref(app);
            let terminal_title = terminal_view.terminal_title_from_shell();
            let git_branch = terminal_view.current_git_branch(app);
            let working_directory = resolved_terminal_working_directory(terminal_view, app);
            let working_directory_text = working_directory
                .clone()
                .unwrap_or_else(|| terminal_title.clone());
            let branch_display =
                branch_label_display(git_branch.as_deref(), working_directory_text.as_str());

            // Title based on "Pane title as"
            let title: Box<dyn Element> = render_pane_title_slot(
                &props,
                || match primary_info {
                    VerticalTabsPrimaryInfo::Command => render_terminal_primary_line_for_view(
                        terminal_view,
                        appearance,
                        main_text_color,
                        app,
                    ),
                    VerticalTabsPrimaryInfo::WorkingDirectory => {
                        Text::new_inline(working_directory_text.clone(), font_family, 12.)
                            .with_clip(ClipConfig::start())
                            .with_color(main_text_color.into())
                            .finish()
                    }
                    VerticalTabsPrimaryInfo::Branch => match branch_display {
                        (branch_text, true) => {
                            render_git_branch_text(&branch_text, main_text_color, 12., appearance)
                        }
                        (fallback_text, false) => Text::new_inline(fallback_text, font_family, 12.)
                            .with_clip(ClipConfig::start())
                            .with_color(main_text_color.into())
                            .finish(),
                    },
                },
                12.,
                main_text_color,
                ClipConfig::ellipsis(),
                appearance,
                app,
            );

            // Subtitle based on "Additional metadata"
            let subtitle: Option<Box<dyn Element>> = match compact_subtitle {
                VerticalTabsCompactSubtitle::Branch => compact_branch_subtitle_display(
                    git_branch.as_deref(),
                    working_directory.as_deref(),
                )
                .map(|(text, show_branch_icon)| {
                    if show_branch_icon {
                        render_git_branch_text(&text, sub_text_color, 10., appearance)
                    } else {
                        Text::new_inline(text, font_family, 10.)
                            .with_clip(ClipConfig::start())
                            .with_color(sub_text_color.into())
                            .finish()
                    }
                }),
                VerticalTabsCompactSubtitle::WorkingDirectory => working_directory.map(|wd| {
                    Text::new_inline(wd, font_family, 10.)
                        .with_clip(ClipConfig::start())
                        .with_color(sub_text_color.into())
                        .finish()
                }),
                VerticalTabsCompactSubtitle::Command => {
                    let agent_text = terminal_agent_text(terminal_view, app);
                    let (conv_title, cli_title) =
                        preferred_agent_tab_titles(&agent_text, agent_tab_text_preference(app));
                    let line_data = terminal_primary_line_data(
                        terminal_view.is_long_running_and_user_controlled(),
                        conv_title,
                        cli_title,
                        terminal_title.as_str(),
                        working_directory_text.as_str(),
                        terminal_title_fallback_font(&agent_text),
                        terminal_view.last_completed_command_text(),
                    );
                    Some(
                        Text::new_inline(line_data.text().to_string(), font_family, 10.)
                            .with_clip(ClipConfig::ellipsis())
                            .with_color(sub_text_color.into())
                            .finish(),
                    )
                }
            };

            (title, subtitle)
        } else {
            let title = render_pane_title_slot(
                &props,
                || {
                    render_compact_non_terminal_title(
                        props.displayed_title(),
                        &props.typed,
                        appearance,
                    )
                },
                12.,
                main_text_color,
                if matches!(props.typed, TypedPane::Code(_)) {
                    ClipConfig::start()
                } else {
                    ClipConfig::ellipsis()
                },
                appearance,
                app,
            );
            let subtitle = if effective_subtitle.is_empty() {
                None
            } else {
                let subtitle_clip = if matches!(props.typed, TypedPane::Code(_)) {
                    ClipConfig::start()
                } else {
                    ClipConfig::ellipsis()
                };
                Some(
                    Text::new_inline(effective_subtitle, font_family, 10.)
                        .with_clip(subtitle_clip)
                        .with_color(sub_text_color.into())
                        .finish(),
                )
            };
            (title, subtitle)
        };

    // Title row with optional indicator
    let title_row = if has_indicator {
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., title_element).finish())
            .with_child(
                Container::new(render_title_indicator(theme))
                    .with_margin_left(4.)
                    .finish(),
            )
            .finish()
    } else {
        title_element
    };

    // Assemble text column: title + optional subtitle
    // Top-align the icon when there are two lines of content; center for single-line rows.
    let icon_alignment = if subtitle_element.is_some() {
        CrossAxisAlignment::Start
    } else {
        CrossAxisAlignment::Center
    };

    let mut text_col = Flex::column()
        .with_main_axis_size(MainAxisSize::Min)
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_spacing(1.);
    text_col.add_child(title_row);

    if let Some(subtitle) = subtitle_element {
        text_col.add_child(subtitle);
    }

    let content = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(icon_alignment)
        .with_spacing(ICON_WITH_STATUS_GAP)
        .with_child(icon)
        .with_child(Shrinkable::new(1., text_col.finish()).finish())
        .finish();

    render_pane_row_element(props, Padding::uniform(8.), true, content, theme)
}

impl Workspace {
    pub(super) fn render_vertical_tabs_panel(
        &self,
        side: super::PanelPosition,
        app: &AppContext,
    ) -> Box<dyn Element> {
        render_vertical_tabs_panel(&self.vertical_tabs_panel, self, side, app)
    }
}

#[cfg(test)]
#[path = "vertical_tabs_tests.rs"]
mod tests;
