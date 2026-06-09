//! TUI event plumbing.
//!
//! The runtime converts raw crossterm events into the shared
//! [`warpui_core::Event`] vocabulary (so element/view dispatch is identical to
//! the GUI), then walks the rendered element tree handing each element the
//! event plus a [`TuiEventContext`] it can use to queue app updates and typed
//! actions back into the shared core.
//!
//! This module freezes the event *types* and the *signature* of
//! [`crossterm_event_to_warp_event`]. The crossterm→warp mapping itself, along
//! with any scroll/mouse normalization helpers, is implemented by the renderer/
//! runtime layer (task 3.4).

use crossterm::event::Event as CrosstermEvent;
use warpui_core::{Action, App, EntityId, Event};

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

/// Collects the side effects an element requests while handling an event:
/// deferred mutations of the [`App`] and typed actions to dispatch through the
/// shared core. The runtime drains these after dispatch and applies them on the
/// main thread, mirroring how GUI event handlers defer work via the app context.
#[derive(Default)]
pub struct TuiEventContext {
    updates: Vec<TuiAppUpdate>,
    typed_actions: Vec<TuiDispatchedAction>,
    origin_view_id: Option<EntityId>,
}

// Consumed by the runtime's dispatch loop (task 3.4); inert until then.
#[allow(dead_code)]
pub(crate) struct TuiDispatchedAction {
    pub(crate) origin_view_id: EntityId,
    pub(crate) action: Box<dyn Action>,
}

impl TuiEventContext {
    /// Queues a closure to run against the [`App`] once dispatch completes.
    pub fn dispatch_app_update<F>(&mut self, update: F)
    where
        F: 'static + FnOnce(&mut App),
    {
        self.updates.push(Box::new(update));
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

    // The `take_*`/`set_origin_view` plumbing is drained by the runtime's
    // dispatch loop (task 3.4); it is inert within the foundation crate alone.
    #[allow(dead_code)]
    pub(crate) fn take_updates(&mut self) -> Vec<TuiAppUpdate> {
        std::mem::take(&mut self.updates)
    }

    #[allow(dead_code)]
    pub(crate) fn take_typed_actions(&mut self) -> Vec<TuiDispatchedAction> {
        std::mem::take(&mut self.typed_actions)
    }

    /// Sets the view that subsequently dispatched actions are attributed to,
    /// returning the previous origin so callers can restore it when leaving the
    /// view's subtree.
    #[allow(dead_code)]
    pub(crate) fn set_origin_view(&mut self, view_id: Option<EntityId>) -> Option<EntityId> {
        std::mem::replace(&mut self.origin_view_id, view_id)
    }
}

/// Converts a raw crossterm event into the shared [`warpui_core::Event`]
/// vocabulary, or `None` if the event has no warp equivalent.
///
/// FROZEN SIGNATURE. The mapping is implemented by the renderer/runtime layer
/// (task 3.4); the signature is fixed here so sibling tasks can build against
/// it.
pub fn crossterm_event_to_warp_event(event: CrosstermEvent) -> Option<Event> {
    todo!("crossterm_event_to_warp_event mapping is implemented in task 3.4: {event:?}")
}
