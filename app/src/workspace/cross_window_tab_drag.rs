/// Singleton model that owns all cross-window tab drag state.
///
/// # Overview
///
/// When a user drags a tab out of a window (or drags a single-tab window), this
/// model tracks the drag lifecycle through three phases (see [`DragPhase`]):
/// `Floating` (preview follows the cursor), `InsertedInTarget` (tab has been
/// handed off into another window's tab bar), and `Transitioning` (a view-tree
/// transfer is in progress). The `Transitioning` phase blocks `on_drag` from
/// re-entering the drag handler while views are being moved between windows,
/// which the WarpUI framework does not support within a single event cycle.
///
/// # Relationship with Workspace views
///
/// This model is a singleton: it is not owned by any particular `Workspace`, and
/// all cross-window coordination flows through it. Workspaces call `on_drag` /
/// `on_drop` and inspect the returned [`DragResult`] / [`DropResult`] enums to
/// decide what follow-up action to take (insert a tab, close a window, focus
/// the target). This indirection avoids direct cross-workspace mutation.
///
/// Two drag sources are supported (see [`DragSource`]):
///   - **SingleTabWindow**: the source window itself acts as the floating preview.
///   - **MultiTabWindow**: a dedicated preview window is created for the tab.
///
/// # State machine – single-tab window drag
///
/// ```text
/// [begin_single_tab_drag]
///       │
///       ▼
///   Floating ◄──────────────────┐
///       │                       │
///       │ cursor enters a       │ cursor leaves target tab bar
///       │ target tab bar        │ (reverse_handoff moves tab back
///       │                       │  to the preview window)
///       ▼                       │
///   Transitioning ──► InsertedInTarget
///       │                       │
///       │                       │ on_drop while inserted
///       │                       ▼
///       │                  FinalizeHandoff
///       │
///       │ on_drop while floating
///       └──────────────────► FinalizeFloatingWindow  (no target; keep window at drop position)
/// ```
///
/// Because the source window IS the preview in this case, no extra preview window is
/// created.
///
/// # State machine – multi-tab window drag
///
/// ```text
/// [begin_multi_tab_drag]  (creates a dedicated preview window)
///       │
///       ▼
///   Floating ◄──────────────────┐
///       │                       │
///       │ cursor enters a       │ cursor leaves target tab bar
///       │ target tab bar        │ (reverse_handoff transfers tab
///       │                       │  back into the preview window)
///       ▼                       │
///   Transitioning ──► InsertedInTarget
///       │                       │
///       │                       │ on_drop while inserted
///       │                       ▼
///       │                  FinalizeHandoff
///       │                  (closes preview, removes source tab)
///       │
///       │ on_drop while floating
///       └──────────────────► FinalizePreviewAsNewWindow  (no target; promote preview to permanent
///                                                        window, remove source tab)
/// ```
///
/// View transfers between windows are handled by `transfer_view_tree_to_window`.
use crate::tab::tab_position_id;
use crate::workspace::view::{tab_bar_rects_for_window, TransferredTab, TAB_BAR_POSITION_ID};
use crate::workspace::WorkspaceRegistry;
use pathfinder_geometry::rect::RectF;
use std::collections::HashSet;
use warpui::elements::DraggableState;
use warpui::geometry::vector::{vec2f, Vector2F};
use warpui::platform::TerminationMode;
use warpui::windowing::WindowManager;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity, WindowId};

/// Identifies a window and tab-bar index where a dragged tab can be attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AttachTarget {
    pub window_id: WindowId,
    pub insertion_index: usize,
}

/// Pixel margin added around a candidate window's tab-bar rect when hit-testing
/// for cross-window handoff, so cursor overshoot still registers as a hit.
const TAB_BAR_HIT_MARGIN: f32 = 12.0;

/// Singleton model that manages all cross-window tab drag state.
///
/// At most one cross-window drag is active at a time. The `active_drag` field is `Some`
/// for the duration of the drag and is cleared on drop.
pub struct CrossWindowTabDrag {
    active_drag: Option<ActiveDrag>,
    /// Window ids of source / preview workspaces whose close was requested
    /// as part of a tab-drag handoff but whose `Workspace::on_window_closed`
    /// has not yet run. `is_active()` returns `true` while this set is
    /// non-empty so that `save_app` (in `workspace/global_actions.rs`)
    /// skips persistence during the gap between `ctx.windows().close_window(...)`
    /// (async) and the actual window close — which is when both the source
    /// workspace and the target / promoted-preview workspace still hold
    /// `TabData`s whose `pane_group` is the same `ViewHandle<PaneGroup>`,
    /// and saving the app state would insert the same `terminal_panes.uuid`
    /// twice.
    ///
    /// Entries are added via [`register_pending_source_close`] from
    /// `finalize` and removed via [`finish_pending_source_close`] from
    /// `Workspace::on_window_closed`.
    pending_source_window_closes: HashSet<WindowId>,
}

/// Describes how the drag was initiated, which determines how the preview window is
/// managed.
enum DragSource {
    /// The source window had only one tab, so the window itself acts as the floating
    /// preview. No extra window is created and no view transfers are needed at drag start.
    SingleTabWindow,
    /// The source window had multiple tabs. A dedicated preview window was created to
    /// hold the detached tab, and the pane group + auxiliary views were transferred into it.
    MultiTabWindow {
        source_tab_index: usize,
        preview_window_id: WindowId,
    },
}

/// Mutable state for an in-progress cross-window tab drag.
struct ActiveDrag {
    source_window_id: WindowId,
    source: DragSource,
    /// Size of the preview window (used to reposition it as the cursor moves).
    window_size: Vector2F,
    /// Offset from the drag origin (top-left of the draggable rect) to its center.
    /// Used to convert between origin-based and center-based coordinate systems.
    initial_drag_center_offset: Vector2F,
    /// The last known position of tab index 0 inside the preview window, in window-local
    /// coordinates. Cached so we can still position the window correctly on frames where
    /// the element layout hasn't been computed yet.
    last_known_target_tab_origin_in_window: Vector2F,
    /// Screen-space position of the drag center at the most recent `on_drag`
    /// event. Used by `on_drop` to re-resolve the attach target one more time
    /// so that releasing the mouse directly over a target tab bar (including
    /// the source's own) still commits a handoff instead of promoting the
    /// preview into a new window.
    last_drag_center_on_screen: Option<Vector2F>,
    /// Caller window id from the most recent `on_drag` event. Paired with
    /// `last_drag_center_on_screen` for the drop-time re-resolution above.
    last_caller_window_id: Option<WindowId>,
    /// Whether `on_drop` has already attempted a drop-time re-resolution of
    /// the attach target. Prevents an infinite loop if the post-handoff phase
    /// somehow reverts to `Floating` before `finalize` runs.
    drop_resolution_attempted: bool,
    /// Set once a put-back handoff (target == caller) removes the detached
    /// placeholder from the source, making `source_tab_index` stale. If the
    /// drag later ends in `Floating`, `finalize_preview_as_new_window` must
    /// return `NoOp` instead of asking the caller to remove that index
    /// (which would panic `debug_assert` or remove the wrong tab).
    source_placeholder_consumed: bool,
    /// Source layout (vertical tabs panel vs horizontal tab bar) at drag
    /// start. Frozen for the duration of the drag so the floating ghost
    /// chip rendered in the target keeps using the source layout even if
    /// the user toggles their layout mid-drag.
    was_vertical_layout: bool,
    /// Rendered size of the dragged tab in the source layout, captured at
    /// drag-start from the source window's last-frame tab position. Used
    /// to constrain the ghost chip so it has the same dimensions as the
    /// source tab.
    source_element_size: Vector2F,
    phase: DragPhase,
}

impl ActiveDrag {
    fn source_tab_index(&self) -> usize {
        match &self.source {
            DragSource::SingleTabWindow => 0,
            DragSource::MultiTabWindow {
                source_tab_index, ..
            } => *source_tab_index,
        }
    }

    fn source_was_single_tab(&self) -> bool {
        matches!(self.source, DragSource::SingleTabWindow)
    }

    fn has_dedicated_preview_window(&self) -> bool {
        matches!(self.source, DragSource::MultiTabWindow { .. })
    }

    fn preview_window_id(&self) -> WindowId {
        match &self.source {
            DragSource::SingleTabWindow => self.source_window_id,
            DragSource::MultiTabWindow {
                preview_window_id, ..
            } => *preview_window_id,
        }
    }
}

