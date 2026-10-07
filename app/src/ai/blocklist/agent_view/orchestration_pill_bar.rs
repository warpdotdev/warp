//! Horizontal pill bar shown above the agent view header listing the
//! orchestrator and its child agents. Clicking a pill switches the
//! active pane to that agent's conversation.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_cli::agent::Harness;
use warp_core::channel::ChannelState;
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::color::coloru_with_opacity;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::{Fill, WarpTheme};
use warpui::elements::new_scrollable::{NewScrollable, ScrollableAppearance, SingleAxisConfig};
use warpui::elements::{
    Align, AnchorPair, ChildAnchor, ChildView, ClippedScrollStateHandle, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, DEFAULT_UI_LINE_HEIGHT_RATIO, Element, Empty,
    Fill as ElementFill, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    OffsetPositioning, OffsetType, ParentAnchor, ParentElement, ParentOffsetBounds,
    PositionedElementOffsetBounds, PositioningAxis, Radius, SavePosition, ScrollbarWidth, Stack,
    Text, XAxisAnchor, YAxisAnchor,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::{Cursor, LineStyle};
use warpui::text_layout::{
    ClipConfig, ClipDirection, ClipStyle, DEFAULT_TOP_BOTTOM_RATIO, StyleAndFont, TextStyle,
};
use warpui::{
    AppContext, Entity, EntityId, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::ai::agent::conversation::{
    AIConversation, AIConversationId, ConversationStatus, StatusColorStyle,
};
use crate::ai::artifacts::Artifact;
use crate::ai::blocklist::agent_view::orchestration_conversation_links::{
    is_conversation_open_in_other_visible_view, pane_group_id_containing_terminal_view,
    parent_conversation_id,
};
use crate::ai::blocklist::agent_view::orchestration_pill_bar_model::{
    OrchestrationPillBarEvent, OrchestrationPillBarModel,
};
use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewControllerEvent};
use crate::ai::blocklist::orchestration_topology::{
    LoadedSubtreeRollup, aggregated_orchestrator_status, child_conversations_in_pill_order,
    loaded_subtree_rollup, orchestration_root_conversation_id,
};
use crate::ai::blocklist::telemetry::{
    BlocklistOrchestrationTelemetryEvent, PillBarActionKind, PillBarInteractionEvent,
    PillBarPillKind, PillSwitchOutcome,
};
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::ai::harness_display;
use crate::features::FeatureFlag;
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields};
use crate::pane_group::pane::view::PaneHeaderAction;
use crate::terminal::view::TerminalAction;
use crate::ui_components::icon_with_status::{
    BadgeInnerShape, IconWithStatusVariant, StatusBadgeStyle,
    render_icon_with_status_with_badge_style,
};
use crate::ui_components::icons::Icon;
use crate::workspace::WorkspaceAction;

const PILL_HEIGHT: f32 = 22.;
const PILL_RADIUS: f32 = PILL_HEIGHT / 2.;
const AVATAR_SIZE: f32 = 16.;
const PILL_AVATAR_SLOT_SIZE: f32 = 20.;

/// Visible avatar disc diameter, per design.
const PILL_AVATAR_DISC_SIZE: f32 = 15.;
/// Gap between the avatar disc and each of the pill's horizontal edges. The
/// disc is dead-centre in the pill, so this is symmetric: (22 - 15) / 2 = 3.5.
const PILL_AVATAR_VERTICAL_PADDING: f32 = (PILL_HEIGHT - PILL_AVATAR_DISC_SIZE) / 2.;
/// Square box the status badge is sized and anchored against. It does *not*
/// size the avatar disc (that is [`PILL_AVATAR_DISC_SIZE`]) — it only reserves
/// the square whose bottom-right corner the badge hangs off.
const AVATAR_WITH_STATUS_TOTAL_SIZE: f32 = PILL_AVATAR_SLOT_SIZE;
const PILL_LABEL_MAX_WIDTH: f32 = 83.;
const PILL_ROW_GAP: f32 = 8.;
const PILL_CONTENT_GAP: f32 = 2.;
const PILL_SELECTED_HOVER_CONTENT_GAP: f32 = 4.;
const PILL_HORIZONTAL_PADDING_LEFT: f32 = 4.;
const PILL_HORIZONTAL_PADDING_RIGHT: f32 = 6.;
const PILL_ICON_BUTTON_SIZE: f32 = 16.;
const PILL_ICON_SIZE: f32 = 12.;
const PILL_OVERFLOW_BUTTON_RIGHT_OFFSET: f32 = 4.;
const STATIC_PILL_LABEL_MAX_WIDTH: f32 = 110.;
const STATIC_PILL_HORIZONTAL_PADDING_RIGHT: f32 = 10.;
/// Width of the overlaid horizontal scrollbar; thin hairline by design.
const PILL_BAR_SCROLLBAR_WIDTH: f32 = 4.;
/// Desired gap between the pills and the scrollbar thumb.
const PILL_BAR_SCROLLBAR_GAP: f32 = 1.;
/// Bottom gutter for the overlaid scrollbar. Its track sits 2px below the thumb
/// (NewScrollable's `RIGHT_PADDING`), so gap = gutter - width - 2.
const PILL_BAR_SCROLLBAR_GUTTER: f32 = PILL_BAR_SCROLLBAR_GAP + PILL_BAR_SCROLLBAR_WIDTH + 2.;

/// Stable palette used to color child agent avatars deterministically by name.
fn pill_palette(theme: &WarpTheme) -> [ColorU; 6] {
    [
        theme.ansi_fg_blue(),
        theme.ansi_fg_magenta(),
        theme.ansi_fg_cyan(),
        theme.ansi_fg_green(),
        theme.ansi_fg_yellow(),
        theme.ansi_fg_red(),
    ]
}

pub(crate) fn pill_avatar_color(name: &str, theme: &WarpTheme) -> ColorU {
    let palette = pill_palette(theme);
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let idx = (hasher.finish() as usize) % palette.len();
    palette[idx]
}

pub(crate) fn pill_initial(name: &str) -> char {
    name.trim()
        .chars()
        .next()
        .map(|c| c.to_ascii_uppercase())
        .unwrap_or('A')
}

/// Renders the orchestrator avatar disc shared by pill, breadcrumb, and transcript
/// surfaces.
pub(crate) fn render_orchestrator_avatar_disc(
    size: f32,
    theme: &WarpTheme,
    appearance: &Appearance,
) -> Box<dyn Element> {
    render_avatar_disc(
        theme.ansi_fg_cyan(),
        AvatarGlyph::Icon(Icon::Agent),
        size,
        theme,
        appearance,
    )
}

/// Renders a child-agent avatar using the same deterministic-color + initial-letter
/// treatment as the orchestration pill bar.
pub(crate) fn render_agent_avatar_disc(
    name: &str,
    size: f32,
    theme: &WarpTheme,
    appearance: &Appearance,
) -> Box<dyn Element> {
    render_avatar_disc(
        pill_avatar_color(name, theme),
        AvatarGlyph::Letter(pill_initial(name)),
        size,
        theme,
        appearance,
    )
}

/// What kind of pill we are rendering, which determines click behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PillKind {
    Orchestrator,
    Child,
}

impl PillKind {
    fn telemetry_kind(self) -> PillBarPillKind {
        match self {
            Self::Orchestrator => PillBarPillKind::Orchestrator,
            Self::Child => PillBarPillKind::Child,
        }
    }
}

/// Whether the user has pinned this pill to the leading section of the bar.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PillPinState {
    Unpinned,
    Pinned,
}

/// Pre-computed data for one pill in the bar.
struct PillSpec {
    conversation_id: AIConversationId,
    label: String,
    avatar_color: ColorU,
    avatar_glyph: AvatarGlyph,
    status: Option<ConversationStatus>,
    is_selected: bool,
    kind: PillKind,
    pin_state: PillPinState,
    /// Child running on a remote worker; drives the cloud-shaped badge variant.
    is_remote_child: bool,
    /// Present when this child is itself an orchestrator: rolled-up state of
    /// its subtree, rendered as a trailing "group" badge on the pill.
    subtree_rollup: Option<LoadedSubtreeRollup>,
}

/// Everything `pill_specs` computes for one render of the bar. The bar is a
/// drill-down view: it anchors on one conversation and renders only that
/// conversation's DIRECT children, with breadcrumbs back up the tree when
/// the anchored level sits below the root.
struct PillBarContents {
    anchor_id: AIConversationId,
    /// Root of the orchestration tree when the anchor is not itself the
    /// root; drives the leading breadcrumb pill.
    breadcrumb_root_id: Option<AIConversationId>,
    /// The anchor's direct parent when it is neither the anchor nor already
    /// covered by the root breadcrumb (i.e. the anchor sits 2+ levels below
    /// the root); rendered after the root breadcrumb.
    breadcrumb_parent_id: Option<AIConversationId>,
    specs: Vec<PillSpec>,
}

#[derive(Clone, Copy)]
enum AvatarGlyph {
    Letter(char),
    Icon(Icon),
}

/// Width of the per-pill 3-dot overflow menu when expanded.
const OVERFLOW_MENU_WIDTH: f32 = 200.;
/// Size in logical pixels of the 3-dot button at the trailing edge of each
/// child pill.
const OVERFLOW_BUTTON_SIZE: f32 = PILL_ICON_BUTTON_SIZE;
/// How much of the label slot the overflow button overlays.
const OVERFLOW_BUTTON_LABEL_RESERVE: f32 =
    OVERFLOW_BUTTON_SIZE + PILL_OVERFLOW_BUTTON_RIGHT_OFFSET - PILL_HORIZONTAL_PADDING_RIGHT;

/// Returns the saved-position id used to anchor the 3-dot menu to a
/// specific child pill's overflow button. The id is global within the
/// position cache, so we include the conversation id to keep it unique
/// across multiple sibling pills.
fn overflow_button_position_id(conversation_id: AIConversationId) -> String {
    format!("orchestration-pill-overflow-{conversation_id}")
}

/// Returns the saved-position id used to anchor the hover details card
/// to a specific pill's body. Unique per conversation so neighbouring
/// pills don't fight over the same id.
fn pill_body_position_id(conversation_id: AIConversationId) -> String {
    format!("orchestration-pill-body-{conversation_id}")
}

fn pill_label_width(
    label: &str,
    font_size: f32,
    font_properties: Properties,
    appearance: &Appearance,
    app: &AppContext,
) -> f32 {
    if label.is_empty() {
        return 0.;
    }

    let font_cache = app.font_cache();
    let text_layout_system = font_cache.text_layout_system();
    let line = text_layout_system.layout_line(
        label,
        LineStyle {
            font_size,
            line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
            fixed_width_tab_size: None,
        },
        &[(
            0..label.chars().count(),
            StyleAndFont::new(
                appearance.ui_font_family(),
                font_properties,
                TextStyle::new(),
            ),
        )],
        f32::MAX,
        ClipConfig::default(),
    );
    line.width
}

/// Width of the per-pill hover details card.
const HOVER_CARD_WIDTH: f32 = 280.;
const HOVER_CARD_HORIZONTAL_PADDING: f32 = 12.;
const HOVER_CARD_VERTICAL_PADDING: f32 = 10.;
const HOVER_CARD_CONTENT_WIDTH: f32 = HOVER_CARD_WIDTH - 2. * HOVER_CARD_HORIZONTAL_PADDING;
const HOVER_CARD_HEADER_AVATAR_NAME_GAP: f32 = 8.;
const HOVER_CARD_HEADER_NAME_BADGE_GAP: f32 = 8.;
/// Slightly larger than the longest expected status label ("In progress") plus
/// its icon and padding.
const HOVER_CARD_STATUS_BADGE_MAX_WIDTH: f32 = 96.;

/// Typed actions dispatched by the pill bar's widgets. Each action carries
/// the targeted child pill's conversation id so a single shared `Menu`
/// instance can serve every child.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrchestrationPillBarAction {
    /// Open the 3-dot menu for the given child conversation.
    OpenMenu(AIConversationId),
    /// Close the open menu (forwarded from the `Menu`'s `Close` event).
    CloseMenu,
    /// Menu item: split the pane and host this child in the new pane.
    OpenInNewPane(AIConversationId),
    /// Menu item: open this child in a new tab.
    OpenInNewTab(AIConversationId),
    /// Menu item: open this child's run in the Oz web app.
    ViewInOz(AIConversationId),
    /// Menu item: stop the in-progress task.
    Stop(AIConversationId),
    /// Menu item: cancel and remove from local history.
    Kill(AIConversationId),
    /// Set/clear which pill the user is hovering (drives the details card).
    SetHoveredPill(Option<AIConversationId>),
    /// Menu item: focus the existing pane/tab that already owns the
    /// child agent's transcript instead of splitting/opening a new one.
    FocusOpenedConversation(AIConversationId),
    /// Toggle the pin state for the given child conversation.
    TogglePin(AIConversationId),
    /// Pill body was clicked. Dispatched in lieu of the navigation
    /// `TerminalAction` so telemetry can be emitted before the
    /// downstream navigation runs.
    PillClicked {
        conversation_id: AIConversationId,
        pill_kind: PillKind,
        /// Set for the leading breadcrumb pills so telemetry can tell
        /// drill-up navigation apart from same-level pill switches
        /// (navigation itself only depends on `pill_kind`).
        is_breadcrumb: bool,
    },
}

