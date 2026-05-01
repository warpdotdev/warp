//! Renders a horizontal pill bar in the agent view header containing the
//! orchestrator agent and any child agents spawned by it. Clicking a pill
//! switches the active pane to that agent's conversation.
//!
//! V1 scope: avatars (deterministic color + initial), pill labels, click to
//! switch. Hover popover, pin / unpin, and the 3-dot menu (Open in new pane /
//! Open in new tab / Stop agent / Kill agent) are tracked as follow-ups.

use std::cell::RefCell;
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::hash::{Hash, Hasher};

use pathfinder_color::ColorU;
use warp_core::ui::{appearance::Appearance, theme::WarpTheme};
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Empty, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::{AppContext, Entity, ModelHandle, SingletonEntity, View, ViewContext};

use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::blocklist::agent_view::orchestration_conversation_links::parent_conversation_id;
use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewControllerEvent};
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::features::FeatureFlag;
use crate::pane_group::pane::view::PaneHeaderAction;
use crate::terminal::view::TerminalAction;
use crate::ui_components::icons::Icon;
use warp_core::ui::theme::color::internal_colors;

const PILL_HEIGHT: f32 = 22.;
const PILL_RADIUS: f32 = PILL_HEIGHT / 2.;
const AVATAR_SIZE: f32 = 16.;
const PILL_LABEL_MAX_WIDTH: f32 = 110.;
const PILL_GAP: f32 = 6.;
const PILL_HORIZONTAL_PADDING_LEFT: f32 = 4.;
const PILL_HORIZONTAL_PADDING_RIGHT: f32 = 10.;

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

fn pill_avatar_color(name: &str, theme: &WarpTheme) -> ColorU {
    let palette = pill_palette(theme);
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let idx = (hasher.finish() as usize) % palette.len();
    palette[idx]
}

fn pill_initial(name: &str) -> char {
    name.trim()
        .chars()
        .next()
        .map(|c| c.to_ascii_uppercase())
        .unwrap_or('A')
}

/// What kind of pill we are rendering, which determines click behavior.
#[derive(Clone, Copy)]
enum PillKind {
    Orchestrator,
    Child,
}

/// Pre-computed data for one pill in the bar.
struct PillSpec {
    conversation_id: AIConversationId,
    label: String,
    avatar_color: ColorU,
    avatar_glyph: AvatarGlyph,
    is_selected: bool,
    kind: PillKind,
}

#[derive(Clone, Copy)]
enum AvatarGlyph {
    Letter(char),
    Icon(Icon),
}

/// View that renders the orchestration pill bar above the agent view content.
///
/// Shows one pill for the orchestrator (parent of the active conversation, or
/// the active conversation itself if it has no parent) and one pill per child
/// agent spawned by that orchestrator. Clicking a non-active pill switches
/// to that agent's pane.
pub struct OrchestrationPillBar {
    agent_view_controller: ModelHandle<AgentViewController>,
    /// Hover state per conversation id, persisted across renders so hover
    /// effects work correctly. Wrapped in a `RefCell` so `render` (which only
    /// has `&self`) can lazily insert handles for ids that aren't yet
    /// populated by `ensure_mouse_states`. Synthesizing a fresh
    /// `MouseStateHandle::default()` in the render hot path is *not*
    /// equivalent: the next render after a mouse-down would produce yet
    /// another fresh handle, losing the down-state before mouse-up arrives
    /// and silently swallowing the click (per the WarpUI mouse-state rule).
    mouse_states: RefCell<HashMap<AIConversationId, MouseStateHandle>>,
}

impl Entity for OrchestrationPillBar {
    type Event = ();
}

impl OrchestrationPillBar {
    pub fn new(
        agent_view_controller: ModelHandle<AgentViewController>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, |this, _, event, ctx| match event {
            BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
            | BlocklistAIHistoryEvent::AppendedExchange { .. }
            | BlocklistAIHistoryEvent::SetActiveConversation { .. }
            | BlocklistAIHistoryEvent::StartedNewConversation { .. } => {
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
            }
            this.ensure_mouse_states(ctx);
            ctx.notify();
        });