/// Tracks which phase of the drag lifecycle is currently active.
///
/// See the module-level doc for full state-transition diagrams.
enum DragPhase {
    /// The preview window is floating freely, following the cursor. When the cursor
    /// enters another window's tab bar the model transitions to `GhostInTarget`
    /// (no view-tree transfer yet). For the back-to-caller path (cursor re-enters
    /// the source window's own tab bar during a multi-tab drag) it returns
    /// `HandoffNeeded` to trigger a live transfer.
    Floating,
    /// The cursor is hovering over a target window's tab bar but **no view-tree
    /// transfer has occurred**. The target renders a lightweight visual ghost
    /// (insertion slot + floating chip) so the user can see where the tab will
    /// land. The real `transfer_view_tree_to_window` is deferred to drop time.
    ///
    /// `ghost_cursor_in_target` is the cursor position in the target window's
    /// coordinate space, updated on every drag event. The target workspace reads
    /// it via `CrossWindowTabDrag::ghost_state_for_window` to position the chip.
    GhostInTarget {
        target_window_id: WindowId,
        target_insertion_index: usize,
        ghost_cursor_in_target: Vector2F,
    },
    /// The tab has been transferred into another window's tab list and is being
    /// dragged within that window's tab bar. Used only for the back-to-caller
    /// path (multi-tab drag returning to the source window). The preview window
    /// stays alive so a reverse-handoff can move the tab back if needed.
    InsertedInTarget {
        target_window_id: WindowId,
        target_insertion_index: usize,
    },
    /// A handoff (transferring the tab into a target window) or reverse-handoff
    /// (transferring it back to the preview window) is in progress. Set immediately
    /// before views are moved between windows to prevent re-entrant drag processing.
    Transitioning,
}

/// Data read by a target window's renderer to display the ghost visual during
/// a deferred cross-window drag hover (i.e. while in `DragPhase::GhostInTarget`).
pub struct GhostState {
    /// Index in the target's tab list where the insertion slot is shown.
    pub insertion_index: usize,
    /// Cursor position in target window coordinates.
    pub cursor_in_window: Vector2F,
    /// Cursor position within the source element at drag-start time
    /// (`initial_drag_center_offset`). The chip's top-left is placed at
    /// `cursor_in_window - cursor_offset_in_element` so that the cursor sits
    /// at the same relative position inside the chip as it did in the original
    /// tab when the drag was initiated.
    pub cursor_offset_in_element: Vector2F,
    /// Rendered size of the dragged tab in the source layout. The chip is
    /// constrained to this size so it looks identical to the source tab.
    pub source_element_size: Vector2F,
    /// Window id of the preview workspace whose first tab is the dragged
    /// tab. The target's renderer looks this workspace up and renders its
    /// `tabs[0]` using the same code path the source layout uses, so the
    /// chip's contents match the source tab exactly.
    pub preview_window_id: WindowId,
    /// Source layout (vertical tabs panel vs horizontal tab bar) at drag
    /// start. Determines which render path the ghost should mirror.
    pub was_vertical_layout: bool,
}

/// Information returned to the calling workspace after a handoff back to the caller's
/// own window, so it can insert the tab at the correct position.
pub struct HandoffCallerInfo {
    pub transferred_tab: TransferredTab,
    pub insertion_index: usize,
}

/// Result of processing a drag event (`on_drag`). The calling workspace inspects this
/// to decide what follow-up action to take.
pub enum DragResult {
    /// The drag event was fully handled internally; no caller action needed.
    Handled,
    /// The draggable element's position needs to be adjusted by the given offset.
    /// This happens in the single-tab case where the source window is physically
    /// repositioned and the draggable coordinates must be corrected to match.
    AdjustDraggable { adjustment: Vector2F },
    /// The cursor is over a target tab bar and a handoff should be initiated.
    /// The caller must call the appropriate `execute_handoff_*` method.
    HandoffNeeded { target: AttachTarget },
}

/// Result of processing a drop event (`on_drop`).
///
/// Cross-workspace mutations (updating the preview/target workspace, focusing
/// windows) are performed inside `on_drop` itself.  The returned variant only
/// tells the **calling** workspace what to do with its own state.
pub enum DropResult {
    /// Nothing to do (drag was already finalized or was mid-transition).
    NoOp,
    /// The calling workspace should focus its own active tab (single-tab
    /// floating drop — the source window IS the preview).
    FocusSelf,
    /// The source window's only tab was transferred elsewhere.  The calling
    /// workspace should unsubscribe the pane group and close itself.
    CloseSourceWindow { transferred_tab_index: usize },
    /// One tab was transferred out of a multi-tab source.  The calling
    /// workspace should unsubscribe and remove the tab.
    RemoveSourceTab { transferred_tab_index: usize },
    /// One tab was transferred out of a multi-tab source via a handoff to
    /// a different window.  The calling workspace should unsubscribe, remove
    /// the tab, and close the now-unused preview window.
    RemoveSourceTabAndClosePreview {
        transferred_tab_index: usize,
        preview_window_id: WindowId,
    },
    /// A `Floating` drop landed on empty space but a prior put-back had
    /// already committed the tab back into the source. The preview still
    /// carries a `TabData` pointing at the same pane group, so the caller
    /// must close it (not promote it to a new permanent window) and the
    /// source side has nothing else to do. See
    /// `ActiveDrag::source_placeholder_consumed` and
    /// `finalize_preview_as_new_window` for the overlap this guards.
    ClosePreviewOnly { preview_window_id: WindowId },
    /// The drop landed directly over a candidate tab bar (typically the
    /// source window, but potentially any other window). The caller should
    /// run `Workspace::perform_handoff(target, ctx)` to move the tab into
    /// `target` and then re-invoke `CrossWindowTabDrag::finalize(ctx)` to
    /// close the preview and clean up source state.
    DropInto { target: AttachTarget },
}

impl Entity for CrossWindowTabDrag {
    type Event = ();
}

impl SingletonEntity for CrossWindowTabDrag {}

impl CrossWindowTabDrag {
    pub fn new() -> Self {
        Self {
            active_drag: None,
            pending_source_window_closes: HashSet::new(),
        }
    }

    pub fn is_active(&self) -> bool {
        self.active_drag.is_some() || !self.pending_source_window_closes.is_empty()
    }

    /// Records that `window_id` is a source / preview workspace whose close
    /// has been requested as part of a tab-drag handoff but whose
    /// `on_window_closed` has not yet run. Keeps `is_active()` true until
    /// the matching [`finish_pending_source_close`] call, so `save_app`
    /// doesn't try to persist both sides of the still-duplicated
    /// `terminal_panes.uuid` in the meantime. See the field doc on
    /// `CrossWindowTabDrag::pending_source_window_closes`.
    pub fn register_pending_source_close(&mut self, window_id: WindowId) {
        self.pending_source_window_closes.insert(window_id);
    }

    /// Clears a pending source-window close registered via
    /// [`register_pending_source_close`]. Called from
    /// `Workspace::on_window_closed` once the source / preview workspace has
    /// actually been unregistered, so `is_active()` returns false on the
    /// first `save_app` that follows the close. Safe to call for a window
    /// id that wasn't registered — it is a no-op in that case.
    pub fn finish_pending_source_close(&mut self, window_id: WindowId) {
        self.pending_source_window_closes.remove(&window_id);
    }

    pub fn source_window_id(&self) -> Option<WindowId> {
        self.active_drag.as_ref().map(|d| d.source_window_id)
    }

    /// Returns the tab index of the dragged tab within the source window, if a drag is active.
    pub fn transferred_tab_index(&self) -> Option<usize> {
        self.active_drag.as_ref().map(|d| d.source_tab_index())
    }

    /// Returns the tab index in the source window of the detached placeholder
    /// that should be hidden (rendered with 0 width / skipped in snapshots)
    /// while a cross-window drag is in progress.
    ///
    /// Differs from [`Self::transferred_tab_index`] after a put-back handoff
    /// (`target == caller`): once the placeholder has been removed and the
    /// real tab re-inserted into the source, `source_tab_index` no longer
    /// points at a placeholder and must not drive rendering — otherwise an
    /// unrelated tab at that stale index would be collapsed to 0 width.
    pub fn source_placeholder_tab_index(&self) -> Option<usize> {
        self.active_drag.as_ref().and_then(|d| {
            if d.source_placeholder_consumed {
                None
            } else {
                Some(d.source_tab_index())
            }
        })
    }

    /// Returns `true` if a put-back handoff has already removed the detached
    /// placeholder from the source, so a subsequent handoff attempt would
    /// operate on a stale `source_tab_index` and corrupt an unrelated tab.
    /// See `ActiveDrag::source_placeholder_consumed`.
    pub fn source_placeholder_consumed(&self) -> bool {
        self.active_drag
            .as_ref()
            .is_some_and(|d| d.source_placeholder_consumed)
    }

    pub fn has_dedicated_preview_window(&self) -> bool {
        self.active_drag
            .as_ref()
            .is_some_and(|d| d.has_dedicated_preview_window())
    }

    pub fn handed_off_target(&self) -> Option<(WindowId, usize)> {
        self.active_drag.as_ref().and_then(|d| match &d.phase {
            DragPhase::InsertedInTarget {
                target_window_id,
                target_insertion_index,
            } => Some((*target_window_id, *target_insertion_index)),
            DragPhase::GhostInTarget { .. } | DragPhase::Floating | DragPhase::Transitioning => {
                None
            }
        })
    }