/// Renders the pill bar above the agent view: one pill for the orchestrator
/// and one per child agent. Clicking a non-active pill switches to its pane.
pub struct OrchestrationPillBar {
    agent_view_controller: ModelHandle<AgentViewController>,
    /// Hover state per pill, persisted across renders. `RefCell` so
    /// `render` can lazily insert handles missing from `ensure_mouse_states`.
    mouse_states: RefCell<HashMap<AIConversationId, MouseStateHandle>>,
    /// Hover state per child pill's 3-dot button (separate from the pill body).
    overflow_button_mouse_states: RefCell<HashMap<AIConversationId, MouseStateHandle>>,
    /// Hover state per child pill's leading pin button (independent of the
    /// pill body and the 3-dot button so each surface highlights on its own).
    pin_button_mouse_states: RefCell<HashMap<AIConversationId, MouseStateHandle>>,
    /// Shared dropdown menu rebuilt per-open with the targeted child's id.
    menu: ViewHandle<Menu<OrchestrationPillBarAction>>,
    /// `Some(id)` when the 3-dot menu is open targeting that child.
    menu_open_for: Option<AIConversationId>,
    /// `Some(id)` when the cursor is hovering that pill (drives the details card).
    hovered_pill: Option<AIConversationId>,
}

impl Entity for OrchestrationPillBar {
    type Event = ();
}

impl OrchestrationPillBar {
    fn overflow_menu_item(
        label: &'static str,
        icon: Icon,
        action: OrchestrationPillBarAction,
        hover_background: Fill,
        icon_color: Option<Fill>,
    ) -> MenuItem<OrchestrationPillBarAction> {
        let mut fields = MenuItemFields::new(label)
            .with_icon(icon)
            .with_override_hover_background_color(hover_background)
            .with_on_select_action(action);
        if let Some(color) = icon_color {
            fields = fields.with_override_icon_color(color);
        }
        MenuItem::Item(fields)
    }

    pub fn new(
        agent_view_controller: ModelHandle<AgentViewController>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, |this, _, event, ctx| match event {
            BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
            | BlocklistAIHistoryEvent::AppendedExchange { .. }
            | BlocklistAIHistoryEvent::SetActiveConversation { .. }
            | BlocklistAIHistoryEvent::StartedNewConversation { .. }
            // A remote child's run-id linkage can land after
            // StartedNewConversation; pill contents and badges keyed on run
            // linkage must refresh when it does.
            | BlocklistAIHistoryEvent::ConversationServerTokenAssigned { .. } => {
                this.ensure_mouse_states(ctx);
                ctx.notify();
            }
            BlocklistAIHistoryEvent::RemoveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id, ..
            } => {
                this.mouse_states.borrow_mut().remove(conversation_id);
                this.overflow_button_mouse_states
                    .borrow_mut()
                    .remove(conversation_id);
                this.pin_button_mouse_states
                    .borrow_mut()
                    .remove(conversation_id);
                // Pin set + scroll handle pruning live in the pill bar
                // model singleton.
                // If the menu was open for a child that just disappeared,
                // close it so we don't leave a dangling menu pointing at a
                // dead conversation id.
                if this.menu_open_for == Some(*conversation_id) {
                    this.menu_open_for = None;
                }
                ctx.notify();
            }
            _ => {}
        });
        ctx.subscribe_to_model(&agent_view_controller, |this, _, event, ctx| {
            if matches!(
                event,
                AgentViewControllerEvent::EnteredAgentView { .. }
                    | AgentViewControllerEvent::ExitedAgentView { .. }
            ) {
                this.mouse_states.borrow_mut().clear();
                this.overflow_button_mouse_states.borrow_mut().clear();
                this.pin_button_mouse_states.borrow_mut().clear();
                this.menu_open_for = None;
            }
            this.ensure_mouse_states(ctx);
            ctx.notify();
        });

        let menu = ctx.add_typed_action_view(|_ctx| {
            Menu::new()
                .with_width(OVERFLOW_MENU_WIDTH)
                .with_drop_shadow()
                .prevent_interaction_with_other_elements()
        });

        // Forward the menu's Close event so menu_open_for stays in sync.
        ctx.subscribe_to_view(&menu, |this, _, event, ctx| match event {
            MenuEvent::Close { .. } => {
                this.handle_action(&OrchestrationPillBarAction::CloseMenu, ctx);
            }
            MenuEvent::ItemSelected | MenuEvent::ItemHovered => {}
        });

        // Re-render whenever any pane toggles a pin so the bars stay in sync.
        let pill_bar_model = OrchestrationPillBarModel::handle(ctx);
        ctx.subscribe_to_model(&pill_bar_model, |_, _, event, ctx| match event {
            OrchestrationPillBarEvent::PinSetChanged => ctx.notify(),
        });

        Self {
            agent_view_controller,
            mouse_states: RefCell::new(HashMap::new()),
            overflow_button_mouse_states: RefCell::new(HashMap::new()),
            pin_button_mouse_states: RefCell::new(HashMap::new()),
            menu,
            menu_open_for: None,
            hovered_pill: None,
        }
    }

    /// Rebuilds menu items for the given child and opens the menu.
    fn open_menu_for(&mut self, conversation_id: AIConversationId, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme();
        let hover_background: Fill = internal_colors::neutral_4(theme).into();
        let item = |label, icon, action| {
            Self::overflow_menu_item(label, icon, action, hover_background, None)
        };
        let destructive_color: Fill = theme.ansi_fg_red().into();
        let destructive_item = |label, icon, action| {
            Self::overflow_menu_item(
                label,
                icon,
                action,
                hover_background,
                Some(destructive_color),
            )
        };

        // If this child is already open in a *different* visible terminal
        // view, collapse the create-new entries into a single "Focus pane"
        // entry pointing at the existing owner.
        let self_terminal_view_id = self.agent_view_controller.as_ref(ctx).terminal_view_id();
        let is_open_elsewhere =
            is_conversation_open_in_other_visible_view(conversation_id, self_terminal_view_id, ctx);

        let mut items = if is_open_elsewhere {
            vec![item(
                "Focus pane",
                Icon::ArrowSplit,
                OrchestrationPillBarAction::FocusOpenedConversation(conversation_id),
            )]
        } else {
            vec![
                item(
                    "Open in new pane",
                    Icon::ArrowSplit,
                    OrchestrationPillBarAction::OpenInNewPane(conversation_id),
                ),
                item(
                    "Open in new tab",
                    Icon::Plus,
                    OrchestrationPillBarAction::OpenInNewTab(conversation_id),
                ),
            ]
        };
        if Self::oz_run_url_for_conversation(conversation_id, ctx).is_some() {
            items.push(item(
                "View in Oz",
                Icon::Oz,
                OrchestrationPillBarAction::ViewInOz(conversation_id),
            ));
        }
        // Stop is shown only while the agent is in progress; Kill becomes
        // Delete once the agent's run has finished (Success / Error /
        // Cancelled). Blocked is treated as not-yet-finished (the agent
        // is still mid-flight, waiting on user input).
        let conversation_status = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .map(|conversation| conversation.status().clone());
        let is_in_progress = conversation_status
            .as_ref()
            .is_some_and(|status| status.is_in_progress());
        let is_in_finished_state = conversation_status
            .as_ref()
            .is_some_and(|status| status.is_done());
        items.push(MenuItem::Separator);
        if is_in_progress {
            items.push(destructive_item(
                "Stop agent",
                Icon::StopFilled,
                OrchestrationPillBarAction::Stop(conversation_id),
            ));
        }
        let (kill_label, kill_icon) = if is_in_finished_state {
            ("Delete agent", Icon::Trash)
        } else {
            ("Kill agent", Icon::X)
        };
        items.push(destructive_item(
            kill_label,
            kill_icon,
            OrchestrationPillBarAction::Kill(conversation_id),
        ));

        self.menu.update(ctx, |menu, ctx| {
            menu.set_items(items, ctx);
        });
        self.menu_open_for = Some(conversation_id);
        // Suppress the hover card while the menu overlays the same pill.
        self.hovered_pill = None;
        ctx.focus(&self.menu);
        ctx.notify();
    }

    fn close_menu(&mut self, ctx: &mut ViewContext<Self>) {
        if self.menu_open_for.is_none() {
            return;
        }
        self.menu_open_for = None;
        ctx.notify();
    }

    fn oz_run_url_for_conversation(
        conversation_id: AIConversationId,
        app: &AppContext,
    ) -> Option<String> {
        let run_id = BlocklistAIHistoryModel::as_ref(app)
            .conversation(&conversation_id)?
            .run_id()?;
        let oz_root_url = ChannelState::oz_root_url();
        Some(format!("{oz_root_url}/runs/{run_id}"))
    }

    fn set_hovered_pill(
        &mut self,
        conversation_id: Option<AIConversationId>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.hovered_pill == conversation_id {
            return;
        }
        self.hovered_pill = conversation_id;
        ctx.notify();
    }

    fn ensure_mouse_states(&mut self, ctx: &AppContext) {
        let Some(active_id) = self
            .agent_view_controller
            .as_ref(ctx)
            .agent_view_state()
            .active_conversation_id()
        else {
            return;
        };
        let history = BlocklistAIHistoryModel::as_ref(ctx);
        let Some(active_conversation) = history.conversation(&active_id) else {
            return;
        };
        let anchor_id = drill_down_anchor_id(active_id, active_conversation, ctx);
        // Track only ids that are still rendered; retain step prevents leaking
        // handles for old orchestrators / children when switching views.
        let mut alive: HashSet<AIConversationId> = HashSet::new();
        alive.insert(anchor_id);
        alive.insert(active_id);
        // The breadcrumb pills anchor on the tree root and the anchor's parent.
        let (breadcrumb_root_id, breadcrumb_parent_id) = breadcrumb_ids(history, anchor_id);
        alive.extend(breadcrumb_root_id);
        alive.extend(breadcrumb_parent_id);
        for child_id in history.child_conversation_ids_of(&anchor_id) {
            alive.insert(*child_id);
        }
        let mut mouse_states = self.mouse_states.borrow_mut();
        let mut overflow_states = self.overflow_button_mouse_states.borrow_mut();
        let mut pin_states = self.pin_button_mouse_states.borrow_mut();
        for id in &alive {
            mouse_states.entry(*id).or_default();
            overflow_states.entry(*id).or_default();
            pin_states.entry(*id).or_default();
        }
        mouse_states.retain(|id, _| alive.contains(id));
        overflow_states.retain(|id, _| alive.contains(id));
        pin_states.retain(|id, _| alive.contains(id));
        // Pin set pruning lives in the singleton — `alive` only covers this
        // pane's tree, so pruning here would clobber pins in other panes.
    }

    /// Builds the drill-down pill bar contents for the active conversation,
    /// or `None` when nothing should render.
    fn pill_specs(&self, app: &AppContext) -> Option<PillBarContents> {
        let active_id = self
            .agent_view_controller
            .as_ref(app)
            .agent_view_state()
            .active_conversation_id()?;
        let history = BlocklistAIHistoryModel::as_ref(app);
        let active_conversation = history.conversation(&active_id)?;

        let anchor_id = drill_down_anchor_id(active_id, active_conversation, app);
        let anchor = history.conversation(&anchor_id)?;

        // Per-level ordering is shared with keyboard navigation, but the two
        // consume it differently: cycling walks the whole tree while the bar
        // renders only the anchor's DIRECT children — deeper levels are
        // reached by drilling into a group pill, and the bar follows the
        // keyboard selection by re-anchoring (`drill_down_anchor_id`).
        let children: Vec<_> = child_conversations_in_pill_order(history, anchor_id)
            .into_iter()
            .filter_map(|descendant| history.conversation(&descendant.conversation_id))
            .collect();

        // Nothing to show if the anchor has no children yet.
        if children.is_empty() {
            return None;
        }
        let (breadcrumb_root_id, breadcrumb_parent_id) = breadcrumb_ids(history, anchor_id);
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut specs = Vec::with_capacity(1 + children.len());

        // Anchor pill first; never pinned. Its badge aggregates its subtree,
        // while child pills show per-child status.
        specs.push(PillSpec {
            conversation_id: anchor_id,
            label: orchestrator_label(anchor),
            avatar_color: theme.ansi_fg_cyan(),
            avatar_glyph: AvatarGlyph::Icon(Icon::Agent),
            status: Some(aggregated_orchestrator_status(history, anchor_id)),
            is_selected: anchor_id == active_id,
            kind: PillKind::Orchestrator,
            pin_state: PillPinState::Unpinned,
            is_remote_child: anchor.is_remote_child(),
            subtree_rollup: None,
        });

        // Stamp each child's current pin state; partitioning happens at render.
        let pill_bar_model = OrchestrationPillBarModel::as_ref(app);
        for child in children {
            let name = child
                .agent_name()
                .filter(|n| !n.is_empty())
                .unwrap_or("Agent");
            let pin_state = if pill_bar_model.is_pinned(&child.id()) {
                PillPinState::Pinned
            } else {
                PillPinState::Unpinned
            };
            // A child with children of its own renders as a "group" pill:
            // its own status on the avatar plus a rolled-up subtree badge.
            let subtree_rollup = loaded_subtree_rollup(history, child.id());
            specs.push(PillSpec {
                conversation_id: child.id(),
                label: name.to_string(),
                avatar_color: pill_avatar_color(name, theme),
                avatar_glyph: AvatarGlyph::Letter(pill_initial(name)),
                status: Some(child.status().clone()),
                is_selected: child.id() == active_id,
                kind: PillKind::Child,
                pin_state,
                is_remote_child: child.is_remote_child(),
                subtree_rollup,
            });
        }

        Some(PillBarContents {
            anchor_id,
            breadcrumb_root_id,
            breadcrumb_parent_id,
            specs,
        })
    }
}