        Self {
            agent_view_controller,
            mouse_states: RefCell::new(HashMap::new()),
        }
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
        let orchestrator_id = parent_conversation_id(active_conversation, ctx).unwrap_or(active_id);
        // Build the set of conversation ids that should still have hover state
        // tracked, then insert any missing entries and drop the rest. Without
        // the retain step, switching between orchestrators within the same
        // view would leak `MouseStateHandle`s for old orchestrators / their
        // children indefinitely.
        let mut alive: HashSet<AIConversationId> = HashSet::new();
        alive.insert(orchestrator_id);
        // Include the parent id when the active conversation is a child, so
        // the breadcrumb's parent crumb has a stable handle even before the
        // parent `AIConversation` is loaded into history.
        if let Some(parent_id) = parent_conversation_id(active_conversation, ctx) {
            alive.insert(parent_id);
        }
        for child in history.child_conversations_of(orchestrator_id) {
            alive.insert(child.id());
        }
        let mut mouse_states = self.mouse_states.borrow_mut();
        for id in &alive {
            mouse_states.entry(*id).or_default();
        }
        mouse_states.retain(|id, _| alive.contains(id));
    }

    /// Builds the ordered list of pills to render. Returns `None` when the
    /// pill bar should not be shown at all.
    fn pill_specs(&self, app: &AppContext) -> Option<Vec<PillSpec>> {
        let active_id = self
            .agent_view_controller
            .as_ref(app)
            .agent_view_state()
            .active_conversation_id()?;
        let history = BlocklistAIHistoryModel::as_ref(app);
        let active_conversation = history.conversation(&active_id)?;

        // The pill bar only shows on the orchestrator view. When a child
        // agent is active, breadcrumbs in the pane header title take over.
        // Bail out before any further work (sorting, theme lookup, etc.).
        if parent_conversation_id(active_conversation, app).is_some() {
            return None;
        }

        let orchestrator_id = parent_conversation_id(active_conversation, app).unwrap_or(active_id);
        let orchestrator = history.conversation(&orchestrator_id)?;

        // `child_conversations_of` returns children in registration order
        // (i.e. the order they were spawned by the orchestrator), which is
        // the stable ordering we want for the pills. Don't re-sort by
        // `first_exchange().start_time`: children whose first exchange
        // hasn't started yet would otherwise sort to the front (because
        // `Option::None < Option::Some`) and pop into their time-based
        // position once they begin streaming, reshuffling the bar.
        let children = history.child_conversations_of(orchestrator_id);

        // Nothing to show if the orchestrator has no children yet.
        if children.is_empty() {
            return None;
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut specs = Vec::with_capacity(1 + children.len());

        // Orchestrator pill first.
        specs.push(PillSpec {
            conversation_id: orchestrator_id,
            label: orchestrator_label(orchestrator),
            avatar_color: theme.ansi_fg_cyan(),
            avatar_glyph: AvatarGlyph::Icon(Icon::Oz),
            is_selected: orchestrator_id == active_id,
            kind: PillKind::Orchestrator,
        });

        // Then a pill per child agent.
        for child in children {
            let name = child
                .agent_name()
                .filter(|n| !n.is_empty())
                .unwrap_or("Agent");
            specs.push(PillSpec {
                conversation_id: child.id(),
                label: name.to_string(),
                avatar_color: pill_avatar_color(name, theme),
                avatar_glyph: AvatarGlyph::Letter(pill_initial(name)),
                is_selected: child.id() == active_id,
                kind: PillKind::Child,
            });
        }

        Some(specs)
    }
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

impl View for OrchestrationPillBar {
    fn ui_name() -> &'static str {
        "OrchestrationPillBar"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let Some(specs) = self.pill_specs(app) else {
            return Empty::new().finish();
        };

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(PILL_GAP);

        // Resolve a persistent `MouseStateHandle` for each pill. If `ensure_mouse_states`
        // has not yet seen this id (e.g. mid-event-propagation race), insert a
        // freshly defaulted handle into our `mouse_states` map and reuse it on
        // subsequent renders. Falling back to a transient
        // `MouseStateHandle::default()` here would silently break clicks: the
        // mouse-down notify would re-enter `render` with yet another fresh
        // handle, and mouse-up would land on a different handle than
        // mouse-down.
        let mut mouse_states = self.mouse_states.borrow_mut();
        for spec in specs {
            let mouse_state = mouse_states
                .entry(spec.conversation_id)
                .or_default()
                .clone();
            row.add_child(render_pill(spec, mouse_state, app));
        }
        drop(mouse_states);

        // Wrap in a container with a touch of horizontal padding so the bar
        // doesn't sit flush against the pane edges, and with the same overlay
        // background as the rest of the agent view header so it merges visually.
        Container::new(row.finish())
            .with_padding_left(12.)
            .with_padding_right(12.)
            .with_padding_top(4.)
            .with_padding_bottom(4.)
            .with_background(theme.surface_overlay_1())
            .finish()
    }
}