    /// Returns rendering data for the ghost visual in `window_id`'s tab bar,
    /// or `None` if no ghost is active for that window.
    ///
    /// Called during rendering by the target workspace to position the
    /// floating chip overlay and insertion slot.
    pub fn ghost_state_for_window(&self, window_id: WindowId) -> Option<GhostState> {
        self.active_drag.as_ref().and_then(|d| match &d.phase {
            DragPhase::GhostInTarget {
                target_window_id,
                target_insertion_index,
                ghost_cursor_in_target,
            } if *target_window_id == window_id => Some(GhostState {
                insertion_index: *target_insertion_index,
                cursor_in_window: *ghost_cursor_in_target,
                cursor_offset_in_element: d.initial_drag_center_offset,
                source_element_size: d.source_element_size,
                preview_window_id: d.preview_window_id(),
                was_vertical_layout: d.was_vertical_layout,
            }),
            _ => None,
        })
    }

    pub fn source_was_single_tab(&self) -> bool {
        self.active_drag
            .as_ref()
            .is_some_and(|d| d.source_was_single_tab())
    }

    pub(crate) fn reset_to_floating(&mut self) {
        if let Some(drag) = self.active_drag.as_mut() {
            drag.phase = DragPhase::Floating;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn begin_single_tab_drag(
        &mut self,
        source_window_id: WindowId,
        initial_drag_center_offset: Vector2F,
        window_size: Vector2F,
        last_known_target_tab_origin_in_window: Vector2F,
        was_vertical_layout: bool,
        source_element_size: Vector2F,
    ) {
        log::info!(
            "tab_drag: begin_single_tab_drag source_wid={source_window_id} (source window IS preview)"
        );
        self.active_drag = Some(ActiveDrag {
            source_window_id,
            source: DragSource::SingleTabWindow,
            window_size,
            initial_drag_center_offset,
            last_known_target_tab_origin_in_window,
            last_drag_center_on_screen: None,
            last_caller_window_id: None,
            drop_resolution_attempted: false,
            source_placeholder_consumed: false,
            was_vertical_layout,
            source_element_size,
            phase: DragPhase::Floating,
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn begin_multi_tab_drag(
        &mut self,
        source_window_id: WindowId,
        source_tab_index: usize,
        initial_drag_center_offset: Vector2F,
        window_size: Vector2F,
        last_known_target_tab_origin_in_window: Vector2F,
        preview_window_id: WindowId,
        was_vertical_layout: bool,
        source_element_size: Vector2F,
    ) {
        log::info!(
            "tab_drag: begin_multi_tab_drag source_wid={source_window_id} preview_wid={preview_window_id} source_tab_index={source_tab_index}"
        );
        self.active_drag = Some(ActiveDrag {
            source_window_id,
            source: DragSource::MultiTabWindow {
                source_tab_index,
                preview_window_id,
            },
            window_size,
            initial_drag_center_offset,
            last_known_target_tab_origin_in_window,
            last_drag_center_on_screen: None,
            last_caller_window_id: None,
            drop_resolution_attempted: false,
            source_placeholder_consumed: false,
            was_vertical_layout,
            source_element_size,
            phase: DragPhase::Floating,
        });
    }

    /// Called by `Workspace::perform_handoff`
    /// (`target.window_id == caller_window_id`) has removed the detached
    /// placeholder tab from the source workspace. See the field doc on
    /// `ActiveDrag::source_placeholder_consumed` for why this matters.
    pub fn mark_source_placeholder_consumed(&mut self) {
        if let Some(drag) = self.active_drag.as_mut() {
            drag.source_placeholder_consumed = true;
        }
    }

    pub fn on_drag(
        &mut self,
        caller_window_id: WindowId,
        drag_position: RectF,
        ctx: &mut ModelContext<Self>,
    ) -> DragResult {
        let Some(drag) = self.active_drag.as_mut() else {
            return DragResult::Handled;
        };

        if matches!(drag.phase, DragPhase::Transitioning) {
            return DragResult::Handled;
        }

        let source_window_origin = match ctx.window_bounds(&caller_window_id) {
            Some(bounds) => bounds.origin(),
            None => return DragResult::Handled,
        };

        let drag_origin_in_window = vec2f(drag_position.min_x(), drag_position.min_y());
        let drag_center_in_window = drag_origin_in_window + drag.initial_drag_center_offset;
        let drag_origin_on_screen = source_window_origin + drag_origin_in_window;
        let drag_center_on_screen = source_window_origin + drag_center_in_window;

        // Cache the screen-space cursor so `on_drop` can re-run attach-target
        // resolution with the same coordinates the user last saw.
        drag.last_drag_center_on_screen = Some(drag_center_on_screen);
        drag.last_caller_window_id = Some(caller_window_id);

        match &drag.phase {
            DragPhase::GhostInTarget {
                target_window_id,
                target_insertion_index,
                ghost_cursor_in_target: _,
            } => {
                let target_wid = *target_window_id;
                let target_idx = *target_insertion_index;
                self.on_drag_while_ghost(
                    caller_window_id,
                    drag_origin_on_screen,
                    drag_center_on_screen,
                    target_wid,
                    target_idx,
                    ctx,
                )
            }
            DragPhase::InsertedInTarget {
                target_window_id,
                target_insertion_index,
            } => {
                let target_wid = *target_window_id;
                let target_idx = *target_insertion_index;
                self.on_drag_while_inserted(
                    caller_window_id,
                    drag_origin_on_screen,
                    drag_center_on_screen,
                    target_wid,
                    target_idx,
                    ctx,
                )
            }
            DragPhase::Floating => self.on_drag_while_floating(
                caller_window_id,
                drag_origin_on_screen,
                drag_center_on_screen,
                ctx,
            ),
            DragPhase::Transitioning => DragResult::Handled,
        }
    }

    /// Handles a drag event while the cursor is hovering over a target window's
    /// tab bar in ghost mode — no view-tree transfer has occurred yet.
    ///
    /// On every event this method:
    /// - Repositions the preview window to follow the cursor (so it is in the
    ///   right place if the cursor moves off the target).
    /// - Checks if the cursor is still over the target tab bar. If not, clears
    ///   the ghost and transitions back to `Floating`.
    /// - Recomputes the insertion index and cursor position in target coords
    ///   and, if either changed, updates the phase and notifies the target to
    ///   re-render the ghost visuals.
    fn on_drag_while_ghost(
        &mut self,
        caller_window_id: WindowId,
        drag_origin_on_screen: Vector2F,
        drag_center_on_screen: Vector2F,
        target_window_id: WindowId,
        target_insertion_index: usize,
        ctx: &mut ModelContext<Self>,
    ) -> DragResult {
        let Some(drag) = self.active_drag.as_mut() else {
            return DragResult::Handled;
        };
        let preview_window_id = drag.preview_window_id();

        // Keep the preview window repositioned to follow the cursor so that it
        // is already in position if the cursor later moves off the target.
        let target_tab_origin_in_window = ctx
            .element_position_by_id_at_last_frame(preview_window_id, tab_position_id(0))
            .or_else(|| {
                ctx.element_position_by_id_at_last_frame(preview_window_id, TAB_BAR_POSITION_ID)
            })
            .map(|rect| vec2f(rect.min_x(), rect.min_y()))
            .unwrap_or(drag.last_known_target_tab_origin_in_window);
        drag.last_known_target_tab_origin_in_window = target_tab_origin_in_window;
        let new_window_origin = drag_origin_on_screen - target_tab_origin_in_window;
        let new_bounds = RectF::new(new_window_origin, drag.window_size);
        ctx.set_and_cache_window_bounds(preview_window_id, new_bounds);

        // Compute the DragResult to return to the caller (for single-tab
        // draggable coordinate adjustment).
        let drag_result = if drag.has_dedicated_preview_window() {
            DragResult::Handled
        } else {
            ctx.windows().cancel_synthetic_drag(preview_window_id);
            let source_window_origin = ctx
                .window_bounds(&caller_window_id)
                .map(|b| b.origin())
                .unwrap_or_default();
            DragResult::AdjustDraggable {
                adjustment: source_window_origin - new_window_origin,
            }
        };

        // Hit-test the target tab bar — same expanded margin as the entry check
        // so the cursor must leave the same area to clear the ghost.
        let still_over_target = ctx
            .window_bounds(&target_window_id)
            .map(|wb| {
                tab_bar_rects_for_window(target_window_id, ctx)
                    .into_iter()
                    .any(|tb| {
                        let on_screen = RectF::new(
                            vec2f(wb.min_x() + tb.min_x(), wb.min_y() + tb.min_y()),
                            tb.size(),
                        );
                        expanded_rect(on_screen, TAB_BAR_HIT_MARGIN)
                            .contains_point(drag_center_on_screen)
                    })
            })
            .unwrap_or(false);

        if !still_over_target {
            // Cursor left the target — clear the ghost and go back to Floating.
            log::info!(
                "tab_drag: on_drag_while_ghost: cursor left target_wid={target_window_id} (GhostInTarget->Floating)"
            );
            if let Some(drag) = self.active_drag.as_mut() {
                drag.phase = DragPhase::Floating;
            }
            if let Some(ws) = WorkspaceRegistry::as_ref(ctx).get(target_window_id, ctx) {
                ws.update(ctx, |_, ctx| ctx.notify());
            }
            // Restore the preview's opacity (it was set to 0.0 on entry into
            // the target in `on_drag_while_floating`) and re-focus it.
            ctx.windows().set_window_alpha(preview_window_id, 1.0);
            ctx.windows().show_window_and_focus_app(preview_window_id);
            return drag_result;
        }

        // Still over the target — update insertion index and cursor position.
        let new_index = WorkspaceRegistry::as_ref(ctx)
            .get(target_window_id, ctx)
            .map(|ws| {
                ws.read(ctx, |workspace, ctx| {
                    workspace.tab_insertion_index_for_cursor(
                        target_window_id,
                        drag_center_on_screen,
                        ctx,
                    )
                })
            })
            .unwrap_or(target_insertion_index);

        let new_cursor_in_target = ctx
            .window_bounds(&target_window_id)
            .map(|wb| drag_center_on_screen - wb.origin())
            .unwrap_or_default();

        if new_index != target_insertion_index
            || new_cursor_in_target
                != (match self.active_drag.as_ref().map(|d| &d.phase) {
                    Some(DragPhase::GhostInTarget {
                        ghost_cursor_in_target,
                        ..
                    }) => *ghost_cursor_in_target,
                    _ => Vector2F::zero(),
                })
        {
            if let Some(drag) = self.active_drag.as_mut() {
                drag.phase = DragPhase::GhostInTarget {
                    target_window_id,
                    target_insertion_index: new_index,
                    ghost_cursor_in_target: new_cursor_in_target,
                };
            }
            if let Some(ws) = WorkspaceRegistry::as_ref(ctx).get(target_window_id, ctx) {
                ws.update(ctx, |_, ctx| ctx.notify());
            }
        }

        drag_result
    }

    /// Handles a drag event while the tab is inserted in a target window's tab
    /// bar. Reorders the tab within the target's tab list, or initiates a
    /// reverse-handoff if the cursor has left the tab bar. Kept separate from
    /// the local adjacent-swap path in `Workspace::calculate_updated_tab_index`
    /// because this path operates in screen coordinates and uses
    /// `tab_insertion_index_for_cursor` for arbitrary-position insertion.
    fn on_drag_while_inserted(
        &mut self,
        caller_window_id: WindowId,
        drag_origin_on_screen: Vector2F,
        drag_center_on_screen: Vector2F,
        target_window_id: WindowId,
        target_insertion_index: usize,
        ctx: &mut ModelContext<Self>,
    ) -> DragResult {
        let Some(drag) = self.active_drag.as_mut() else {
            return DragResult::Handled;
        };

        // Hit-test both the horizontal tab bar and vertical tabs panel, since
        // both can be rendered simultaneously. Expand each rect by
        // `TAB_BAR_HIT_MARGIN` to match `cross_window_attach_target`: without
        // this, a cursor in the 12 px entry margin would satisfy the handoff
        // check but fail this stay-check on the next frame, bouncing back
        // via `reverse_handoff` (the put-back → reverse → empty-window bug).
        let still_over_target_tab_bar = ctx
            .window_bounds(&target_window_id)
            .map(|wb| {
                tab_bar_rects_for_window(target_window_id, ctx)
                    .into_iter()
                    .any(|tb| {
                        let tab_bar_on_screen = RectF::new(
                            vec2f(wb.min_x() + tb.min_x(), wb.min_y() + tb.min_y()),
                            tb.size(),
                        );
                        expanded_rect(tab_bar_on_screen, TAB_BAR_HIT_MARGIN)
                            .contains_point(drag_center_on_screen)
                    })
            })
            .unwrap_or(false);

        if still_over_target_tab_bar {
            let new_insertion_index = WorkspaceRegistry::as_ref(ctx)
                .get(target_window_id, ctx)
                .map(|ws| {
                    ws.read(ctx, |workspace, ctx| {
                        workspace.tab_insertion_index_for_cursor(
                            target_window_id,
                            drag_center_on_screen,
                            ctx,
                        )
                    })
                });

            if let Some(new_insertion_index) = new_insertion_index {
                let target_index = if new_insertion_index > target_insertion_index {
                    new_insertion_index - 1
                } else {
                    new_insertion_index
                };

                if target_index != target_insertion_index {
                    if let Some(target_ws) =
                        WorkspaceRegistry::as_ref(ctx).get(target_window_id, ctx)
                    {
                        target_ws.update(ctx, |workspace, ctx| {
                            let tab = workspace.tabs.remove(target_insertion_index);
                            workspace.tabs.insert(target_index, tab);
                            workspace.set_active_tab_index(target_index, ctx);
                            ctx.notify();
                        });
                    }
                    drag.phase = DragPhase::InsertedInTarget {
                        target_window_id,
                        target_insertion_index: target_index,
                    };
                }
            }

            let current_target_index = match &drag.phase {
                DragPhase::InsertedInTarget {
                    target_insertion_index,
                    ..
                } => *target_insertion_index,
                _ => target_insertion_index,
            };

            if let Some(target_window_bounds) = ctx.window_bounds(&target_window_id) {
                let mouse_pos_in_target = drag_center_on_screen - target_window_bounds.origin();
                let mouse_offset = -drag.initial_drag_center_offset;
                if let Some(target_ws) = WorkspaceRegistry::as_ref(ctx).get(target_window_id, ctx) {
                    target_ws.update(ctx, |workspace, _ctx| {
                        if let Some(tab) = workspace.tabs.get(current_target_index) {
                            tab.draggable_state
                                .set_dragging(mouse_pos_in_target, mouse_offset);
                        }
                    });
                }
            }

            if drag.has_dedicated_preview_window() {
                let preview_wid = drag.preview_window_id();
                let target_tab_origin_in_window = ctx
                    .element_position_by_id_at_last_frame(preview_wid, tab_position_id(0))
                    .or_else(|| {
                        ctx.element_position_by_id_at_last_frame(preview_wid, TAB_BAR_POSITION_ID)
                    })
                    .map(|rect| vec2f(rect.min_x(), rect.min_y()))
                    .unwrap_or_else(|| vec2f(0.0, 0.0));
                let new_window_origin = drag_origin_on_screen - target_tab_origin_in_window;
                let new_bounds = RectF::new(new_window_origin, drag.window_size);
                ctx.set_and_cache_window_bounds(preview_wid, new_bounds);
            }

            return DragResult::Handled;
        }

        // Cursor left the target tab bar: enter Transitioning to block re-entrant drag
        // processing, then reverse the handoff to move the tab back to the preview window.
        drag.phase = DragPhase::Transitioning;
        self.reverse_handoff(
            caller_window_id,
            target_window_id,
            target_insertion_index,
            ctx,
        );
        DragResult::Handled
    }

    /// Handles a drag event while the tab is floating freely.
    ///
    /// Repositions the preview window to follow the cursor, checks whether the cursor
    /// is now over another window's tab bar, and returns `HandoffNeeded` if the tab
    /// should be transferred into the target.
    fn on_drag_while_floating(
        &mut self,
        caller_window_id: WindowId,
        drag_origin_on_screen: Vector2F,
        drag_center_on_screen: Vector2F,
        ctx: &mut ModelContext<Self>,
    ) -> DragResult {
        let Some(drag) = self.active_drag.as_mut() else {
            return DragResult::Handled;
        };
        let preview_window_id = drag.preview_window_id();

        let target_tab_origin_in_window = ctx
            .element_position_by_id_at_last_frame(preview_window_id, tab_position_id(0))
            .or_else(|| {
                ctx.element_position_by_id_at_last_frame(preview_window_id, TAB_BAR_POSITION_ID)
            })
            .map(|rect| vec2f(rect.min_x(), rect.min_y()))
            .unwrap_or(drag.last_known_target_tab_origin_in_window);

        drag.last_known_target_tab_origin_in_window = target_tab_origin_in_window;

        let handoff_target = cross_window_attach_target(
            caller_window_id,
            drag.source_window_id,
            drag_center_on_screen,
            preview_window_id,
            ctx,
        );

        let new_window_origin = drag_origin_on_screen - target_tab_origin_in_window;
        let new_bounds = RectF::new(new_window_origin, drag.window_size);
        ctx.set_and_cache_window_bounds(preview_window_id, new_bounds);

        let drag_result = if drag.has_dedicated_preview_window() {
            DragResult::Handled
        } else {
            ctx.windows().cancel_synthetic_drag(preview_window_id);
            let source_window_origin = ctx
                .window_bounds(&caller_window_id)
                .map(|b| b.origin())
                .unwrap_or_default();
            DragResult::AdjustDraggable {
                adjustment: source_window_origin - new_window_origin,
            }
        };

        if let Some(target) = handoff_target {
            let Some(drag) = self.active_drag.as_mut() else {
                return drag_result;
            };

            // Back-to-caller path: cursor returned to the source window's own
            // tab bar during a multi-tab drag. This requires a live handoff
            // since the tab needs to be physically re-inserted into the source.
            if target.window_id == caller_window_id {
                log::info!(
                    "tab_drag: on_drag_while_floating -> HandoffNeeded (back-to-caller) target_wid={} insertion_index={} caller_wid={caller_window_id} (phase Floating->Transitioning)",
                    target.window_id,
                    target.insertion_index
                );
                drag.phase = DragPhase::Transitioning;
                return DragResult::HandoffNeeded { target };
            }

            // Cross-window target: enter GhostInTarget — show a cheap visual
            // in the target without any view-tree transfer. The real
            // `transfer_view_tree_to_window` is deferred until drop.
            log::info!(
                "tab_drag: on_drag_while_floating -> GhostInTarget target_wid={} insertion_index={} caller_wid={caller_window_id} (Floating->GhostInTarget)",
                target.window_id,
                target.insertion_index
            );

            let ghost_cursor_in_target = ctx
                .window_bounds(&target.window_id)
                .map(|wb| drag_center_on_screen - wb.origin())
                .unwrap_or_default();

            drag.phase = DragPhase::GhostInTarget {
                target_window_id: target.window_id,
                target_insertion_index: target.insertion_index,
                ghost_cursor_in_target,
            };

            // Bring the target to the front and visually hide the preview/source
            // window so it isn't visible during the hover. The real view-tree
            // transfer is still deferred to drop time — hiding is independent
            // of that.
            //
            // Use `set_window_alpha(0.0)` instead of `hide_window`: the latter
            // calls `[NSWindow orderOut:]` and runs the `PreviousStateHelper`
            // app-activation dance, which is heavy enough to noticeably stall
            // the drag on entry into a target window. `setAlphaValue:` leaves
            // the window in the window list, key/focus state, and z-order
            // unchanged, so this is essentially free.
            //
            // The preview's alpha is restored in `on_drag_while_ghost` (cursor
            // leaves) and in the `GhostInTarget` failsafe branch of `finalize`.
            ctx.windows().show_window_and_focus_app(target.window_id);
            ctx.windows().set_window_alpha(preview_window_id, 0.0);

            // Notify the target workspace to render the ghost visuals.
            if let Some(ws) = WorkspaceRegistry::as_ref(ctx).get(target.window_id, ctx) {
                ws.update(ctx, |_, ctx| ctx.notify());
            }

            return drag_result;
        }

        drag_result
    }

    /// Entry point for a mouse-up / drop event.
    ///
    /// If the drag is `Floating` with a dedicated preview, one last
    /// attach-target resolution is attempted against the most recent cursor
    /// position; if a target is found, returns `DropResult::DropInto` and
    /// leaves `active_drag` in place so the caller can run `perform_handoff`
    /// and then `finalize`. This covers the "mouse released directly over a
    /// tab bar" case (including the source's own bar) where no handoff had
    /// triggered in-flight. Otherwise, delegates to `finalize`.
    pub fn on_drop(&mut self, ctx: &mut ModelContext<Self>) -> DropResult {
        // Ghost phase: drop at the ghost's insertion position without any
        // prior view-tree transfer. Return DropInto so the workspace calls
        // perform_handoff (real transfer) and then finalize.
        if let Some(drag) = self.active_drag.as_ref() {
            if let DragPhase::GhostInTarget {
                target_window_id,
                target_insertion_index,
                ..
            } = drag.phase
            {
                let target = AttachTarget {
                    window_id: target_window_id,
                    insertion_index: target_insertion_index,
                };
                log::info!(
                    "tab_drag: on_drop GhostInTarget -> DropResult::DropInto target_wid={} insertion_index={}",
                    target.window_id,
                    target.insertion_index
                );
                return DropResult::DropInto { target };
            }
        }

        let (phase_name, has_dedicated_preview, drop_resolution_attempted_before) =
            match &self.active_drag {
                Some(d) => (
                    match &d.phase {
                        DragPhase::Floating => "Floating",
                        DragPhase::GhostInTarget { .. } => "GhostInTarget",
                        DragPhase::InsertedInTarget { .. } => "InsertedInTarget",
                        DragPhase::Transitioning => "Transitioning",
                    },
                    d.has_dedicated_preview_window(),
                    d.drop_resolution_attempted,
                ),
                None => ("<no active drag>", false, false),
            };
        log::info!(
            "tab_drag: on_drop ENTER phase={phase_name} has_dedicated_preview={has_dedicated_preview} drop_resolution_attempted_before={drop_resolution_attempted_before}"
        );
        if let Some(drag) = self.active_drag.as_mut() {
            if matches!(drag.phase, DragPhase::Floating)
                && drag.has_dedicated_preview_window()
                && !drag.drop_resolution_attempted
                // Skip the drop-time re-resolve once a prior put-back has
                // already committed the tab back into the source. At that
                // point `source_tab_index` is stale, so re-running
                // `perform_handoff` via `DropInto` would operate on a
                // bystander tab and duplicate the pane group across two
                // windows. See the `ClosePreviewOnly` branch in
                // `finalize_preview_as_new_window`.
                && !drag.source_placeholder_consumed
            {
                drag.drop_resolution_attempted = true;
                let last_cursor = drag.last_drag_center_on_screen;
                let last_caller = drag.last_caller_window_id;
                let source_window_id = drag.source_window_id;
                let preview_window_id = drag.preview_window_id();
                if let (Some(cursor), Some(caller)) = (last_cursor, last_caller) {
                    let resolved = cross_window_attach_target(
                        caller,
                        source_window_id,
                        cursor,
                        preview_window_id,
                        ctx,
                    );
                    if let Some(target) = resolved {
                        log::info!(
                            "tab_drag: on_drop -> DropResult::DropInto target_wid={} insertion_index={}",
                            target.window_id,
                            target.insertion_index
                        );
                        return DropResult::DropInto { target };
                    }
                } else {
                    log::warn!(
                        "tab_drag: on_drop drop-time re-resolve skipped (missing last_cursor or last_caller)"
                    );
                }
            } else if matches!(drag.phase, DragPhase::Floating)
                && drag.has_dedicated_preview_window()
                && drag.source_placeholder_consumed
            {
                log::info!(
                    "tab_drag: on_drop skipping drop-time re-resolve (source_placeholder_consumed)"
                );
            }
        }

        self.finalize(ctx)
    }

    /// Finalizes the drag. Consumes `active_drag`, clears the focus-suppression
    /// flag, performs cross-workspace updates (preview/target workspace
    /// mutations, window focus), and returns a `DropResult` that tells the
    /// **calling** workspace what source-side cleanup is needed.
    ///
    /// Callers that received `DropResult::DropInto` from `on_drop` must invoke
    /// this after running `perform_handoff`.
    pub fn finalize(&mut self, ctx: &mut ModelContext<Self>) -> DropResult {
        let Some(drag) = self.active_drag.take() else {
            log::info!("tab_drag: finalize called with no active drag -> NoOp");
            return DropResult::NoOp;
        };

        ctx.set_suppress_focus_for_window(None);

        let result = match &drag.phase {
            DragPhase::Floating => {
                if drag.has_dedicated_preview_window() {
                    log::info!(
                        "tab_drag: finalize branch=Floating+dedicated_preview -> finalize_preview_as_new_window (CREATES NEW WINDOW) source_wid={} preview_wid={}",
                        drag.source_window_id,
                        drag.preview_window_id()
                    );
                    self.finalize_preview_as_new_window(&drag, ctx)
                } else {
                    log::info!(
                        "tab_drag: finalize branch=Floating+single_tab -> FocusSelf source_wid={}",
                        drag.source_window_id
                    );
                    ctx.windows()
                        .show_window_and_focus_app(drag.preview_window_id());
                    DropResult::FocusSelf
                }
            }
            // Failsafe: finalize was called while still in GhostInTarget
            // (unusual — e.g. window closed mid-drag). Erase the ghost from
            // the target and treat as a floating drop.
            DragPhase::GhostInTarget {
                target_window_id, ..
            } => {
                log::warn!(
                    "tab_drag: finalize branch=GhostInTarget (direct finalize without drop) target_wid={target_window_id} source_wid={}",
                    drag.source_window_id
                );
                if let Some(ws) = WorkspaceRegistry::as_ref(ctx).get(*target_window_id, ctx) {
                    ws.update(ctx, |_, ctx| ctx.notify());
                }
                // Restore the preview's opacity — it was set to 0.0 on entry
                // into the target in `on_drag_while_floating` and the normal
                // restore path in `on_drag_while_ghost` was bypassed.
                ctx.windows()
                    .set_window_alpha(drag.preview_window_id(), 1.0);
                if drag.has_dedicated_preview_window() {
                    self.finalize_preview_as_new_window(&drag, ctx)
                } else {
                    ctx.windows()
                        .show_window_and_focus_app(drag.preview_window_id());
                    DropResult::FocusSelf
                }
            }
            DragPhase::InsertedInTarget {
                target_window_id,
                target_insertion_index,
            } => {
                log::info!(
                    "tab_drag: finalize branch=InsertedInTarget target_wid={target_window_id} insertion_index={target_insertion_index} source_wid={}",
                    drag.source_window_id
                );
                self.finalize_handoff(&drag, *target_window_id, *target_insertion_index, ctx)
            }
            DragPhase::Transitioning => {
                log::warn!(
                    "tab_drag: finalize branch=Transitioning (drop landed during handoff) -> NoOp source_wid={}",
                    drag.source_window_id
                );
                DropResult::NoOp
            }
        };

        // Register any source / preview window whose close was requested as
        // part of this drop so `is_active()` stays true until its
        // `on_window_closed` runs; see `pending_source_window_closes` field
        // doc for the `terminal_panes.uuid` race this guards.
        match &result {
            DropResult::CloseSourceWindow { .. } => {
                log::info!(
                    "tab_drag: register_pending_source_close source_wid={} (CloseSourceWindow)",
                    drag.source_window_id
                );
                self.register_pending_source_close(drag.source_window_id);
            }
            DropResult::RemoveSourceTabAndClosePreview {
                preview_window_id, ..
            } => {
                log::info!(
                    "tab_drag: register_pending_source_close preview_wid={preview_window_id} (RemoveSourceTabAndClosePreview)"
                );
                self.register_pending_source_close(*preview_window_id);
            }
            DropResult::ClosePreviewOnly { preview_window_id } => {
                log::info!(
                    "tab_drag: register_pending_source_close preview_wid={preview_window_id} (ClosePreviewOnly)"
                );
                self.register_pending_source_close(*preview_window_id);
            }
            DropResult::FocusSelf
            | DropResult::RemoveSourceTab { .. }
            | DropResult::NoOp
            | DropResult::DropInto { .. } => {}
        }

        result
    }

    /// Resolves a `Floating` drop with a dedicated preview. Branches on
    /// whether a prior put-back handoff has already committed the tab back
    /// into the source (`source_placeholder_consumed`):
    ///
    /// - **Consumed**: the source already owns the tab; the preview window
    ///   still carries a stale `TabData` pointing at the same pane group
    ///   (inserted by the last `reverse_handoff`). Close the preview and
    ///   leave the source untouched. Returning `ClosePreviewOnly` keeps the
    ///   pending-close guard live so no `save_app` snapshots both windows
    ///   before the preview finishes closing — same race as
    ///   `RemoveSourceTabAndClosePreview`.
    /// - **Not consumed**: genuine new-window drop. Promote the preview to
    ///   a permanent window (clearing `is_tab_drag_preview`, re-syncing
    ///   window chrome, focusing it) and tell the caller to drop its
    ///   source-side placeholder via `CloseSourceWindow` /
    ///   `RemoveSourceTab`.
    fn finalize_preview_as_new_window(
        &self,
        drag: &ActiveDrag,
        ctx: &mut ModelContext<Self>,
    ) -> DropResult {
        let preview_window_id = drag.preview_window_id();

        if drag.source_placeholder_consumed {
            // The tab already lives in the source from an earlier put-back.
            // Do NOT promote the preview — that would leave a duplicate
            // `TabData` referencing the same pane group in both windows and
            // trip the `terminal_panes.uuid` UNIQUE constraint on the next
            // `save_app`. Close the preview instead and hand the caller a
            // `ClosePreviewOnly` so `finalize` can register the pending
            // close.
            log::info!(
                "tab_drag: finalize_preview_as_new_window -> ClosePreviewOnly (source placeholder already consumed by put-back) preview_wid={preview_window_id}"
            );
            return DropResult::ClosePreviewOnly { preview_window_id };
        }

        if let Some(ws) = WorkspaceRegistry::as_ref(ctx).get(preview_window_id, ctx) {
            ws.update(ctx, |ws, ctx| {
                ws.set_is_tab_drag_preview(false);
                // The preview's `suppress_detach_panes_on_window_close` flag
                // is latched to `true` by every forward handoff out of the
                // preview (`prepare_for_transferred_tab_attach` in
                // `execute_handoff_multi_tab_to_other`) and is *not* cleared
                // by `reverse_handoff` for the multi-tab case (only
                // `is_tab_drag_preview` is restored there). Promoting the
                // preview to a permanent window without clearing this flag
                // would leave a normal-looking window that silently skips
                // pane-detach on its next user-initiated close.
                ws.set_suppress_detach_panes_on_window_close(false);
                ws.sync_window_button_visibility(ctx);
                ws.update_titlebar_height(ctx);
                ctx.notify();
            });
        } else {
            log::warn!(
                "tab_drag: finalize_preview_as_new_window no workspace for preview_wid={preview_window_id}"
            );
        }
        ctx.windows().show_window_and_focus_app(preview_window_id);
        Self::deferred_focus(preview_window_id, ctx);

        if drag.source_was_single_tab() {
            DropResult::CloseSourceWindow {
                transferred_tab_index: drag.source_tab_index(),
            }
        } else {
            DropResult::RemoveSourceTab {
                transferred_tab_index: drag.source_tab_index(),
            }
        }
    }

    /// Cleans up the target workspace's drag state and tells the calling
    /// workspace to clean up its source tab (if source ≠ target).
    fn finalize_handoff(
        &self,
        drag: &ActiveDrag,
        target_window_id: WindowId,
        target_tab_index: usize,
        ctx: &mut ModelContext<Self>,
    ) -> DropResult {
        if let Some(ws) = WorkspaceRegistry::as_ref(ctx).get(target_window_id, ctx) {
            ws.update(ctx, |ws, _ctx| {
                ws.current_workspace_state.is_tab_being_dragged = false;
                if let Some(tab) = ws.tabs.get(target_tab_index) {
                    tab.draggable_state.set_suppress_overlay_paint(false);
                    tab.draggable_state.cancel_drag();
                }
            });
        }
        ctx.windows().show_window_and_focus_app(target_window_id);
        Self::deferred_focus(target_window_id, ctx);

        if drag.source_window_id == target_window_id {
            if drag.has_dedicated_preview_window() {
                log::info!(
                    "tab_drag: finalize_handoff source==target, closing preview_wid={}",
                    drag.preview_window_id()
                );
                ctx.windows().close_window(
                    drag.preview_window_id(),
                    TerminationMode::ContentTransferred,
                );
            }
            return DropResult::NoOp;
        }

        if drag.source_was_single_tab() {
            log::info!(
                "tab_drag: finalize_handoff -> CloseSourceWindow transferred_tab_index={}",
                drag.source_tab_index()
            );
            DropResult::CloseSourceWindow {
                transferred_tab_index: drag.source_tab_index(),
            }
        } else {
            log::info!(
                "tab_drag: finalize_handoff -> RemoveSourceTabAndClosePreview transferred_tab_index={} preview_wid={}",
                drag.source_tab_index(),
                drag.preview_window_id()
            );
            DropResult::RemoveSourceTabAndClosePreview {
                transferred_tab_index: drag.source_tab_index(),
                preview_window_id: drag.preview_window_id(),
            }
        }
    }

    /// Schedules `focus_active_tab` on the next event-loop tick because
    /// `show_window_and_focus_app` returns before the view receives its focus
    /// event. Running on the next tick ensures the terminal can accept input
    /// immediately after the drop.
    fn deferred_focus(window_id: WindowId, ctx: &mut ModelContext<Self>) {
        if let Some(ws) = WorkspaceRegistry::as_ref(ctx).get(window_id, ctx) {
            ws.update(ctx, |_ws, ctx| {
                ctx.spawn(async {}, move |view, _output, ctx| {
                    // Re-issue the OS-level focus request on the next tick:
                    // some Linux WMs silently drop the focus request when it
                    // races with pending state changes (e.g. a preview window
                    // becoming a normal window).
                    ctx.windows().show_window_and_focus_app(window_id);
                    view.focus_active_tab(ctx);
                });
            });
        }
    }

    /// Hands off a single-tab drag to another window.
    ///
    /// Transfers the pane group tree from the source (caller) window into the target,
    /// inserts the tab at the target index, hides the source window, and transitions
    /// the phase to `InsertedInTarget`.
    pub fn execute_handoff_single_tab_to_other(
        &mut self,
        target: AttachTarget,
        transferred_tab: TransferredTab,
        caller_window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) {
        log::info!(
            "tab_drag: execute_handoff_single_tab_to_other target_wid={} insertion_index={} caller_wid={caller_window_id}",
            target.window_id,
            target.insertion_index
        );
        let Some(drag) = self.active_drag.as_mut() else {
            log::warn!("tab_drag: execute_handoff_single_tab_to_other no active drag");
            return;
        };

        let Some(target_workspace) = WorkspaceRegistry::as_ref(ctx).get(target.window_id, ctx)
        else {
            log::warn!(
                "tab_drag: execute_handoff_single_tab_to_other no target workspace for target_wid={} (reset_to_floating)",
                target.window_id
            );
            self.reset_to_floating();
            return;
        };

        let pane_group_id = transferred_tab.pane_group.id();
        ctx.transfer_view_tree_to_window(pane_group_id, caller_window_id, target.window_id);

        target_workspace.update(ctx, move |workspace, ctx| {
            workspace.insert_transferred_tab_at_index(transferred_tab, target.insertion_index, ctx);
            workspace.current_workspace_state.is_tab_being_dragged = true;
        });

        ctx.windows().hide_window(caller_window_id);
        ctx.windows().show_window_and_focus_app(target.window_id);
        if let Some(workspace) = WorkspaceRegistry::as_ref(ctx).get(target.window_id, ctx) {
            workspace.update(ctx, |ws, ctx| {
                ws.focus_active_tab(ctx);
            });
        }

        drag.phase = DragPhase::InsertedInTarget {
            target_window_id: target.window_id,
            target_insertion_index: target.insertion_index,
        };
    }

    /// Hands off a multi-tab drag back to the caller's own window.
    ///
    /// Retrieves the tab from the preview window, transfers the pane group tree
    /// back to the caller window, hides the preview (kept alive for a potential
    /// `reverse_handoff` if the user drags out again), and transitions to
    /// `InsertedInTarget` on the caller. Returns the tab and insertion index so
    /// the caller can insert it into its tab list.
    pub fn execute_handoff_back_to_caller(
        &mut self,
        target: AttachTarget,
        caller_draggable_state: DraggableState,
        caller_window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<HandoffCallerInfo> {
        log::info!(
            "tab_drag: execute_handoff_back_to_caller target_wid={} insertion_index={} caller_wid={caller_window_id}",
            target.window_id,
            target.insertion_index
        );
        let drag = self.active_drag.as_mut().or_else(|| {
            log::warn!("tab_drag: execute_handoff_back_to_caller no active drag -> None");
            None
        })?;
        let preview_window_id = drag.preview_window_id();

        let preview_workspace =
            WorkspaceRegistry::as_ref(ctx).get(preview_window_id, ctx).or_else(|| {
                log::warn!(
                    "tab_drag: execute_handoff_back_to_caller no preview workspace for preview_wid={preview_window_id} -> None"
                );
                None
            })?;

        let mut transferred_tab = preview_workspace
            .read(ctx, |workspace, ctx| {
                workspace.get_tab_transfer_info_for_attach(0, ctx)
            })
            .or_else(|| {
                log::warn!(
                    "tab_drag: execute_handoff_back_to_caller preview has no tab at index 0 -> None"
                );
                None
            })?;

        transferred_tab.draggable_state = caller_draggable_state;

        preview_workspace.update(ctx, |workspace, ctx| {
            workspace.prepare_for_transferred_tab_attach(&transferred_tab.pane_group, ctx);
        });

        let pane_group_id = transferred_tab.pane_group.id();
        ctx.transfer_view_tree_to_window(pane_group_id, preview_window_id, caller_window_id);

        let insertion_index = if target.insertion_index > drag.source_tab_index() {
            target.insertion_index - 1
        } else {
            target.insertion_index
        };

        ctx.set_suppress_focus_for_window(None);
        ctx.windows().hide_window(preview_window_id);
        ctx.windows().show_window_and_focus_app(caller_window_id);

        drag.phase = DragPhase::InsertedInTarget {
            target_window_id: caller_window_id,
            target_insertion_index: insertion_index,
        };
        log::info!(
            "tab_drag: execute_handoff_back_to_caller -> InsertedInTarget target_wid={caller_window_id} insertion_index={insertion_index}"
        );

        Some(HandoffCallerInfo {
            transferred_tab,
            insertion_index,
        })
    }

    /// Hands off a multi-tab drag to a different (non-caller) window.
    ///
    /// Retrieves the tab from the preview window, transfers the pane group tree
    /// into the target, inserts the tab, hides the preview, and transitions to
    /// `InsertedInTarget`.
    pub fn execute_handoff_multi_tab_to_other(
        &mut self,
        target: AttachTarget,
        ctx: &mut ModelContext<Self>,
    ) {
        log::info!(
            "tab_drag: execute_handoff_multi_tab_to_other target_wid={} insertion_index={}",
            target.window_id,
            target.insertion_index
        );
        let Some(drag) = self.active_drag.as_mut() else {
            log::warn!("tab_drag: execute_handoff_multi_tab_to_other no active drag");
            return;
        };
        let preview_window_id = drag.preview_window_id();

        let Some(preview_workspace) = WorkspaceRegistry::as_ref(ctx).get(preview_window_id, ctx)
        else {
            log::warn!(
                "tab_drag: execute_handoff_multi_tab_to_other no preview workspace for preview_wid={preview_window_id} (reset_to_floating)"
            );
            self.reset_to_floating();
            return;
        };
        let Some(target_workspace) = WorkspaceRegistry::as_ref(ctx).get(target.window_id, ctx)
        else {
            log::warn!(
                "tab_drag: execute_handoff_multi_tab_to_other no target workspace for target_wid={} (reset_to_floating)",
                target.window_id
            );
            self.reset_to_floating();
            return;
        };
        let Some(mut transferred_tab) = preview_workspace.read(ctx, |workspace, ctx| {
            workspace.get_tab_transfer_info_for_attach(0, ctx)
        }) else {
            log::warn!(
                "tab_drag: execute_handoff_multi_tab_to_other preview has no tab at index 0 for preview_wid={preview_window_id} (reset_to_floating)"
            );
            self.reset_to_floating();
            return;
        };
        transferred_tab.draggable_state = DraggableState::default();

        preview_workspace.update(ctx, |workspace, ctx| {
            workspace.prepare_for_transferred_tab_attach(&transferred_tab.pane_group, ctx);
        });

        let pane_group_id = transferred_tab.pane_group.id();
        ctx.transfer_view_tree_to_window(pane_group_id, preview_window_id, target.window_id);

        let target_insertion_index = target.insertion_index;
        target_workspace.update(ctx, move |workspace, ctx| {
            workspace.insert_transferred_tab_at_index(transferred_tab, target_insertion_index, ctx);
            workspace.current_workspace_state.is_tab_being_dragged = true;
        });

        ctx.windows().hide_window(preview_window_id);
        ctx.windows().show_window_and_focus_app(target.window_id);
        if let Some(workspace) = WorkspaceRegistry::as_ref(ctx).get(target.window_id, ctx) {
            workspace.update(ctx, |ws, ctx| {
                ws.focus_active_tab(ctx);
            });
        }

        drag.phase = DragPhase::InsertedInTarget {
            target_window_id: target.window_id,
            target_insertion_index: target.insertion_index,
        };
        log::info!(
            "tab_drag: execute_handoff_multi_tab_to_other -> InsertedInTarget target_wid={} insertion_index={}",
            target.window_id,
            target.insertion_index
        );
    }

    /// Reverses a handoff: moves the tab back from the target window into the preview window.
    ///
    /// Called when the cursor leaves the target tab bar while in `InsertedInTarget`. Extracts
    /// the tab from the target, transfers the view tree back to the preview, removes the tab
    /// from the target, and transitions back to `Floating`.
    fn reverse_handoff(
        &mut self,
        caller_window_id: WindowId,
        target_window_id: WindowId,
        target_insertion_index: usize,
        ctx: &mut ModelContext<Self>,
    ) {
        log::info!(
            "tab_drag: reverse_handoff caller_wid={caller_window_id} target_wid={target_window_id} target_insertion_index={target_insertion_index} (phase Transitioning->Floating)"
        );
        let Some(drag) = self.active_drag.as_mut() else {
            log::warn!("tab_drag: reverse_handoff no active drag");
            return;
        };

        let preview_window_id = drag.preview_window_id();
        let preview_is_self = preview_window_id == caller_window_id;

        let Some(target_workspace) = WorkspaceRegistry::as_ref(ctx).get(target_window_id, ctx)
        else {
            drag.phase = DragPhase::Floating;
            return;
        };
        if !preview_is_self
            && WorkspaceRegistry::as_ref(ctx)
                .get(preview_window_id, ctx)
                .is_none()
        {
            drag.phase = DragPhase::Floating;
            return;
        }

        let Some(transferred_tab) = target_workspace.read(ctx, |workspace, ctx| {
            workspace.get_tab_transfer_info_for_attach(target_insertion_index, ctx)
        }) else {
            drag.phase = DragPhase::Floating;
            return;
        };

        target_workspace.update(ctx, |workspace, ctx| {
            workspace.prepare_for_transferred_tab_attach(&transferred_tab.pane_group, ctx);
        });

        transferred_tab.draggable_state.cancel_drag();

        let pane_group_id = transferred_tab.pane_group.id();
        ctx.transfer_view_tree_to_window(pane_group_id, target_window_id, preview_window_id);

        target_workspace.update(ctx, |workspace, ctx| {
            workspace.current_workspace_state.is_tab_being_dragged = false;
            workspace.remove_tab_without_undo(target_insertion_index, ctx);
        });

        if !preview_is_self {
            let is_single_tab_source = drag.source_was_single_tab();
            if let Some(preview_workspace) =
                WorkspaceRegistry::as_ref(ctx).get(preview_window_id, ctx)
            {
                preview_workspace.update(ctx, |workspace, ctx| {
                    if is_single_tab_source {
                        workspace.set_suppress_detach_panes_on_window_close(false);
                        transferred_tab
                            .draggable_state
                            .set_suppress_overlay_paint(true);
                    } else {
                        workspace.set_is_tab_drag_preview(true);
                    }
                    workspace.tabs.clear();
                    workspace.insert_transferred_tab_at_index(transferred_tab, 0, ctx);
                });
            }
        } else if drag.source_was_single_tab() {
            // For a single-tab source, `preview_window_id == source_window_id`
            // and on this code path the OS-level drag is still bound to the
            // source's draggable, so `caller_window_id == preview_window_id`
            // and the `!preview_is_self` block above is skipped. The forward
            // handoff still latched `suppress_detach_panes_on_window_close`
            // on the source via `prepare_for_transferred_tab_attach`, so we
            // must clear it explicitly here. Without this, ending the drop
            // back in the source (`Floating` → `FocusSelf`) leaves the flag
            // set and a later normal close of that window silently skips
            // pane-detach cleanup.
            if let Some(preview_workspace) =
                WorkspaceRegistry::as_ref(ctx).get(preview_window_id, ctx)
            {
                preview_workspace.update(ctx, |workspace, _ctx| {
                    workspace.set_suppress_detach_panes_on_window_close(false);
                });
            }
        }

        ctx.windows().show_window_and_focus_app(preview_window_id);

        drag.phase = DragPhase::Floating;
        log::info!("tab_drag: reverse_handoff complete, phase=Floating");
    }
}

/// Finds the best attach target for a dragged tab.
///
/// Walks the z-ordered window list behind the preview and returns the first
/// window whose tab bar (expanded by `TAB_BAR_HIT_MARGIN`) contains the
/// cursor. Iterates all z-behind windows rather than short-circuiting on the
/// topmost window that contains the cursor, since a lower window's tab bar
/// can still be exposed where the cursor sits.
///
/// When the preview isn't in the ordered list (single-tab case), falls back
/// to scanning the source window and all other workspaces, picking the tab
/// bar whose center is closest to the cursor.
fn cross_window_attach_target(
    caller_window_id: WindowId,
    source_window_id: WindowId,
    cursor_position_on_screen: Vector2F,
    preview_window_id: WindowId,
    ctx: &AppContext,
) -> Option<AttachTarget> {
    let ordered_windows = WindowManager::as_ref(ctx).ordered_window_ids();

    if let Some(preview_idx) = ordered_windows
        .iter()
        .position(|id| *id == preview_window_id)
    {
        for &window_id in &ordered_windows[preview_idx + 1..] {
            let Some(window_bounds) = ctx.window_bounds(&window_id) else {
                continue;
            };
            // Collect both the horizontal tab bar and vertical tabs panel
            // rects so hovering either registers as a hit.
            let tab_bar_positions = tab_bar_rects_for_window(window_id, ctx);
            if tab_bar_positions.is_empty() {
                // No rects laid out yet (e.g. first frame); skip so z-behind
                // windows still get a chance to match.
                continue;
            }

            let hit = tab_bar_positions.into_iter().any(|tab_bar_position| {
                let tab_bar_on_screen = RectF::new(
                    vec2f(
                        window_bounds.min_x() + tab_bar_position.min_x(),
                        window_bounds.min_y() + tab_bar_position.min_y(),
                    ),
                    tab_bar_position.size(),
                );
                expanded_rect(tab_bar_on_screen, TAB_BAR_HIT_MARGIN)
                    .contains_point(cursor_position_on_screen)
            });
            if !hit {
                continue;
            }

            let insertion_index = compute_insertion_index_for_window(
                window_id,
                caller_window_id,
                cursor_position_on_screen,
                ctx,
            );

            return Some(AttachTarget {
                window_id,
                insertion_index,
            });
        }
        return None;
    }

    let mut best_target: Option<(f32, AttachTarget)> = None;
    let mut update_best_target =
        |window_id: WindowId, insertion_index: usize, tab_bar_position_on_screen: RectF| {
            let distance_from_center =
                (cursor_position_on_screen - tab_bar_position_on_screen.center()).length();
            let target = AttachTarget {
                window_id,
                insertion_index,
            };
            match best_target {
                Some((best_distance, _)) if best_distance <= distance_from_center => {}
                _ => {
                    best_target = Some((distance_from_center, target));
                }
            }
        };

    if source_window_id != preview_window_id {
        if let Some(window_bounds) = ctx.window_bounds(&source_window_id) {
            for tab_bar_position in tab_bar_rects_for_window(source_window_id, ctx) {
                let tab_bar_position_on_screen = RectF::new(
                    vec2f(
                        window_bounds.min_x() + tab_bar_position.min_x(),
                        window_bounds.min_y() + tab_bar_position.min_y(),
                    ),
                    tab_bar_position.size(),
                );
                if !expanded_rect(tab_bar_position_on_screen, TAB_BAR_HIT_MARGIN)
                    .contains_point(cursor_position_on_screen)
                {
                    continue;
                }
                let insertion_index = compute_insertion_index_for_window(
                    source_window_id,
                    caller_window_id,
                    cursor_position_on_screen,
                    ctx,
                );
                update_best_target(
                    source_window_id,
                    insertion_index,
                    tab_bar_position_on_screen,
                );
            }
        }
    }

    for (window_id, workspace) in WorkspaceRegistry::as_ref(ctx).all_workspaces(ctx) {
        if window_id == preview_window_id || window_id == source_window_id {
            continue;
        }

        let Some(window_bounds) = ctx.window_bounds(&window_id) else {
            continue;
        };

        for tab_bar_position in tab_bar_rects_for_window(window_id, ctx) {
            let tab_bar_position_on_screen = RectF::new(
                vec2f(
                    window_bounds.min_x() + tab_bar_position.min_x(),
                    window_bounds.min_y() + tab_bar_position.min_y(),
                ),
                tab_bar_position.size(),
            );
            if !expanded_rect(tab_bar_position_on_screen, TAB_BAR_HIT_MARGIN)
                .contains_point(cursor_position_on_screen)
            {
                continue;
            }

            let insertion_index = workspace.read(ctx, |workspace, ctx| {
                workspace.tab_insertion_index_for_cursor(window_id, cursor_position_on_screen, ctx)
            });
            update_best_target(window_id, insertion_index, tab_bar_position_on_screen);
        }
    }

    best_target.map(|(_, target)| target)
}

/// Returns `rect` expanded outward by `margin` pixels on every side.
fn expanded_rect(rect: RectF, margin: f32) -> RectF {
    RectF::new(
        vec2f(rect.min_x() - margin, rect.min_y() - margin),
        vec2f(rect.width() + 2.0 * margin, rect.height() + 2.0 * margin),
    )
}

fn compute_insertion_index_for_window(
    target_window_id: WindowId,
    caller_window_id: WindowId,
    cursor_position_on_screen: Vector2F,
    ctx: &AppContext,
) -> usize {
    if target_window_id == caller_window_id {
        if let Some(ws) = WorkspaceRegistry::as_ref(ctx).get(caller_window_id, ctx) {
            return ws.read(ctx, |workspace, ctx| {
                workspace.tab_insertion_index_for_cursor(
                    target_window_id,
                    cursor_position_on_screen,
                    ctx,
                )
            });
        }
    }

    if let Some(ws) = WorkspaceRegistry::as_ref(ctx).get(target_window_id, ctx) {
        ws.read(ctx, |workspace, ctx| {
            workspace.tab_insertion_index_for_cursor(
                target_window_id,
                cursor_position_on_screen,
                ctx,
            )
        })
    } else {
        0
    }
}