/// Resolves the breadcrumb targets shown while the bar is drilled below the
/// tree root: the root itself, plus the anchor's direct parent when that
/// parent is a distinct intermediate level (anchor 2+ levels below the
/// root). When the parent IS the root only the root breadcrumb is returned,
/// so the bar never shows duplicate affordances.
fn breadcrumb_ids(
    history: &BlocklistAIHistoryModel,
    anchor_id: AIConversationId,
) -> (Option<AIConversationId>, Option<AIConversationId>) {
    let root_id = orchestration_root_conversation_id(history, anchor_id)
        .filter(|root_id| *root_id != anchor_id);
    let parent_id = history
        .conversation(&anchor_id)
        .and_then(|anchor| history.resolved_parent_conversation_id_for_conversation(anchor))
        .filter(|parent_id| Some(*parent_id) != root_id && *parent_id != anchor_id);
    (root_id, parent_id)
}

/// Resolves which conversation's level the drill-down bar shows for
/// `active_id`: a conversation with children anchors its own level, while a
/// leaf anchors its parent's level so sibling navigation stays symmetric. At
/// orchestration depth 1 this matches the historical root-anchored behavior
/// exactly.
fn drill_down_anchor_id(
    active_id: AIConversationId,
    active_conversation: &AIConversation,
    app: &AppContext,
) -> AIConversationId {
    let history = BlocklistAIHistoryModel::as_ref(app);
    if history.child_conversation_ids_of(&active_id).is_empty() {
        parent_conversation_id(active_conversation, app).unwrap_or(active_id)
    } else {
        active_id
    }
}

/// Renders a non-interactive agent pill using the same deterministic-color
/// + initial-letter avatar as the live pill bar.
pub fn render_static_agent_pill(name: &str, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let avatar = render_agent_avatar_disc(name, AVATAR_SIZE, theme, appearance);
    let text_color = theme.ansi_fg_magenta();
    let bg_color = coloru_with_opacity(text_color, 10);
    let label_text = Text::new(name.to_string(), appearance.ui_font_family(), 12.)
        .with_color(text_color)
        .soft_wrap(false)
        .with_clip(ClipConfig::ellipsis())
        .finish();

    let row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(6.)
        .with_child(avatar)
        .with_child(
            ConstrainedBox::new(label_text)
                .with_max_width(STATIC_PILL_LABEL_MAX_WIDTH)
                .finish(),
        )
        .finish();

    ConstrainedBox::new(
        Container::new(row)
            .with_padding_left(PILL_HORIZONTAL_PADDING_LEFT)
            .with_padding_right(STATIC_PILL_HORIZONTAL_PADDING_RIGHT)
            .with_background_color(bg_color)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(PILL_RADIUS)))
            .finish(),
    )
    .with_height(PILL_HEIGHT)
    .finish()
}

/// Returns the label to use for the orchestrator pill. Prefers the explicitly
/// set agent name, falling back to "Orchestrator" so the pill is meaningful
/// even before any naming has happened.
fn orchestrator_label(orchestrator: &AIConversation) -> String {
    orchestrator
        .agent_name()
        .filter(|n| !n.is_empty())
        .map(|n| n.to_string())
        .unwrap_or_else(|| "Orchestrator".to_string())
}

impl OrchestrationPillBar {
    /// Resolves the anchor / root / total-pills / total-pinned tuple used
    /// to enrich every `PillBarInteraction` event. The anchor becomes the
    /// payload's `source_conversation_id`; the tree root rides alongside
    /// so drilled-down interactions stay attributable to their tree.
    /// Returns `None` when there is no active orchestration tree to
    /// attribute the interaction to.
    fn pill_bar_telemetry_context(
        &self,
        app: &AppContext,
    ) -> Option<(AIConversationId, AIConversationId, usize, usize)> {
        let contents = self.pill_specs(app)?;
        let total_pills = contents.specs.len();
        let total_pinned = contents
            .specs
            .iter()
            .filter(|spec| matches!(spec.pin_state, PillPinState::Pinned))
            .count();
        let root_id = contents.breadcrumb_root_id.unwrap_or(contents.anchor_id);
        Some((contents.anchor_id, root_id, total_pills, total_pinned))
    }

    /// Pill kind for `target_id` in the current pill specs. Defaults
    /// to `Child` if the id is no longer in the bar.
    fn pill_kind_for(&self, target_id: AIConversationId, app: &AppContext) -> PillBarPillKind {
        self.pill_specs(app)
            .and_then(|contents| {
                contents
                    .specs
                    .into_iter()
                    .find(|spec| spec.conversation_id == target_id)
                    .map(|spec| spec.kind.telemetry_kind())
            })
            .unwrap_or(PillBarPillKind::Child)
    }

    fn emit_pill_bar_interaction(
        &self,
        action: PillBarActionKind,
        pill_kind: PillBarPillKind,
        target_conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.emit_pill_bar_interaction_with_outcome(
            action,
            pill_kind,
            target_conversation_id,
            None,
            ctx,
        );
    }

    /// Same as [`Self::emit_pill_bar_interaction`] but stamps a
    /// `switch_outcome` on the payload. Use for `Switch` actions where
    /// the analyst needs to know whether the click navigated in place
    /// or focused an existing pane.
    fn emit_pill_switch(
        &self,
        pill_kind: PillBarPillKind,
        target_conversation_id: AIConversationId,
        outcome: PillSwitchOutcome,
        ctx: &mut ViewContext<Self>,
    ) {
        self.emit_pill_bar_interaction_with_outcome(
            PillBarActionKind::Switch,
            pill_kind,
            target_conversation_id,
            Some(outcome),
            ctx,
        );
    }

    fn emit_pill_bar_interaction_with_outcome(
        &self,
        action: PillBarActionKind,
        pill_kind: PillBarPillKind,
        target_conversation_id: AIConversationId,
        switch_outcome: Option<PillSwitchOutcome>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some((source_conversation_id, root_conversation_id, total_pills, total_pinned)) =
            self.pill_bar_telemetry_context(ctx)
        else {
            return;
        };
        send_telemetry_from_ctx!(
            BlocklistOrchestrationTelemetryEvent::PillBarInteraction(PillBarInteractionEvent {
                action,
                pill_kind,
                total_pills,
                total_pinned,
                source_conversation_id,
                root_conversation_id,
                target_conversation_id,
                switch_outcome,
            }),
            ctx
        );
    }

    /// Dispatches the focus-existing-pane navigation. Pulled out of
    /// the `FocusOpenedConversation` handler so the `PillClicked`
    /// handler can reuse the same nav logic without emitting the
    /// menu-driven `FocusOpenedConversation` telemetry event.
    fn navigate_to_conversation_pane(&self, id: AIConversationId, ctx: &mut ViewContext<Self>) {
        // "Focus pane" is purely a focus operation: the conversation
        // already lives in some other visible terminal view (verified
        // by `is_conversation_open_in_other_visible_view` at the call
        // site) and we just want to move the user's cursor there. We
        // deliberately do *not* go through
        // `RestoreOrNavigateToConversation`: that path calls
        // `set_active_conversation_id` with whichever
        // `terminal_view_id` it receives, which would either
        // reassign the terminal surface to a stale id pulled from
        // `AgentConversationsModel::nav_data` or, worse, blank out
        // the real conversation pane while the conversation pops back into
        // the orchestrator.
        //
        // Resolve the canonical terminal surface directly from
        // `BlocklistAIHistoryModel` (the single source of truth) and
        // pick the appropriate focus action based on whether the
        // conversation pane lives in the same pane group as us:
        //   * Same pane group (sibling pane in this tab) —
        //     dispatch `TerminalAction::RevealChildAgent`. The pane
        //     group's handler walks visible terminal panes and calls
        //     `group.focus_pane(.., true, ctx)` from its own
        //     `ViewContext<PaneGroup>`, which actually shifts focus
        //     to the sibling pane. Going through the workspace's
        //     `focus_pane` from a different `ViewContext` doesn't
        //     reliably move focus when the destination is in the
        //     same pane group.
        //   * Different pane group (other tab / window) —
        //     dispatch `WorkspaceAction::FocusTerminalViewInWorkspace`,
        //     which walks all tabs/windows and activates the
        //     containing tab as needed.
        let conversation_view_id =
            BlocklistAIHistoryModel::as_ref(ctx).terminal_surface_id_for_conversation(&id);
        let Some(conversation_view_id) = conversation_view_id else {
            log::warn!(
                "navigate_to_conversation_pane: no canonical terminal surface for {id:?}; falling back to switch-in-place"
            );
            ctx.dispatch_typed_action(
                &PaneHeaderAction::<TerminalAction, TerminalAction>::CustomAction(
                    TerminalAction::SwitchAgentViewToConversation {
                        conversation_id: id,
                    },
                ),
            );
            return;
        };
        let self_pane_group_id = self.agent_view_controller.as_ref(ctx).pane_group_id();
        let conversation_pane_group_id =
            pane_group_id_containing_terminal_view(conversation_view_id, ctx);
        if conversation_pane_group_id.is_some() && conversation_pane_group_id == self_pane_group_id
        {
            ctx.dispatch_typed_action(
                &PaneHeaderAction::<TerminalAction, TerminalAction>::CustomAction(
                    TerminalAction::RevealChildAgent {
                        conversation_id: id,
                    },
                ),
            );
        } else {
            ctx.dispatch_typed_action(&WorkspaceAction::FocusTerminalViewInWorkspace {
                terminal_view_id: conversation_view_id,
            });
        }
    }
}

impl TypedActionView for OrchestrationPillBar {
    type Action = OrchestrationPillBarAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            OrchestrationPillBarAction::OpenMenu(id) => {
                let pill_kind = self.pill_kind_for(*id, ctx);
                self.emit_pill_bar_interaction(PillBarActionKind::OpenMenu, pill_kind, *id, ctx);
                self.open_menu_for(*id, ctx);
            }
            OrchestrationPillBarAction::CloseMenu => {
                self.close_menu(ctx);
            }
            OrchestrationPillBarAction::OpenInNewPane(id) => {
                // Defer the actual pane split / tab open / cancel logic to
                // `TerminalView::handle_action`, which already owns the
                // wiring added in Phase C. We just translate the typed
                // pill-bar action into the existing `TerminalAction` and
                // dispatch it through the pane header action surface so
                // it bubbles up the standard way (mirrors the pill-click
                // path in `render_pill`).
                self.emit_pill_bar_interaction(
                    PillBarActionKind::OpenInNewPane,
                    PillBarPillKind::Child,
                    *id,
                    ctx,
                );
                self.close_menu(ctx);
                ctx.dispatch_typed_action(
                    &PaneHeaderAction::<TerminalAction, TerminalAction>::CustomAction(
                        TerminalAction::OpenChildAgentInNewPane {
                            conversation_id: *id,
                        },
                    ),
                );
            }
            OrchestrationPillBarAction::OpenInNewTab(id) => {
                self.emit_pill_bar_interaction(
                    PillBarActionKind::OpenInNewTab,
                    PillBarPillKind::Child,
                    *id,
                    ctx,
                );
                self.close_menu(ctx);
                ctx.dispatch_typed_action(
                    &PaneHeaderAction::<TerminalAction, TerminalAction>::CustomAction(
                        TerminalAction::OpenChildAgentInNewTab {
                            conversation_id: *id,
                        },
                    ),
                );
            }
            OrchestrationPillBarAction::ViewInOz(id) => {
                self.emit_pill_bar_interaction(
                    PillBarActionKind::ViewInOz,
                    PillBarPillKind::Child,
                    *id,
                    ctx,
                );
                self.close_menu(ctx);
                if let Some(url) = Self::oz_run_url_for_conversation(*id, ctx) {
                    ctx.open_url(&url);
                }
            }
            OrchestrationPillBarAction::Stop(id) => {
                self.emit_pill_bar_interaction(
                    PillBarActionKind::Stop,
                    PillBarPillKind::Child,
                    *id,
                    ctx,
                );
                self.close_menu(ctx);
                ctx.dispatch_typed_action(
                    &PaneHeaderAction::<TerminalAction, TerminalAction>::CustomAction(
                        TerminalAction::StopAgentConversation {
                            conversation_id: *id,
                        },
                    ),
                );
            }
            OrchestrationPillBarAction::Kill(id) => {
                self.emit_pill_bar_interaction(
                    PillBarActionKind::Kill,
                    PillBarPillKind::Child,
                    *id,
                    ctx,
                );
                self.close_menu(ctx);
                ctx.dispatch_typed_action(
                    &PaneHeaderAction::<TerminalAction, TerminalAction>::CustomAction(
                        TerminalAction::KillAgentConversation {
                            conversation_id: *id,
                        },
                    ),
                );
            }
            OrchestrationPillBarAction::SetHoveredPill(id) => {
                self.set_hovered_pill(*id, ctx);
            }
            OrchestrationPillBarAction::TogglePin(id) => {
                // Singleton emits an event that drives the re-render in every
                // pill bar, so no `ctx.notify()` needed here.
                let id = *id;
                // Determine which way the toggle is going before applying
                // it so the telemetry payload reports the resulting state
                // rather than the prior one.
                let was_pinned = OrchestrationPillBarModel::as_ref(ctx).is_pinned(&id);
                let action_kind = if was_pinned {
                    PillBarActionKind::TogglePinOff
                } else {
                    PillBarActionKind::TogglePinOn
                };
                self.emit_pill_bar_interaction(action_kind, PillBarPillKind::Child, id, ctx);
                OrchestrationPillBarModel::handle(ctx).update(ctx, |model, ctx| {
                    model.toggle_pin(id, ctx);
                });
            }
            OrchestrationPillBarAction::PillClicked {
                conversation_id,
                pill_kind,
                is_breadcrumb,
            } => {
                let id = *conversation_id;
                let self_terminal_view_id =
                    self.agent_view_controller.as_ref(ctx).terminal_view_id();
                let is_open_elsewhere =
                    is_conversation_open_in_other_visible_view(id, self_terminal_view_id, ctx);
                // Pill-body clicks always emit a single `Switch` event,
                // with `switch_outcome` capturing what navigation
                // actually happened. Analysts can count all pill clicks
                // with `action = switch` and slice by outcome — no need
                // to UNION with `FocusOpenedConversation` (which is
                // reserved for the menu-driven "Focus pane" gesture).
                let outcome = if is_open_elsewhere {
                    PillSwitchOutcome::FocusedExistingPane
                } else {
                    PillSwitchOutcome::SwitchedInPlace
                };
                let telemetry_kind = if *is_breadcrumb {
                    PillBarPillKind::Breadcrumb
                } else {
                    pill_kind.telemetry_kind()
                };
                self.emit_pill_switch(telemetry_kind, id, outcome, ctx);
                if is_open_elsewhere {
                    self.navigate_to_conversation_pane(id, ctx);
                } else {
                    ctx.dispatch_typed_action(
                        &PaneHeaderAction::<TerminalAction, TerminalAction>::CustomAction(
                            navigation_action_for_pill(*pill_kind, id),
                        ),
                    );
                }
            }
            OrchestrationPillBarAction::FocusOpenedConversation(id) => {
                self.emit_pill_bar_interaction(
                    PillBarActionKind::FocusOpenedConversation,
                    PillBarPillKind::Child,
                    *id,
                    ctx,
                );
                self.close_menu(ctx);
                self.navigate_to_conversation_pane(*id, ctx);
            }
        }
    }
}

