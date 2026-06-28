//! TUI event-dispatch types.
//!
//! The TUI runtime (`crate::runtime`) converts raw crossterm events into the
//! shared [`Event`](crate::Event) vocabulary (so element/view dispatch is
//! identical to the GUI), then walks the rendered element tree handing each
//! element the event plus a [`TuiEventContext`] it can use to queue app
//! updates and typed actions back into the shared core.
//!
//! This module holds the dispatch-side types that are part of the
//! [`TuiElement`](super::TuiElement) contract; the crossterm → warp event
//! conversion lives with the runtime.

use crate::{Action, App, EntityId};

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

type TuiAppUpdate = Box<dyn FnOnce(&mut App)>;

#[derive(Default)]
pub struct TuiEventContext {
    updates: Vec<TuiAppUpdate>,
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

impl TuiEventContext {
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

    pub(crate) fn take_updates(&mut self) -> Vec<TuiAppUpdate> {
        std::mem::take(&mut self.updates)
    }

    pub(crate) fn take_typed_actions(&mut self) -> Vec<TuiDispatchedAction> {
        std::mem::take(&mut self.typed_actions)
    }

    /// Sets the view that subsequently dispatched actions are attributed to,
    /// returning the previous origin so callers can restore it when leaving the
    /// view's subtree.
    pub(crate) fn set_origin_view(&mut self, view_id: Option<EntityId>) -> Option<EntityId> {
        std::mem::replace(&mut self.origin_view_id, view_id)
    }
}
