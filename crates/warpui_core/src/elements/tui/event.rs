//! TUI event-dispatch types.
//!
//! The TUI runtime (`crate::runtime`) converts raw crossterm events into
//! [`TuiEvent`]s, then walks the rendered element tree handing each element the
//! event plus a [`TuiEventContext`] it can use to queue notifications and typed
//! actions back into the shared core.
//!
//! This module holds the dispatch-side types that are part of the
//! [`TuiElement`](super::TuiElement) contract; the crossterm → warp event
//! conversion lives with the runtime.

use std::collections::HashSet;
use std::rc::Rc;

use super::{
    TuiElement, TuiLocalPoint, TuiPoint, TuiScene, TuiScreenPoint, TuiScreenRect, TuiSize,
    TuiViewMapContext,
};
use crate::event::{KeyEventDetails, ModifiersState};
use crate::keymap::Keystroke;
use crate::{Action, EntityId, EntityIdMap};

/// A terminal scroll delta `(columns, rows)`.
pub type TuiScrollDelta = (isize, isize);

/// Input events dispatched through TUI elements.
#[derive(Clone, Debug)]
pub enum TuiEvent {
    KeyDown {
        keystroke: Keystroke,
        chars: String,
        details: KeyEventDetails,
        is_composing: bool,
    },
    Paste {
        text: String,
    },
    ScrollWheel {
        position: TuiPoint,
        delta: TuiScrollDelta,
        precise: bool,
        modifiers: ModifiersState,
    },
    LeftMouseDown {
        position: TuiPoint,
        modifiers: ModifiersState,
        click_count: u32,
        is_first_mouse: bool,
    },
    LeftMouseUp {
        position: TuiPoint,
        modifiers: ModifiersState,
    },
    LeftMouseDragged {
        position: TuiPoint,
        modifiers: ModifiersState,
    },
    MiddleMouseDown {
        position: TuiPoint,
        modifiers: ModifiersState,
        click_count: u32,
    },
    RightMouseDown {
        position: TuiPoint,
        modifiers: ModifiersState,
        click_count: u32,
    },
    MouseMoved {
        position: TuiPoint,
        modifiers: ModifiersState,
        is_synthetic: bool,
    },
}

impl TuiEvent {
    /// Returns the terminal-cell position carried by pointer-like events.
    pub fn position(&self) -> Option<TuiPoint> {
        match self {
            Self::ScrollWheel { position, .. }
            | Self::LeftMouseDown { position, .. }
            | Self::LeftMouseUp { position, .. }
            | Self::LeftMouseDragged { position, .. }
            | Self::MiddleMouseDown { position, .. }
            | Self::RightMouseDown { position, .. }
            | Self::MouseMoved { position, .. } => Some(*position),
            Self::KeyDown { .. } | Self::Paste { .. } => None,
        }
    }

    /// Returns the keymap data carried by key-down events.
    pub(crate) fn key_down(&self) -> Option<(&Keystroke, bool)> {
        match self {
            Self::KeyDown {
                keystroke,
                is_composing,
                ..
            } => Some((keystroke, *is_composing)),
            _ => None,
        }
    }
}

/// Whether an element that handled an event wants its ancestors to keep seeing
/// it. Returned by event-aware elements during dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiDispatchEventResult {
    /// Continue offering the event to ancestor elements.
    PropagateToParent,
    /// Consume the event; ancestors do not see it.
    StopPropagation,
}

/// The outcome of dispatching an event through a rendered tree: whether any
/// element handled it (e.g. to decide if a redraw is warranted).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TuiEventDispatchResult {
    pub handled: bool,
}

pub struct TuiEventContext<'a> {
    scene: Rc<TuiScene>,
    rendered_views: &'a mut EntityIdMap<Box<dyn TuiElement>>,
    notified: HashSet<EntityId>,
    typed_actions: Vec<TuiDispatchedAction>,
    origin_view_id: Option<EntityId>,
}

/// A typed action queued during element-tree dispatch, attributed to the view
/// whose subtree raised it. Drained by the runtime, which dispatches it
/// through the shared responder chain rooted at the origin view.
pub(crate) struct TuiDispatchedAction {
    pub(crate) origin_view_id: EntityId,
    pub(crate) action: Box<dyn Action>,
}

impl<'a> TuiEventContext<'a> {
    /// Creates dispatch state for the last painted element tree and scene.
    pub fn new(
        scene: Rc<TuiScene>,
        rendered_views: &'a mut EntityIdMap<Box<dyn TuiElement>>,
    ) -> Self {
        Self {
            scene,
            rendered_views,
            notified: HashSet::new(),
            typed_actions: Vec::new(),
            origin_view_id: None,
        }
    }

    /// Returns the visible portion of retained element bounds.
    pub fn visible_rect(&self, origin: TuiScreenPoint, size: TuiSize) -> Option<TuiScreenRect> {
        self.scene.visible_rect(origin, size)
    }

    /// Returns whether a higher painted layer covers `point`.
    pub fn is_covered(&self, point: TuiScreenPoint) -> bool {
        self.scene.is_covered(point)
    }

    /// Converts a terminal pointer position to signed element-local cells.
    pub fn local_point(&self, origin: TuiScreenPoint, position: TuiPoint) -> TuiLocalPoint {
        TuiLocalPoint::new(
            i32::from(position.x).saturating_sub(origin.x),
            i32::from(position.y).saturating_sub(origin.y),
        )
    }

    /// Returns whether a pointer is inside visible, uncovered element bounds.
    pub fn hit_test(&self, origin: TuiScreenPoint, size: TuiSize, position: TuiPoint) -> bool {
        self.visible_rect(origin, size)
            .is_some_and(|rect| rect.contains(position))
            && !self.is_covered(TuiScreenPoint::new(
                i32::from(position.x),
                i32::from(position.y),
                origin.z_index,
            ))
    }
    /// Queues a typed action to dispatch from the view currently being
    /// processed. Panics if called outside of view event processing, where
    /// there is no origin view to attribute the action to.
    pub fn dispatch_typed_action(&mut self, action: impl Action) {
        let origin_view_id = self
            .origin_view_id
            .expect("typed actions can only be dispatched while processing a rendered TUI view");
        self.typed_actions.push(TuiDispatchedAction {
            origin_view_id,
            action: Box::new(action),
        });
    }

    /// Queues a notification for the current event origin.
    pub fn notify(&mut self) {
        let origin_view_id = self
            .origin_view_id
            .expect("notifications can only be queued while processing a rendered TUI view");
        self.notified.insert(origin_view_id);
    }

    /// Drains notifications after dispatch so each origin view is notified once.
    pub(crate) fn take_notified(&mut self) -> HashSet<EntityId> {
        std::mem::take(&mut self.notified)
    }

    pub(crate) fn take_typed_actions(&mut self) -> Vec<TuiDispatchedAction> {
        std::mem::take(&mut self.typed_actions)
    }

    /// Sets the view that subsequently dispatched actions are attributed to,
    /// returning the previous origin so callers can restore it when leaving the
    /// view's subtree.
    pub fn set_origin_view(&mut self, view_id: Option<EntityId>) -> Option<EntityId> {
        std::mem::replace(&mut self.origin_view_id, view_id)
    }
}

impl TuiViewMapContext for TuiEventContext<'_> {
    fn rendered_views_mut(&mut self) -> &mut EntityIdMap<Box<dyn TuiElement>> {
        self.rendered_views
    }
}