impl View for OrchestrationPillBar {
    fn ui_name() -> &'static str {
        "OrchestrationPillBar"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let Some(PillBarContents {
            anchor_id,
            breadcrumb_root_id,
            breadcrumb_parent_id,
            specs,
        }) = self.pill_specs(app)
        else {
            return Empty::new().finish();
        };

        // Row reports its intrinsic width so the wrapping horizontal
        // scrollable below has something larger than the pane width to
        // pan through when there are many child pills.
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_spacing(PILL_ROW_GAP);

        // Resolve a persistent `MouseStateHandle` for each pill. If `ensure_mouse_states`
        // has not yet seen this id (e.g. mid-event-propagation race), insert a
        // freshly defaulted handle into our `mouse_states` map and reuse it on
        // subsequent renders. Falling back to a transient
        // `MouseStateHandle::default()` here would silently break clicks: the
        // mouse-down notify would re-enter `render` with yet another fresh
        // handle, and mouse-up would land on a different handle than
        // mouse-down.
        let mut mouse_states = self.mouse_states.borrow_mut();
        let mut overflow_states = self.overflow_button_mouse_states.borrow_mut();
        let mut pin_states = self.pin_button_mouse_states.borrow_mut();
        let menu_open_for = self.menu_open_for;
        // Cache this view's terminal_view_id once so each pill click can
        // cheaply check whether its target conversation is currently
        // owned by *another* terminal view. The pill bar renders inside
        // the orchestrator pane, so any child whose owner differs from
        // this id has been split off into another pane/tab.
        let self_terminal_view_id = self.agent_view_controller.as_ref(app).terminal_view_id();
        // Row layout: orchestrator, pinned, divider, unpinned. `pill_specs`
        // already follows the canonical pill order, so partitioning preserves
        // the exact order used by keyboard navigation.
        let mut orchestrator_pill: Option<Box<dyn Element>> = None;
        let mut pinned_pills: Vec<Box<dyn Element>> = Vec::new();
        let mut unpinned_pills: Vec<Box<dyn Element>> = Vec::new();
        // Leading breadcrumbs while drilled into a sub-level of the
        // orchestration tree: the root first, then the anchor's direct
        // parent when it is an intermediate level of its own.
        let breadcrumb_pills: Vec<Box<dyn Element>> = [
            breadcrumb_root_id.map(|root_id| (root_id, PillKind::Orchestrator)),
            breadcrumb_parent_id.map(|parent_id| (parent_id, PillKind::Child)),
        ]
        .into_iter()
        .flatten()
        .map(|(target_id, pill_kind)| {
            let mouse_state = mouse_states.entry(target_id).or_default().clone();
            render_breadcrumb_pill(target_id, pill_kind, mouse_state, app)
        })
        .collect();
        for spec in specs {
            let mouse_state = mouse_states
                .entry(spec.conversation_id)
                .or_default()
                .clone();
            // Each child pill gets its own dedicated 3-dot button mouse
            // state so hover highlight on the button is independent of the
            // pill body. Orchestrator pills don't get a 3-dot button (no
            // overflow actions apply to the home view), so we still create
            // the entry for layout symmetry but won't render the button.
            let overflow_mouse_state = overflow_states
                .entry(spec.conversation_id)
                .or_default()
                .clone();
            // Orchestrator pills don't render a pin button but we keep an
            // entry for symmetry.
            let pin_mouse_state = pin_states.entry(spec.conversation_id).or_default().clone();
            let menu_is_open_for_this = menu_open_for == Some(spec.conversation_id);
            let kind = spec.kind;
            let pin_state = spec.pin_state;
            let pill = render_pill(
                spec,
                mouse_state,
                overflow_mouse_state,
                pin_mouse_state,
                menu_is_open_for_this,
                self_terminal_view_id,
                app,
            );
            match (kind, pin_state) {
                (PillKind::Orchestrator, _) => orchestrator_pill = Some(pill),
                (PillKind::Child, PillPinState::Pinned) => {
                    pinned_pills.push(pill);
                }
                (PillKind::Child, PillPinState::Unpinned) => {
                    unpinned_pills.push(pill);
                }
            }
        }
        drop(mouse_states);
        drop(overflow_states);
        drop(pin_states);

        for pill in breadcrumb_pills {
            row.add_child(pill);
        }
        if let Some(pill) = orchestrator_pill {
            row.add_child(pill);
        }
        let has_unpinned = !unpinned_pills.is_empty();
        for pill in pinned_pills {
            row.add_child(pill);
        }
        // Divider between leading section (orchestrator + pinned) and unpinned.
        if has_unpinned {
            row.add_child(render_pinned_divider(app));
        }
        for pill in unpinned_pills {
            row.add_child(pill);
        }

        // Pan + clip the pill row when it overflows the pane. The scroll
        // handle is keyed by orchestrator id and shared across sibling
        // panes so the user's scroll position survives navigating between
        // pill bars rendered for the same orchestration tree.
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let horizontal_scroll_state =
            OrchestrationPillBarModel::as_ref(app).horizontal_scroll_state_for(anchor_id);
        let scrollable = NewScrollable::horizontal(
            SingleAxisConfig::Clipped {
                handle: horizontal_scroll_state,
                // Gutter goes inside the scrollable; outer padding can't clear
                // the scrollbar since it sits outside the scrollbar's track.
                child: Container::new(row.finish())
                    .with_padding_bottom(PILL_BAR_SCROLLBAR_GUTTER)
                    .finish(),
            },
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            ElementFill::None,
        )
        // Overlaid so the bar height stays constant whether or not it overflows.
        .with_horizontal_scrollbar(ScrollableAppearance::new(
            ScrollbarWidth::Custom(PILL_BAR_SCROLLBAR_WIDTH),
            true,
        ))
        // Let a standard vertical mouse wheel pan the bar horizontally;
        // trackpad horizontal swipes already work through the default path.
        .with_remap_cross_axis_wheel_to_main_axis(true)
        .with_propagate_mousewheel_if_not_handled(true)
        .finish();

        // L/R padding outside so it doesn't scroll with the content; bottom is
        // small since the scrollbar gutter is handled inside the scrollable.
        let bar = Container::new(scrollable)
            .with_padding_left(12.)
            .with_padding_right(12.)
            .with_padding_top(4.)
            .with_padding_bottom(2.)
            .finish();

        // When the 3-dot menu is open, overlay it directly beneath the
        // clicked pill's overflow button. We anchor to the saved position id
        // associated with that button (`overflow_button_position_id(id)`,
        // registered via `SavePosition` in `render_overflow_button`), aligning
        // the menu's top-right corner to the button's bottom-right so the menu
        // tucks neatly under the trailing edge of the pill, regardless of how
        // far across the bar that pill happens to be rendered.
        //
        // Otherwise, when no menu is open but a pill is hovered, we overlay
        // the hover details card under that pill instead. The two overlays
        // are mutually exclusive by design: opening the menu clears
        // `hovered_pill` (see `open_menu_for`).
        //
        // Defensive: only render the hover card if the cursor is still
        // genuinely over the pill. The `SetHoveredPill(None)` action that
        // the pill's `on_hover` callback dispatches when the cursor leaves
        // can be missed in edge cases (window-focus changes, layout drops
        // mid-hover, the cursor exiting the app entirely, or a re-entry
        // into a stale Hoverable that suppresses the synthetic
        // hover-out), leaving `hovered_pill` stuck on a stale id and the
        // card visible until something else triggers a re-render. Reading
        // `MouseState::is_mouse_over_element` directly at render time
        // makes the overlay strictly track the cursor: as soon as the
        // pointer moves off the pill, the next render hides the card,
        // regardless of whether the typed-action callback fired.
        let overlay = if let Some(target_id) = self.menu_open_for {
            Some(MenuOrCard::Menu(target_id))
        } else {
            self.hovered_pill.and_then(|id| {
                let mouse_states = self.mouse_states.borrow();
                let still_over_pill = mouse_states
                    .get(&id)
                    .and_then(|handle| handle.lock().ok().map(|s| s.is_mouse_over_element()))
                    .unwrap_or(false);
                drop(mouse_states);
                if !still_over_pill {
                    return None;
                }
                render_hover_card(id, self.agent_view_controller.as_ref(app), app)
                    .map(|card| MenuOrCard::Card { id, card })
            })
        };

        match overlay {
            Some(MenuOrCard::Menu(target_id)) => {
                let mut stack = Stack::new();
                stack.add_child(bar);
                let position_id = overflow_button_position_id(target_id);
                stack.add_positioned_overlay_child(
                    ChildView::new(&self.menu).finish(),
                    OffsetPositioning::from_axes(
                        PositioningAxis::relative_to_stack_child(
                            &position_id,
                            PositionedElementOffsetBounds::WindowByPosition,
                            OffsetType::Pixel(0.),
                            AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Right),
                        )
                        .with_conditional_anchor(),
                        PositioningAxis::relative_to_stack_child(
                            &position_id,
                            PositionedElementOffsetBounds::WindowByPosition,
                            OffsetType::Pixel(4.),
                            AnchorPair::new(YAxisAnchor::Bottom, YAxisAnchor::Top),
                        )
                        .with_conditional_anchor(),
                    ),
                );
                stack.finish()
            }
            Some(MenuOrCard::Card { id, card }) => {
                let mut stack = Stack::new();
                stack.add_child(bar);
                let position_id = pill_body_position_id(id);
                stack.add_positioned_overlay_child(
                    card,
                    OffsetPositioning::from_axes(
                        PositioningAxis::relative_to_stack_child(
                            &position_id,
                            PositionedElementOffsetBounds::WindowByPosition,
                            OffsetType::Pixel(0.),
                            AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                        )
                        .with_conditional_anchor(),
                        PositioningAxis::relative_to_stack_child(
                            &position_id,
                            PositionedElementOffsetBounds::WindowByPosition,
                            OffsetType::Pixel(6.),
                            AnchorPair::new(YAxisAnchor::Bottom, YAxisAnchor::Top),
                        )
                        .with_conditional_anchor(),
                    ),
                );
                stack.finish()
            }
            None => bar,
        }
    }
}

/// Local enum used by `View::render` to model the at-most-one overlay
/// rendered on top of the pill bar (the 3-dot menu *or* the hover details
/// card). Wrapping these in one enum keeps the positioning logic in a
/// single match arm rather than two near-duplicate `if let` branches.
enum MenuOrCard {
    Menu(AIConversationId),
    Card {
        id: AIConversationId,
        card: Box<dyn Element>,
    },
}