fn render_pill(
    spec: PillSpec,
    mouse_state: MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let conversation_id = spec.conversation_id;
    let kind = spec.kind;
    let is_selected = spec.is_selected;
    // `spec` is owned by value, so we can move `label` directly into the
    // build closure below without cloning.
    let label = spec.label;
    let avatar_color = spec.avatar_color;
    let avatar_glyph = spec.avatar_glyph;

    // `Hoverable::new`'s build closure is `FnOnce` (see
    // `crates/warpui_core/src/elements/hoverable.rs`). We can therefore move
    // `label` into the closure by value rather than cloning it on every
    // build.
    Hoverable::new(mouse_state, move |hover_state| {
        let (background, text_color) = if is_selected {
            (
                theme.foreground().into_solid(),
                theme.background().into_solid(),
            )
        } else if hover_state.is_hovered() || hover_state.is_clicked() {
            (
                warp_core::ui::theme::color::internal_colors::neutral_3(theme),
                warp_core::ui::theme::color::internal_colors::text_main(theme, theme.background()),
            )
        } else {
            (
                warp_core::ui::theme::color::internal_colors::neutral_2(theme),
                warp_core::ui::theme::color::internal_colors::text_main(theme, theme.background()),
            )
        };

        let label_text = Text::new(
            label,
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 1.,
        )
        .with_color(text_color)
        .soft_wrap(false)
        .with_clip(ClipConfig::ellipsis())
        .with_style(Properties {
            weight: if is_selected {
                Weight::Semibold
            } else {
                Weight::Normal
            },
            ..Default::default()
        })
        .finish();

        // Avatar circle: rendered as a Stack-layered colored disc with the
        // glyph centered on top. This avoids visually competing with the pill
        // background (which sits behind it) and keeps the corners clean.
        let avatar = render_avatar_disc(avatar_color, avatar_glyph, theme, appearance);

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(6.)
            .with_child(avatar)
            .with_child(
                ConstrainedBox::new(label_text)
                    .with_max_width(PILL_LABEL_MAX_WIDTH)
                    .finish(),
            )
            .finish();

        // Constrain pill to a fixed height so the half-stadium corner radius
        // renders as a clean continuous shape rather than awkwardly clamping.
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
    .with_cursor(if is_selected {
        Cursor::Arrow
    } else {
        Cursor::PointingHand
    })
    .on_click(move |ctx, _app, _| {
        if is_selected {
            return;
        }
        // Both child and orchestrator pills use SwitchAgentViewToConversation
        // so the active pane navigates in place — no new splits.
        // Wrapped in PaneHeaderAction::CustomAction since the pill bar lives
        // inside the pane header chrome (mirrors `agent_view_back_button`).
        let _ = kind;
        ctx.dispatch_typed_action(
            PaneHeaderAction::<TerminalAction, TerminalAction>::CustomAction(
                TerminalAction::SwitchAgentViewToConversation { conversation_id },
            ),
        );
    })
    .finish()
}

/// Renders the avatar circle as a colored disc with a centered glyph (letter
/// or icon) on top. Uses `Stack` so the disc is a clean rounded square that
/// composites cleanly over the pill's own background without visual seams.
fn render_avatar_disc(
    avatar_color: ColorU,
    glyph: AvatarGlyph,
    theme: &WarpTheme,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let disc = ConstrainedBox::new(
        Container::new(Empty::new().finish())
            .with_background_color(avatar_color)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(AVATAR_SIZE / 2.)))
            .finish(),
    )
    .with_width(AVATAR_SIZE)
    .with_height(AVATAR_SIZE)
    .finish();

    let glyph_element: Box<dyn Element> = match glyph {
        AvatarGlyph::Letter(letter) => Text::new(
            letter.to_string(),
            appearance.ui_font_family(),
            (appearance.monospace_font_size() - 2.).max(9.),
        )
        .with_color(theme.background().into_solid())
        .with_style(Properties {
            weight: Weight::Bold,
            ..Default::default()
        })
        .finish(),
        AvatarGlyph::Icon(icon) => {
            ConstrainedBox::new(icon.to_warpui_icon(theme.background()).finish())
                .with_width(10.)
                .with_height(10.)
                .finish()
        }
    };

    // Center the glyph on top of the disc both horizontally and vertically by
    // using `MainAxisAlignment::Center` (along axis) and
    // `CrossAxisAlignment::Center` (perpendicular) on both Flex containers.
    let glyph_centered = ConstrainedBox::new(
        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(glyph_element)
                    .finish(),
            )
            .finish(),
    )
    .with_width(AVATAR_SIZE)
    .with_height(AVATAR_SIZE)
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
/// orchestrator. Returns `None` when breadcrumbs should not be shown
/// (orchestrator view, feature flag off, or no parent conversation available).
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
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    // Mirror the gating used by `maybe_add_parent_navigation_card` in
    // `pane_impl.rs` so the breadcrumb path can't accidentally render in a
    // non-AgentView build / state.
    if !FeatureFlag::AgentView.is_enabled() {
        return None;
    }
    if !FeatureFlag::OrchestrationPillBar.is_enabled() {
        return None;
    }
    if !agent_view_controller.is_fullscreen() {
        return None;
    }
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

    // Parent crumb uses the Oz glyph on a neutral disc to match the
    // orchestrator pill in the pill bar.
    let parent_spec = CrumbSpec {
        conversation_id: parent_id,
        label: parent_label,
        avatar_color: theme.ansi_fg_cyan(),
        avatar_glyph: AvatarGlyph::Icon(Icon::Oz),
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

    let mut row = Flex::row()
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Max)
        .with_spacing(4.);
    row.add_child(render_crumb(
        parent_spec,
        Some(parent_crumb_mouse_state),
        theme,
        appearance,
    ));
    row.add_child(chevron);
    row.add_child(render_crumb(child_spec, None, theme, appearance));
    Some(row.finish())
}

fn render_crumb(
    spec: CrumbSpec,
    mouse_state: Option<MouseStateHandle>,
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
    .on_click(move |ctx, _, _| {
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

    let avatar = render_avatar_disc(avatar_color, avatar_glyph, theme, appearance);

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