/// Builds the hover details card overlay for the given conversation, or
/// returns `None` if there's no conversation to summarise (e.g. the id
/// has just been removed from history).
///
/// V1 scope keeps the card pragmatic: title + description + a compact
/// chips row showing the agent's harness (placeholder for now), branch
/// (from any PR artifact), and a clickable-looking PR chip. We hide chips
/// whose data is not available rather than showing empty placeholders.
fn render_hover_card(
    conversation_id: AIConversationId,
    _agent_view_controller: &AgentViewController,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let history = BlocklistAIHistoryModel::as_ref(app);
    let conversation = history.conversation(&conversation_id)?;

    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let bg = theme.surface_2();
    let main_text = internal_colors::text_main(theme, bg);
    let sub_text = internal_colors::text_sub(theme, bg);
    let outline = theme.outline();

    let name = conversation
        .agent_name()
        .filter(|n| !n.is_empty())
        .map(|n| n.to_string())
        .or_else(|| conversation.title())
        .unwrap_or_else(|| "Agent".to_string());

    // Header: small avatar disc + bold agent name on the left, status
    // badge right-aligned. We use the conversation's `ConversationStatus`
    // (mapped to icon+color via `status_icon_and_color`) to drive the
    // badge so the card matches the colors used elsewhere in the agent
    // details panel.
    let is_orchestrator = conversation.parent_conversation_id().is_none();
    let avatar = if is_orchestrator {
        render_orchestrator_avatar_disc(AVATAR_SIZE, theme, appearance)
    } else {
        render_agent_avatar_disc(&name, AVATAR_SIZE, theme, appearance)
    };
    let name_text = Text::new(
        name,
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(main_text)
    .with_style(Properties {
        weight: Weight::Semibold,
        ..Default::default()
    })
    .with_clip(ClipConfig::ellipsis())
    .soft_wrap(false)
    .finish();
    // Orchestrator hover cards use aggregated tree status; child cards use
    // per-child status.
    let aggregated_status;
    let badge_status: &ConversationStatus = if is_orchestrator {
        aggregated_status = aggregated_orchestrator_status(history, conversation_id);
        &aggregated_status
    } else {
        conversation.status()
    };
    let status_badge = ConstrainedBox::new(render_status_badge(badge_status, theme, appearance))
        .with_max_width(HOVER_CARD_STATUS_BADGE_MAX_WIDTH)
        .finish();
    // Reserve fixed space for the badge so long names ellipsize instead of
    // pushing it off the card.
    let name_max_width = HOVER_CARD_CONTENT_WIDTH
        - AVATAR_SIZE
        - HOVER_CARD_HEADER_AVATAR_NAME_GAP
        - HOVER_CARD_HEADER_NAME_BADGE_GAP
        - HOVER_CARD_STATUS_BADGE_MAX_WIDTH;
    let header = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_child(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(HOVER_CARD_HEADER_AVATAR_NAME_GAP)
                .with_child(avatar)
                .with_child(
                    ConstrainedBox::new(name_text)
                        .with_max_width(name_max_width)
                        .finish(),
                )
                .finish(),
        )
        .with_child(status_badge)
        .finish();

    // Working directory line: pulled from the root task's first exchange
    // when available, falling back to the most recent exchange. Hidden
    // entirely when neither is populated (e.g. cloud agents whose CWD
    // hasn't synced yet).
    //
    // Use `dirs::home_dir()` (cross-platform: `$HOME` on unix,
    // `%USERPROFILE%` on Windows) to find the home prefix, then defer to
    // the shared `warp_util::path::user_friendly_path` helper so the cwd
    // displays as `~/foo` regardless of OS — and matches the same
    // tilde-substitution behaviour used by the tab title, prompt header,
    // and pwd chip.
    let home_dir = dirs::home_dir();
    let home_dir_str = home_dir.as_deref().and_then(|p| p.to_str());
    let cwd_line: Option<Box<dyn Element>> = conversation
        .initial_working_directory()
        .or_else(|| conversation.current_working_directory())
        .filter(|s| !s.is_empty())
        .map(|cwd| {
            Text::new(
                warp_util::path::user_friendly_path(&cwd, home_dir_str).into_owned(),
                appearance.ui_font_family(),
                appearance.monospace_font_size() - 1.,
            )
            .with_color(main_text)
            .with_clip(ClipConfig {
                direction: ClipDirection::Start,
                style: ClipStyle::Ellipsis,
            })
            .soft_wrap(false)
            .finish()
        });

    // Description: title or initial query, truncated visually via wrapping
    // inside a constrained box.
    let description_text = conversation
        .title()
        .filter(|s| !s.is_empty())
        .or_else(|| conversation.initial_query())
        .filter(|s| !s.is_empty());
    let description: Option<Box<dyn Element>> = description_text.map(|description| {
        let trimmed = if description.chars().count() > 200 {
            let truncated: String = description.chars().take(197).collect();
            format!("{truncated}\u{2026}")
        } else {
            description
        };
        Text::new(
            trimmed,
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 1.,
        )
        .with_color(sub_text)
        .soft_wrap(true)
        .finish()
    });

    // Chips row: branch (if known via a PR artifact) + PR (if known) +
    // harness (always when known). Hidden entirely when no chip applies.
    let mut chips: Vec<Box<dyn Element>> = Vec::new();

    // Harness chip: prefer the spawn-time `orchestration_harness_type`
    // so child agents report their harness immediately; fall back to
    // Oz so the chip slot stays populated.
    let harness = conversation.orchestration_harness().unwrap_or(Harness::Oz);
    let harness_icon = harness_display::icon_for(harness);
    let harness_label = harness_display::display_name(harness).to_string();
    let harness_color = harness_display::brand_color(harness).unwrap_or(sub_text);
    chips.push(render_chip(
        harness_icon,
        harness_label,
        harness_color,
        main_text,
        theme,
        appearance,
    ));

    for artifact in conversation.artifacts() {
        if let Artifact::PullRequest {
            url: _,
            branch,
            repo,
            number,
        } = artifact
        {
            if !branch.is_empty() {
                chips.push(render_chip(
                    Icon::GitBranch,
                    branch.clone(),
                    sub_text,
                    main_text,
                    theme,
                    appearance,
                ));
            }
            if let (Some(repo), Some(number)) = (repo, number) {
                chips.push(render_chip(
                    Icon::Github,
                    format!("{repo}#{number}"),
                    sub_text,
                    main_text,
                    theme,
                    appearance,
                ));
            }
            // Only one PR artifact is meaningful per conversation; bail.
            break;
        }
    }

    // Assemble.
    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(8.)
        .with_child(header);
    if let Some(cwd_line) = cwd_line {
        column = column.with_child(
            ConstrainedBox::new(cwd_line)
                .with_max_width(HOVER_CARD_CONTENT_WIDTH)
                .finish(),
        );
    }
    if let Some(description) = description {
        column = column.with_child(
            ConstrainedBox::new(description)
                .with_max_width(HOVER_CARD_CONTENT_WIDTH)
                .finish(),
        );
    }
    if !chips.is_empty() {
        let mut chip_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(6.);
        for chip in chips {
            chip_row = chip_row.with_child(chip);
        }
        column = column.with_child(chip_row.finish());
    }

    let card = Container::new(column.finish())
        .with_padding_left(HOVER_CARD_HORIZONTAL_PADDING)
        .with_padding_right(HOVER_CARD_HORIZONTAL_PADDING)
        .with_padding_top(HOVER_CARD_VERTICAL_PADDING)
        .with_padding_bottom(HOVER_CARD_VERTICAL_PADDING)
        .with_background(bg)
        .with_border(warpui::elements::Border::all(1.).with_border_fill(outline))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .finish();

    Some(
        ConstrainedBox::new(card)
            .with_width(HOVER_CARD_WIDTH)
            .finish(),
    )
}

/// Renders the colored "Working / Done / Error / Cancelled / Blocked"
/// status badge that sits in the top-right of the hover card. Mirrors
/// the visual treatment in `conversation_details_panel::render_status_section`
/// (icon + label, tinted with the same opacity•10 chip background) so
/// the card and the side panel can't drift.
fn render_status_badge(
    status: &ConversationStatus,
    theme: &WarpTheme,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let (icon, color) = status.status_icon_and_color(theme, StatusColorStyle::Standard);
    let icon_el = ConstrainedBox::new(icon.to_warpui_icon(color.into()).finish())
        .with_width(12.)
        .with_height(12.)
        .finish();
    let label = Text::new(
        status.to_string(),
        appearance.ui_font_family(),
        appearance.monospace_font_size() - 2.,
    )
    .with_color(color)
    .soft_wrap(false)
    .finish();
    let row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(4.)
        .with_child(icon_el)
        .with_child(label)
        .finish();
    Container::new(row)
        .with_padding_left(6.)
        .with_padding_right(6.)
        .with_padding_top(2.)
        .with_padding_bottom(2.)
        .with_background_color(coloru_with_opacity(color, 10))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .finish()
}

/// Places a pill's leading avatar content — an
/// [`AVATAR_WITH_STATUS_TOTAL_SIZE`] box built by [`render_avatar_lockup_box`],
/// with or without a status badge layered on it — in the fixed-width leading
/// slot. The slot spans the full pill height so hover swaps (avatar ↔ pin
/// button) never shift the label.
///
/// The box is bottom-aligned rather than centered: the status badge hangs off
/// its bottom-right corner and design wants that badge flush with the pill's
/// bottom edge, so the box's bottom has to be the pill's bottom.
fn render_avatar_slot(avatar: Box<dyn Element>) -> Box<dyn Element> {
    ConstrainedBox::new(Align::new(avatar).bottom_left().finish())
        .with_width(PILL_AVATAR_SLOT_SIZE)
        .with_height(PILL_HEIGHT)
        .finish()
}

/// Renders a small icon + label chip used inside the hover details card.
fn render_chip(
    icon: Icon,
    label: String,
    icon_color: ColorU,
    text_color: ColorU,
    theme: &WarpTheme,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let icon_el = ConstrainedBox::new(icon.to_warpui_icon(icon_color.into()).finish())
        .with_width(12.)
        .with_height(12.)
        .finish();
    let text = Text::new(
        label,
        appearance.ui_font_family(),
        appearance.monospace_font_size() - 2.,
    )
    .with_color(text_color)
    .soft_wrap(false)
    .with_clip(ClipConfig::ellipsis())
    .finish();
    let row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(4.)
        .with_child(icon_el)
        .with_child(
            ConstrainedBox::new(text)
                .with_max_width(HOVER_CARD_WIDTH - 60.)
                .finish(),
        )
        .finish();
    Container::new(row)
        .with_padding_left(6.)
        .with_padding_right(6.)
        .with_padding_top(2.)
        .with_padding_bottom(2.)
        .with_background(internal_colors::neutral_2(theme))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .finish()
}

fn navigation_action_for_pill(kind: PillKind, conversation_id: AIConversationId) -> TerminalAction {
    match kind {
        // The orchestrator pill is the "home" conversation for the tree, so
        // navigating back to it should switch the current pane's agent view.
        PillKind::Orchestrator => TerminalAction::SwitchAgentViewToConversation { conversation_id },
        // Child conversations already have a dedicated hidden pane/session
        // created at StartAgent time. Revealing that pane keeps any live
        // harness session, CLI listener, ambient-agent session state, and PTY
        // output attached to the real owner instead of trying to recreate the
        // child transcript in the current pane.
        PillKind::Child => TerminalAction::RevealChildAgent { conversation_id },
    }
}

/// Leading breadcrumb pill shown while the bar is drilled into a sub-level
/// of the orchestration tree. Clicking it navigates to `target_id` — the
/// tree root (`PillKind::Orchestrator`) or an intermediate parent
/// (`PillKind::Child`, revealed via its hidden pane like any child pill).
fn render_breadcrumb_pill(
    target_id: AIConversationId,
    pill_kind: PillKind,
    mouse_state: MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let pill_rest_bg = theme
        .background()
        .blend(&internal_colors::fg_overlay_2(theme))
        .into_solid();
    let pill_hover_bg = theme
        .background()
        .blend(&internal_colors::fg_overlay_3(theme))
        .into_solid();
    let text_color = internal_colors::fg_overlay_6(theme).into_solid();
    let label = BlocklistAIHistoryModel::as_ref(app)
        .conversation(&target_id)
        .map(|conversation| match pill_kind {
            PillKind::Orchestrator => orchestrator_label(conversation),
            PillKind::Child => conversation
                .agent_name()
                .filter(|name| !name.is_empty())
                .unwrap_or("Agent")
                .to_string(),
        })
        .unwrap_or_else(|| "Orchestrator".to_string());

    Hoverable::new(mouse_state, move |hover_state| {
        let background = if hover_state.is_hovered() || hover_state.is_clicked() {
            pill_hover_bg
        } else {
            pill_rest_bg
        };
        let chevron =
            ConstrainedBox::new(Icon::ChevronLeft.to_warpui_icon(text_color.into()).finish())
                .with_width(PILL_ICON_SIZE)
                .with_height(PILL_ICON_SIZE)
                .finish();
        let label_text = Text::new(
            label,
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 1.,
        )
        .with_color(text_color)
        .soft_wrap(false)
        .with_clip(ClipConfig::ellipsis())
        .finish();
        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(PILL_CONTENT_GAP)
            .with_child(chevron)
            .with_child(
                ConstrainedBox::new(label_text)
                    .with_max_width(PILL_LABEL_MAX_WIDTH)
                    .finish(),
            )
            .finish();
        ConstrainedBox::new(
            Container::new(row)
                .with_padding_left(PILL_HORIZONTAL_PADDING_LEFT)
                .with_padding_right(PILL_HORIZONTAL_PADDING_RIGHT)
                .with_background_color(background)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(PILL_RADIUS)))
                .finish(),
        )
        .with_height(PILL_HEIGHT)
        .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _app, _| {
        ctx.dispatch_typed_action(OrchestrationPillBarAction::PillClicked {
            conversation_id: target_id,
            pill_kind,
            is_breadcrumb: true,
        });
    })
    .finish()
}

/// Horizontal padding inside the subtree badge around its count text.
const SUBTREE_BADGE_HORIZONTAL_PADDING: f32 = 5.;

/// Fixed slot width for a group pill's subtree badge: fits the count text
/// and is never narrower than the trailing slice the hover 3-dot overlay
/// occupies, so swapping the badge out for the dots never resizes the pill.
fn subtree_rollup_badge_slot_width(
    rollup: &LoadedSubtreeRollup,
    appearance: &Appearance,
    app: &AppContext,
) -> f32 {
    let text_width = pill_label_width(
        &rollup.descendant_count.to_string(),
        appearance.monospace_font_size() - 2.,
        Properties::default(),
        appearance,
        app,
    );
    (text_width + 2. * SUBTREE_BADGE_HORIZONTAL_PADDING).max(OVERFLOW_BUTTON_LABEL_RESERVE)
}

/// Compact trailing badge on a "group" pill: the number of agents in the
/// child's subtree, tinted with the subtree's aggregated status color.
fn render_subtree_rollup_badge(
    rollup: &LoadedSubtreeRollup,
    theme: &WarpTheme,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let (_, color) = rollup
        .status
        .status_icon_and_color(theme, StatusColorStyle::Standard);
    let text = Text::new(
        rollup.descendant_count.to_string(),
        appearance.ui_font_family(),
        appearance.monospace_font_size() - 2.,
    )
    .with_color(color)
    .soft_wrap(false)
    .finish();
    Container::new(text)
        .with_padding_left(SUBTREE_BADGE_HORIZONTAL_PADDING)
        .with_padding_right(SUBTREE_BADGE_HORIZONTAL_PADDING)
        .with_padding_top(1.)
        .with_padding_bottom(1.)
        .with_background_color(coloru_with_opacity(color, 10))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(PILL_RADIUS)))
        .finish()
}

/// 1px vertical divider between the pinned and unpinned sections.
fn render_pinned_divider(app: &AppContext) -> Box<dyn Element> {
    const DIVIDER_HEIGHT: f32 = 16.;
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    ConstrainedBox::new(
        Container::new(Empty::new().finish())
            .with_background(internal_colors::fg_overlay_3(theme))
            .finish(),
    )
    .with_width(1.)
    .with_height(DIVIDER_HEIGHT)
    .finish()
}

/// The clickable pin button a child pill shows in place of its avatar while
/// hovered: a circle occupying exactly the avatar disc's rect.
///
/// Placement deliberately goes through the same
/// [`render_avatar_slot`] / [`render_avatar_lockup_box`] pair the disc itself
/// uses, and the circle is sized off [`PILL_AVATAR_DISC_SIZE`], so the swap
/// cannot shift by a pixel and the two cannot drift apart if the disc's
/// geometry is ever retuned.
///
/// The glyph is placed by explicit padding rather than by a centering
/// container, which keeps it exact regardless of how the surrounding box
/// behaves, and lets [`PIN_GLYPH_OPTICAL_DROP`] bias it downward without
/// moving the circle.
fn render_pin_button(
    is_pinned: bool,
    icon_color: ColorU,
    mouse_state: MouseStateHandle,
    conversation_id: AIConversationId,
) -> Box<dyn Element> {
    // Tint with the pill's own contrasting colour rather than a fixed
    // foreground overlay. `fg_overlay_1` is the foreground at 5% opacity, and a
    // selected pill's background *is* the foreground colour, so the old fill
    // painted a colour onto itself and the hover state was invisible on every
    // selected chip — which, since the bar anchors on the parent of whatever
    // leaf you are viewing, is the common case rather than an edge case.
    let hover_background = coloru_with_opacity(icon_color, PIN_BUTTON_HOVER_OPACITY);
    let glyph_size = PILL_AVATAR_DISC_SIZE * PIN_GLYPH_RATIO;
    let icon = if is_pinned {
        Icon::PinFilled
    } else {
        Icon::Pin
    };
    let button = Hoverable::new(mouse_state, move |hover_state| {
        let glyph = ConstrainedBox::new(icon.to_warpui_icon(icon_color.into()).finish())
            .with_width(glyph_size)
            .with_height(glyph_size)
            .finish();
        // Top and bottom padding still sum to twice `padding`, so the circle
        // keeps the avatar disc's rect exactly; only the glyph inside it moves.
        let padding = (PILL_AVATAR_DISC_SIZE - glyph_size) / 2.;
        let mut circle = Container::new(glyph)
            .with_horizontal_padding(padding)
            .with_padding_top(padding + PIN_GLYPH_OPTICAL_DROP)
            .with_padding_bottom(padding - PIN_GLYPH_OPTICAL_DROP)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                PILL_AVATAR_DISC_SIZE / 2.,
            )));
        if hover_state.is_hovered() || hover_state.is_clicked() {
            circle = circle.with_background(hover_background);
        }
        circle.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _app, _| {
        ctx.dispatch_typed_action(OrchestrationPillBarAction::TogglePin(conversation_id));
    })
    .finish();
    render_avatar_slot(render_avatar_lockup_box(button))
}

fn render_pill(
    spec: PillSpec,
    mouse_state: MouseStateHandle,
    overflow_mouse_state: MouseStateHandle,
    pin_button_mouse_state: MouseStateHandle,
    menu_is_open_for_this: bool,
    self_terminal_view_id: warpui::EntityId,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let conversation_id = spec.conversation_id;
    let kind = spec.kind;
    let is_selected = spec.is_selected;
    let pin_state = spec.pin_state;
    let is_pinned = matches!(pin_state, PillPinState::Pinned);
    // The 3-dot overflow menu offers pane-management actions (open in new
    // pane / tab, focus pane) that don't apply to the single-pane web
    // viewer. Suppress the dots on WASM so the menu can never open.
    #[cfg(not(target_family = "wasm"))]
    let show_overflow_button = matches!(kind, PillKind::Child);
    #[cfg(target_family = "wasm")]
    let show_overflow_button = {
        let _ = &kind;
        false
    };
    // Orchestrator is always anchored at the leading edge with no pin.
    let supports_pinning = matches!(kind, PillKind::Child);
    // `spec` is owned by value, so we can move `label` directly into the
    // build closure below without cloning.
    let label = spec.label;
    let avatar_color = spec.avatar_color;
    let avatar_glyph = spec.avatar_glyph;
    let status = spec.status;
    let is_remote_child = spec.is_remote_child;
    let subtree_rollup = spec.subtree_rollup;

    // Per Figma: fg_overlay_2 at rest, fg_overlay_3 on hover, composed over
    // the theme background. Pre-blend to a solid so the avatar cutout ring
    // matches the painted pill exactly.
    let pill_rest_bg = theme
        .background()
        .blend(&internal_colors::fg_overlay_2(theme))
        .into_solid();
    let pill_hover_bg = theme
        .background()
        .blend(&internal_colors::fg_overlay_3(theme))
        .into_solid();
    let pill_text_color = internal_colors::fg_overlay_6(theme).into_solid();

    // `Hoverable::new`'s build closure is `FnOnce` (see
    // `crates/warpui_core/src/elements/hoverable.rs`). We can therefore move
    // `label` into the closure by value rather than cloning it on every
    // build.
    let pill_body = Hoverable::new(mouse_state, move |hover_state| {
        // Highlight the pill only when it's the currently active
        // conversation. Opening the 3-dot menu on a *non-active* pill
        // should not change that pill's appearance — the menu itself is
        // a separate overlay and the user expects only the truly
        // selected agent's pill to read as selected.
        let (background, text_color) = if is_selected {
            (
                theme.foreground().into_solid(),
                theme.background().into_solid(),
            )
        } else if hover_state.is_hovered() || hover_state.is_clicked() || menu_is_open_for_this {
            (pill_hover_bg, pill_text_color)
        } else {
            (pill_rest_bg, pill_text_color)
        };

        let show_dots = show_overflow_button && (hover_state.is_hovered() || menu_is_open_for_this);
        let label_style = Properties {
            weight: Weight::Normal,
            ..Default::default()
        };
        // At rest, labels use the full budget. When dots are visible, keep
        // the rest-state slot width but reserve its trailing slice so the
        // overlay does not cover glyphs or shift sibling pills. Group pills
        // skip the label reserve: their trailing badge slot (below) already
        // absorbs the overlay.
        let hover_label_slot_width = (show_dots && subtree_rollup.is_none()).then(|| {
            pill_label_width(
                &label,
                appearance.monospace_font_size() - 1.,
                label_style,
                appearance,
                app,
            )
            .min(PILL_LABEL_MAX_WIDTH)
        });

        let label_text = Text::new(
            label,
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 1.,
        )
        .with_color(text_color)
        .soft_wrap(false)
        .with_clip(ClipConfig::ellipsis())
        .with_style(label_style)
        .finish();

        let label_element = if let Some(label_slot_width) = hover_label_slot_width {
            let clipped_label_width = (label_slot_width - OVERFLOW_BUTTON_LABEL_RESERVE).max(0.);
            let spacer_width = label_slot_width - clipped_label_width;
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(
                    ConstrainedBox::new(label_text)
                        .with_max_width(clipped_label_width)
                        .finish(),
                )
                .with_child(
                    ConstrainedBox::new(Empty::new().finish())
                        .with_width(spacer_width)
                        .finish(),
                )
                .finish()
        } else {
            ConstrainedBox::new(label_text)
                .with_max_width(PILL_LABEL_MAX_WIDTH)
                .finish()
        };

        // Child pills swap the leading avatar for a clickable pin glyph on
        // pill hover. Pin state is communicated by position (left of the
        // divider), not the icon — the glyph only appears on hover.
        let outer_pill_hovered = hover_state.is_hovered() || hover_state.is_clicked();
        let show_pin_glyph = supports_pinning && outer_pill_hovered;
        let leading: Box<dyn Element> = match kind {
            PillKind::Orchestrator => match status.as_ref() {
                Some(status) => render_avatar_with_status_overlay(
                    avatar_color,
                    avatar_glyph,
                    status.clone(),
                    is_remote_child,
                    background,
                    theme,
                    appearance,
                ),
                None => render_pill_avatar(avatar_color, avatar_glyph, theme, appearance),
            },
            PillKind::Child => {
                if show_pin_glyph {
                    // Hovered: the leading slot becomes the clickable pin
                    // button. The Hoverable + TogglePin click handler is
                    // attached only here so that when the avatar is the
                    // visible content (not hovered), clicks bubble up to
                    // the outer pill body and navigate as expected.
                    render_pin_button(
                        is_pinned,
                        text_color,
                        pin_button_mouse_state.clone(),
                        conversation_id,
                    )
                } else if let Some(ref status) = status {
                    render_avatar_with_status_overlay(
                        avatar_color,
                        avatar_glyph,
                        status.clone(),
                        is_remote_child,
                        background,
                        theme,
                        appearance,
                    )
                } else {
                    render_pill_avatar(avatar_color, avatar_glyph, theme, appearance)
                }
            }
        };
        let leading_label_spacing = if show_pin_glyph && is_selected {
            PILL_SELECTED_HOVER_CONTENT_GAP
        } else {
            PILL_CONTENT_GAP
        };

        // Body row contains just the avatar + label — the 3-dot button
        // is rendered as a positioned overlay (below) so it doesn't take
        // a slot in this row. That means the pill's intrinsic width is
        // determined by the label alone, and the dots can visually clip
        // the trailing edge of the text when shown without making the
        // pill itself wider or shifting siblings.
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(leading_label_spacing)
            .with_child(leading)
            .with_child(label_element);
        // Group pills append a rolled-up subtree badge after the label. The
        // badge occupies a fixed-width slot; while the 3-dot overlay is
        // shown the slot renders empty so the dots never overlap the badge,
        // and the pill's width stays constant across the swap.
        if let Some(rollup) = &subtree_rollup {
            let slot_width = subtree_rollup_badge_slot_width(rollup, appearance, app);
            let slot_content: Box<dyn Element> = if show_dots {
                Empty::new().finish()
            } else {
                render_subtree_rollup_badge(rollup, theme, appearance)
            };
            row = row.with_child(
                ConstrainedBox::new(Align::new(slot_content).finish())
                    .with_width(slot_width)
                    .finish(),
            );
        }
        let row = row.finish();

        // Constrain pill to a fixed height so the half-stadium corner radius
        // renders as a clean continuous shape rather than awkwardly clamping.
        let pill_inner = ConstrainedBox::new(
            Container::new(row)
                .with_padding_left(PILL_HORIZONTAL_PADDING_LEFT)
                .with_padding_right(PILL_HORIZONTAL_PADDING_RIGHT)
                .with_background_color(background)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(PILL_RADIUS)))
                .finish(),
        )
        .with_height(PILL_HEIGHT)
        .finish();

        // Render the 3-dot button as a positioned overlay only when the
        // pill is being hovered (or its 3-dot menu is already open). The
        // overlay sits at the trailing edge of the pill; the label row
        // reserves matching space only while the dots are visible.
        if show_dots {
            let mut stack = Stack::new();
            stack.add_child(pill_inner);
            stack.add_positioned_child(
                render_overflow_button(
                    overflow_mouse_state.clone(),
                    conversation_id,
                    text_color.into(),
                    theme,
                ),
                OffsetPositioning::offset_from_parent(
                    vec2f(-PILL_OVERFLOW_BUTTON_RIGHT_OFFSET, 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::MiddleRight,
                    ChildAnchor::MiddleRight,
                ),
            );
            stack.finish()
        } else {
            pill_inner
        }
    })
    .with_cursor(if is_selected {
        Cursor::Arrow
    } else {
        Cursor::PointingHand
    })
    // The 3-dot overflow button is a child Hoverable on top of this one.
    // Without `defer_events_to_children`, both the inner and outer click
    // handlers fire for the same mouse-up — the overflow button opens the
    // menu *and* the pill body switches the agent view in place. Defer
    // skips the outer click whenever a child already handled it so the
    // 3-dot click only opens the menu.
    .with_defer_events_to_children()
    .on_hover(move |is_hovered, ctx, _app, _pos| {
        // Drive the hover-details-card overlay via a typed action so the
        // pill bar's `handle_action` can update its `hovered_pill` field
        // and re-render. We pass the conversation id on hover-in and
        // `None` on hover-out; if the pointer scrubs from one pill to
        // another the new pill's hover-in arrives after this pill's
        // hover-out, so the action surface still ends up with the
        // correct id.
        let payload = if is_hovered {
            Some(conversation_id)
        } else {
            None
        };
        ctx.dispatch_typed_action(OrchestrationPillBarAction::SetHoveredPill(payload));
    })
    .on_click(move |ctx, _app, _| {
        if is_selected {
            return;
        }
        // Route the click through `PillClicked` so the pill bar can
        // emit telemetry before forwarding the navigation. The
        // handler reads `self_terminal_view_id` from its own
        // controller, so we no longer need the value captured here.
        let _ = self_terminal_view_id;
        ctx.dispatch_typed_action(OrchestrationPillBarAction::PillClicked {
            conversation_id,
            pill_kind: kind,
            is_breadcrumb: false,
        });
    })
    .on_right_click(move |ctx, _app, _| {
        // Right-clicking a child pill should expose the same overflow
        // actions as clicking the trailing 3-dot button. The menu is still
        // anchored to that button's saved position: opening the menu forces
        // `show_dots`, so the next render creates the anchor before the menu
        // overlay is positioned.
        if show_overflow_button {
            ctx.dispatch_typed_action(OrchestrationPillBarAction::OpenMenu(conversation_id));
        }
    })
    .finish();

    // Cache the painted rect of this pill body under a stable id so the
    // hover details card overlay (rendered as a positioned overlay sibling
    // of the bar in `View::render`) can anchor relative to it without
    // having to know which index this pill ended up at in the row.
    SavePosition::new(pill_body, &pill_body_position_id(conversation_id)).finish()
}

/// Renders the trailing 3-dot button on a child pill. Click dispatches
/// `OrchestrationPillBarAction::OpenMenu(conversation_id)` as a typed action
/// up to the pill bar's `handle_action`, which rebuilds the menu items for
/// that child id and toggles `menu_open_for` on. We use a separate inner
/// `Hoverable` so the button has its own hover highlight independent of the
/// surrounding pill body.
///
/// The button is wrapped in a `SavePosition` so the open menu can anchor
/// itself directly beneath this specific pill's button (see
/// `View::render`); without the saved position, the menu would have to fall
/// back to a bar-relative offset which doesn't track which pill is active.
fn render_overflow_button(
    mouse_state: MouseStateHandle,
    conversation_id: AIConversationId,
    text_color: Fill,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    let button = Hoverable::new(mouse_state, move |hover_state| {
        // The button's own surface gets a subtle filled background when
        // hovered or pressed so it reads as a discrete clickable target
        // even though it sits on top of the pill body's own highlight.
        let bg = if hover_state.is_hovered() || hover_state.is_clicked() {
            Some(internal_colors::fg_overlay_1(theme))
        } else {
            None
        };
        let icon = ConstrainedBox::new(Icon::DotsVertical.to_warpui_icon(text_color).finish())
            .with_width(PILL_ICON_SIZE)
            .with_height(PILL_ICON_SIZE)
            .finish();
        let mut container = Container::new(Align::new(icon).finish())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
        if let Some(bg) = bg {
            container = container.with_background(bg);
        }
        ConstrainedBox::new(container.finish())
            .with_width(OVERFLOW_BUTTON_SIZE)
            .with_height(OVERFLOW_BUTTON_SIZE)
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _app, _| {
        // The outer pill body is configured with
        // `with_defer_events_to_children`, so this child Hoverable is
        // allowed to consume the click event and the outer pill's
        // `SwitchAgentViewToConversation` handler is skipped for the
        // same mouse-up. That keeps a click on the dots strictly to
        // "open the menu" without also switching the agent view.
        ctx.dispatch_typed_action(OrchestrationPillBarAction::OpenMenu(conversation_id));
    })
    .finish();

    // Cache this button's painted rect under a stable id so the open menu
    // (rendered as a positioned overlay sibling of the bar in `View::render`)
    // can anchor relative to it.
    SavePosition::new(button, &overflow_button_position_id(conversation_id)).finish()
}

/// Pin glyph size as a fraction of the avatar disc it sits in. Calibrated
/// against the letter it replaces: matching the letter's ink height exactly
/// read too *small*, because a thin outline carries less visual weight than a
/// solid letterform, so design asked for roughly 4px more. This is the knob to
/// nudge if it still reads wrong.
const PIN_GLYPH_RATIO: f32 = 0.71;

/// Downward nudge of the pin glyph inside its circle.
///
/// This is an *optical* correction, not a geometry one — do not "fix" it to
/// zero because the arithmetic says centered. The pin's mass is concentrated
/// in its head, so a geometrically centered glyph reads as sitting high.
///
/// Absolute pixels rather than a ratio because the pin is only ever drawn in
/// the chip's [`PILL_AVATAR_DISC_SIZE`] circle. If it gains another size, this
/// needs revisiting rather than silently scaling.
const PIN_GLYPH_OPTICAL_DROP: f32 = 1.;

/// Opacity of the pin button's hover tint, over the pill's contrasting colour.
/// A little stronger than the 5% `fg_overlay_1` used to apply, because that
/// colour is nearer the pill's own background than the contrasting one is.
const PIN_BUTTON_HOVER_OPACITY: u8 = 8;

/// Cutout-ring diameter of the status badge, per design.
const PILL_BADGE_RING_SIZE: f32 = 11.;
/// Bounding box of the status icon inside that ring, per design. The 1px it
/// leaves on each side is the visible cutout.
const PILL_BADGE_ICON_SIZE: f32 = 9.;

/// `icon_with_status` expresses badge geometry as fractions of the box the
/// badge is anchored to, so convert the designed absolute sizes once here
/// rather than restating them as ratios.
const PILL_BADGE_STYLE: StatusBadgeStyle = StatusBadgeStyle {
    ring_ratio: PILL_BADGE_RING_SIZE / AVATAR_WITH_STATUS_TOTAL_SIZE,
    icon_ratio: PILL_BADGE_ICON_SIZE / AVATAR_WITH_STATUS_TOTAL_SIZE,
    inner_shape: BadgeInnerShape::RoundedSquare { radius_px: 2.0 },
};

/// Extra overhang of the status badge past the avatar circle's bottom-right
/// edge, as a signed fraction of [`AVATAR_WITH_STATUS_TOTAL_SIZE`] added to
/// `icon_with_status`'s default overhang.
///
/// It is not a free parameter: `0.05` is exactly what cancels that helper's
/// built-in `0.19` default, putting the badge's bottom-right corner on the
/// lockup box's own bottom-right corner. Since the box is bottom-aligned in
/// the slot, that is what makes the ring flush with the pill's bottom edge.
const PILL_BADGE_OVERHANG_RATIO: f32 = 0.05;

/// Top inset of the avatar disc inside the [`AVATAR_WITH_STATUS_TOTAL_SIZE`]
/// box. The box is bottom-aligned in the [`PILL_HEIGHT`]-tall slot so the
/// badge anchored to its bottom-right corner reaches the pill's bottom edge,
/// so this inset plus that bottom-alignment offset has to add up to
/// [`PILL_AVATAR_VERTICAL_PADDING`]: 2 + 1.5 = 3.5.
const PILL_AVATAR_LOCKUP_TOP_INSET: f32 =
    PILL_AVATAR_VERTICAL_PADDING - (PILL_HEIGHT - AVATAR_WITH_STATUS_TOTAL_SIZE);

/// Places the avatar disc inside the square box that the status badge is
/// anchored against, applying the designed padding. Shared by the
/// plain and status-badged paths so a pill's avatar lands in exactly the same
/// spot whether or not it currently has a status.
///
/// The disc hugs the box's leading edge and sits
/// [`PILL_AVATAR_LOCKUP_TOP_INSET`] down from its top edge. Only the vertical
/// placement is ours to choose; horizontally the disc has to stay flush left,
/// because the badge is anchored to the box's bottom-right corner and every
/// pixel the disc moves right is a pixel more of it the badge's cutout ring
/// eats — enough to swallow the agent's initial.
fn render_avatar_lockup_box(disc: Box<dyn Element>) -> Box<dyn Element> {
    ConstrainedBox::new(
        Container::new(Align::new(disc).top_left().finish())
            .with_padding_top(PILL_AVATAR_LOCKUP_TOP_INSET)
            .finish(),
    )
    .with_width(AVATAR_WITH_STATUS_TOTAL_SIZE)
    .with_height(AVATAR_WITH_STATUS_TOTAL_SIZE)
    .finish()
}

/// Renders the leading avatar for a pill with no status badge.
fn render_pill_avatar(
    avatar_color: ColorU,
    glyph: AvatarGlyph,
    theme: &WarpTheme,
    appearance: &Appearance,
) -> Box<dyn Element> {
    render_avatar_slot(render_avatar_lockup_box(render_avatar_disc(
        avatar_color,
        glyph,
        PILL_AVATAR_DISC_SIZE,
        theme,
        appearance,
    )))
}

/// Renders the leading avatar for a pill that has a status: the avatar disc
/// plus its status badge, in the same slot [`render_pill_avatar`] uses.
///
/// Geometry, in pill-content coordinates (the pill is [`PILL_HEIGHT`] = 22
/// tall with a [`PILL_RADIUS`] = 11 stadium cap, and the leading slot spans
/// x = 4..24 after [`PILL_HORIZONTAL_PADDING_LEFT`]):
/// * Lockup box: [`AVATAR_WITH_STATUS_TOTAL_SIZE`] = 20 square, bottom-aligned
///   in the 22-tall slot, so it spans y = 2..22.
/// * Avatar disc: [`PILL_AVATAR_DISC_SIZE`] = 15, inset
///   [`PILL_AVATAR_LOCKUP_TOP_INSET`] = 1.5 from the box's top and flush with
///   its left edge, so it spans y = 3.5..18.5 and x = 4..19 — dead-centre in
///   the pill, [`PILL_AVATAR_VERTICAL_PADDING`] = 3.5 above and below.
/// * Status badge: [`PILL_BADGE_RING_SIZE`] = 11 cutout ring, anchored BR-to-BR
///   with `corner_overlay_offset(20, 0.05)` = 0, so its BR lands on the lockup
///   box's BR at (24, 22) and the ring spans y = 11..22, x = 13..24. That is
///   9 horizontally and 7.5 vertically in from the disc's top-left, per design,
///   and its bottom is flush with the pill's — an emergent property of the
///   box being bottom-aligned, not a hardcoded 22. The ring starts at
///   x = 13 > `PILL_RADIUS`, so it sits in the pill's flat-bottom region and
///   is tangent to that edge rather than clipped by the rounded cap (only
///   x < 11 is governed by the cap's arc).
/// * Status icon: [`PILL_BADGE_ICON_SIZE`] = 9 bounding box centred in the
///   ring, leaving the 1px cutout.
fn render_avatar_with_status_overlay(
    avatar_color: ColorU,
    glyph: AvatarGlyph,
    status: ConversationStatus,
    is_remote_child: bool,
    pill_background: ColorU,
    theme: &WarpTheme,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let avatar = render_avatar_lockup_box(render_avatar_disc(
        avatar_color,
        glyph,
        PILL_AVATAR_DISC_SIZE,
        theme,
        appearance,
    ));
    let lockup = render_icon_with_status_with_badge_style(
        IconWithStatusVariant::CustomAvatar {
            avatar,
            status: Some(status),
            is_ambient: is_remote_child,
        },
        AVATAR_WITH_STATUS_TOTAL_SIZE,
        PILL_BADGE_OVERHANG_RATIO,
        PILL_BADGE_STYLE,
        theme,
        // Cutout ring color for the local badge; ignored by the cloud path.
        pill_background.into(),
    );
    // Same slot helper as the no-status path, so both share one placement
    // rule and the leading slot keeps identical width across the swap.
    render_avatar_slot(lockup)
}

/// Renders the avatar circle as a colored disc with a centered glyph (letter
/// or icon) on top. Uses `Stack` so the disc is a clean rounded square that
/// composites cleanly over the pill's own background without visual seams.
fn render_avatar_disc(
    avatar_color: ColorU,
    glyph: AvatarGlyph,
    size: f32,
    theme: &WarpTheme,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let disc = ConstrainedBox::new(
        Container::new(Empty::new().finish())
            .with_background_color(avatar_color)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(size / 2.)))
            .finish(),
    )
    .with_width(size)
    .with_height(size)
    .finish();
    let glyph_size = size * 0.625;

    let glyph_element: Box<dyn Element> = match glyph {
        AvatarGlyph::Letter(letter) => {
            Text::new(letter.to_string(), appearance.ui_font_family(), glyph_size)
                .with_color(theme.background().into_solid())
                .with_style(Properties {
                    weight: Weight::Bold,
                    ..Default::default()
                })
                // The default 1.2 ratio pads the text box with leading, so
                // centering the box leaves the letter's ink sitting high in
                // the disc. At 1.0 the box is the glyph, and centering it
                // centers what you can see.
                .with_line_height_ratio(1.)
                .finish()
        }
        AvatarGlyph::Icon(icon) => {
            ConstrainedBox::new(icon.to_warpui_icon(theme.background()).finish())
                .with_width(glyph_size)
                .with_height(glyph_size)
                .finish()
        }
    };

    let glyph_centered = ConstrainedBox::new(Align::new(glyph_element).finish())
        .with_width(size)
        .with_height(size)
        .finish();

    Stack::new()
        .with_child(disc)
        .with_child(glyph_centered)
        .finish()
}

/// Pre-computed data for a single breadcrumb in the orchestration breadcrumb row.
struct CrumbSpec {
    conversation_id: AIConversationId,
    label: String,
    avatar_color: ColorU,
    avatar_glyph: AvatarGlyph,
    /// `true` for the trailing crumb (the conversation currently being
    /// viewed). The trailing crumb is rendered with a brighter text color
    /// and is non-interactive.
    is_active: bool,
}

const CRUMB_HEIGHT: f32 = 24.;
const CRUMB_RADIUS: f32 = 4.;
const CRUMB_HORIZONTAL_PADDING: f32 = 6.;

/// Renders a `[Parent Avatar] [Parent Title] > [Child Avatar] [Child Name]`
/// breadcrumb row when the active conversation is a child agent under an
/// orchestrator that has been split off into another pane/tab. Returns
/// `None` for same-pane child views — those render the pill bar with the
/// active child highlighted instead.
///
/// We render this manually rather than going through
/// `crate::ui_components::breadcrumb::render_breadcrumbs` because we need a
/// chevron separator (per the Figma) and per-crumb avatars, neither of which
/// the shared helper supports today.
///
/// `parent_crumb_mouse_state` must be a `MouseStateHandle` owned by the caller
/// (e.g. on a TerminalView field) so hover and click events persist across
/// renders. Inline `MouseStateHandle::default()` would zero state every frame
/// and silently break clicks (per the WarpUI mouse-state guidance).
pub fn render_orchestration_breadcrumbs(
    agent_view_controller: &AgentViewController,
    parent_crumb_mouse_state: MouseStateHandle,
    horizontal_scroll_state: ClippedScrollStateHandle,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    // Mirror the gating used by `maybe_add_parent_navigation_card` in
    // `pane_impl.rs` so the breadcrumb path can't accidentally render in a
    // non-AgentView build / state.
    if !FeatureFlag::AgentView.is_enabled() {
        return None;
    }
    if !agent_view_controller.is_fullscreen() {
        return None;
    }
    // The caller (pane header) decides whether this view should render
    // breadcrumbs vs. the pill bar based on `TerminalView::is_orchestration_split_off`,
    // which is set explicitly by the "Open in new pane" / "Open in new tab"
    // flows. We deliberately don't try to derive split-off-ness from the
    // history model here because that heuristic would also match the
    // swap-target child pane (which should continue rendering the pill bar).
    let active_id = agent_view_controller
        .agent_view_state()
        .active_conversation_id()?;
    let history = BlocklistAIHistoryModel::as_ref(app);
    let active = history.conversation(&active_id)?;
    let parent_id = parent_conversation_id(active, app)?;
    // The parent's `AIConversation` may not yet be loaded into
    // `conversations_by_id` (e.g. a child agent restored on startup whose
    // parent is only known via the `children_by_parent` index — see
    // `pane_group/mod.rs`'s `TODO(QUALITY-378)`). In that case we still want
    // to render a clickable parent crumb so the user can navigate back to
    // the orchestrator: `SwitchAgentViewToConversation` will load the parent
    // through the normal `enter_agent_view_for_conversation` path. Bailing
    // out here would otherwise leave the user with no "back to parent"
    // affordance, since the new flag also suppresses the legacy parent card.
    let parent = history.conversation(&parent_id);

    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    // Prefer the parent's user-visible title; fall back to its agent name,
    // and finally to a generic "Orchestrator" label so the breadcrumb is
    // always meaningful even before titles have been generated (or before
    // the parent conversation itself has been loaded).
    let parent_label = parent
        .and_then(|p| {
            p.title()
                .filter(|t| !t.is_empty())
                .or_else(|| p.agent_name().map(str::to_string))
        })
        .unwrap_or_else(|| "Orchestrator".to_string());

    // Treat empty `agent_name` as missing so the label, avatar color, and
    // initial all consistently fall back to "Agent". Without the
    // `.filter(|n| !n.is_empty())` on `child_name`, an unnamed agent would
    // show "Agent" as the label but be hashed/initialed against the empty
    // string, producing a different color/letter from a real "Agent".
    let child_name = active
        .agent_name()
        .filter(|n| !n.is_empty())
        .unwrap_or("Agent");
    let child_label = child_name.to_string();

    // Parent crumb uses the Warp logo on a neutral disc to match the
    // orchestrator pill in the pill bar.
    let parent_spec = CrumbSpec {
        conversation_id: parent_id,
        label: parent_label,
        avatar_color: theme.ansi_fg_cyan(),
        avatar_glyph: AvatarGlyph::Icon(Icon::Agent),
        is_active: false,
    };

    // Child crumb uses the same deterministic colored disc + initial letter
    // we render in the pill bar.
    let child_spec = CrumbSpec {
        conversation_id: active_id,
        label: child_label,
        avatar_color: pill_avatar_color(child_name, theme),
        avatar_glyph: AvatarGlyph::Letter(pill_initial(child_name)),
        is_active: true,
    };

    let chevron_color = internal_colors::text_sub(theme, theme.background());
    let chevron = ConstrainedBox::new(
        Icon::ChevronRight
            .to_warpui_icon(chevron_color.into())
            .finish(),
    )
    .with_width(16.)
    .with_height(16.)
    .finish();

    // The row uses `MainAxisSize::Min` so its intrinsic width is the sum
    // of the crumbs (avatar + label per crumb plus spacing). Wrapping that
    // row in a horizontal `NewScrollable` lets the user pan through the
    // breadcrumbs whenever the title slot is too narrow to fit them —
    // common when the orchestrator was opened in a split-off pane that's
    // been resized down. With `MainAxisSize::Max` the row would always
    // try to fill the title slot which makes the inner content unscrollable.
    let mut row = Flex::row()
        .with_main_axis_alignment(MainAxisAlignment::Start)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(4.);
    let self_terminal_view_id = agent_view_controller.terminal_view_id();
    row.add_child(render_crumb(
        parent_spec,
        Some(parent_crumb_mouse_state),
        self_terminal_view_id,
        theme,
        appearance,
    ));
    row.add_child(chevron);
    row.add_child(render_crumb(
        child_spec,
        None,
        self_terminal_view_id,
        theme,
        appearance,
    ));

    let scrollable = NewScrollable::horizontal(
        SingleAxisConfig::Clipped {
            handle: horizontal_scroll_state,
            child: row.finish(),
        },
        theme.nonactive_ui_detail().into(),
        theme.active_ui_detail().into(),
        ElementFill::None,
    )
    // Pass `true` for `overlaid_scrollbar` so the horizontal scrollbar
    // paints on top of the row instead of stealing vertical space below
    // it. Reserving space pushes the breadcrumbs upward (off-center)
    // whenever the row overflows; overlaying keeps the row vertically
    // centered in the title slot at the cost of the scrollbar briefly
    // crossing through the bottom edge of the labels — which the user
    // explicitly accepted as a fine trade-off. 4px matches the pill bar
    // for a consistent hairline treatment across orchestration surfaces.
    .with_horizontal_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Custom(4.), true))
    .with_propagate_mousewheel_if_not_handled(true)
    .finish();

    // Center breadcrumbs while they fit; when they overflow, use the full
    // width so horizontal scrolling still works. Wrap in a `Container`
    // with a touch of left padding so the leading parent crumb doesn't
    // sit flush against the pane edge.
    Some(
        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(scrollable)
                .finish(),
        )
        .with_padding_left(4.)
        .finish(),
    )
}

fn render_crumb(
    spec: CrumbSpec,
    mouse_state: Option<MouseStateHandle>,
    self_terminal_view_id: EntityId,
    theme: &WarpTheme,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let conversation_id = spec.conversation_id;
    let is_active = spec.is_active;
    let label = spec.label;
    let avatar_color = spec.avatar_color;
    let avatar_glyph = spec.avatar_glyph;

    // Active (trailing) crumb: bright text, no hover/click. Use the same
    // height + padding as the interactive crumb so the row is uniform.
    if is_active {
        let inner = build_crumb_inner(
            label,
            avatar_color,
            avatar_glyph,
            true,  /* is_active */
            false, /* is_hovered */
            theme,
            appearance,
        );
        return ConstrainedBox::new(inner)
            .with_height(CRUMB_HEIGHT)
            .finish();
    }

    // Interactive (parent) crumb: hover highlight + click handler. The
    // `Hoverable::new` build closure is `FnOnce`, so `label` can move into
    // the closure by value instead of cloning on every build.
    let mouse_state = mouse_state.unwrap_or_default();
    Hoverable::new(mouse_state, move |hover_state| {
        let inner = build_crumb_inner(
            label,
            avatar_color,
            avatar_glyph,
            false, /* is_active */
            hover_state.is_hovered() || hover_state.is_clicked(),
            theme,
            appearance,
        );
        ConstrainedBox::new(inner)
            .with_height(CRUMB_HEIGHT)
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, app, _| {
        // Focus the pane that already hosts the parent conversation
        // rather than switching this (split-off child) pane to it.
        //
        // Pick the focus path based on where the parent's canonical
        // conversation pane lives, mirroring the orchestration pill bar's
        // "Focus pane" handler:
        //   * Same pane group as us (sibling pane in this tab) —
        //     dispatch `TerminalAction::RevealChildAgent`, which the
        //     pane group handles by walking visible terminal panes and
        //     focusing the one whose active conversation matches.
        //     Going through the workspace's `focus_pane` from a
        //     different `ViewContext` doesn't reliably move focus when
        //     the destination is in the same pane group.
        //   * Different pane group (other tab / window) — dispatch
        //     `WorkspaceAction::FocusTerminalViewInWorkspace`, which
        //     walks all tabs/windows and activates the containing tab
        //     as needed.
        //   * No canonical terminal surface anywhere — fall back to
        //     `SwitchAgentViewToConversation` so the breadcrumb stays
        //     useful even after the orchestrator pane has been closed
        //     and the parent conversation only persists in history.
        if let Some(conversation_view_id) = BlocklistAIHistoryModel::as_ref(app)
            .terminal_surface_id_for_conversation(&conversation_id)
        {
            let self_pane_group_id =
                pane_group_id_containing_terminal_view(self_terminal_view_id, app);
            let conversation_pane_group_id =
                pane_group_id_containing_terminal_view(conversation_view_id, app);
            if conversation_pane_group_id.is_some()
                && conversation_pane_group_id == self_pane_group_id
            {
                ctx.dispatch_typed_action(
                    PaneHeaderAction::<TerminalAction, TerminalAction>::CustomAction(
                        TerminalAction::RevealChildAgent { conversation_id },
                    ),
                );
                return;
            }
            ctx.dispatch_typed_action(WorkspaceAction::FocusTerminalViewInWorkspace {
                terminal_view_id: conversation_view_id,
            });
            return;
        }
        ctx.dispatch_typed_action(
            PaneHeaderAction::<TerminalAction, TerminalAction>::CustomAction(
                TerminalAction::SwitchAgentViewToConversation { conversation_id },
            ),
        );
    })
    .finish()
}

/// Builds the inner content (background + padding + avatar + label row) for a
/// single crumb. Shared between active (non-interactive) and interactive paths
/// so both render at the same height with consistent padding.
fn build_crumb_inner(
    label: String,
    avatar_color: ColorU,
    avatar_glyph: AvatarGlyph,
    is_active: bool,
    is_hovered: bool,
    theme: &WarpTheme,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let text_color = if is_active || is_hovered {
        internal_colors::text_main(theme, theme.background())
    } else {
        internal_colors::text_sub(theme, theme.background())
    };

    let label_text = Text::new(
        label,
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(text_color)
    .soft_wrap(false)
    .with_clip(ClipConfig::ellipsis())
    .finish();

    let avatar = render_avatar_disc(avatar_color, avatar_glyph, AVATAR_SIZE, theme, appearance);

    let row = Flex::row()
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(6.)
        .with_child(avatar)
        .with_child(
            ConstrainedBox::new(label_text)
                .with_max_width(220.)
                .finish(),
        )
        .finish();

    let mut container = Container::new(row)
        .with_padding_left(CRUMB_HORIZONTAL_PADDING)
        .with_padding_right(CRUMB_HORIZONTAL_PADDING)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CRUMB_RADIUS)));
    if is_hovered && !is_active {
        container = container.with_background_color(internal_colors::neutral_2(theme));
    }
    container.finish()
}

#[cfg(test)]
#[path = "orchestration_pill_bar_tests.rs"]
mod tests;
