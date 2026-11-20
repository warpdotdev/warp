//! Type-erased plumbing behind `subscribe_to_model` and `subscribe_to_view` on `ModelContext` and
//! `ViewContext`.
//!
//! Every call to one of those methods passes a different closure, so the compiler generates the
//! method's code once per call. To keep that code small, the typed methods only box the closure
//! inside an adapter. Registering the subscription is left to [`AppContext`] methods that are
//! compiled once, and the adapters recover the typed arguments through helpers that are compiled
//! once per entity or event type.
//!
//! The adapters implement dedicated traits rather than `dyn FnMut`, because a `dyn FnMut` vtable
//! also has a `call_once` entry that the compiler has to generate for every closure.

use std::any::Any;
use std::marker::PhantomData;

use super::Subscription;
use super::spawn::downcast_entity;
use crate::{
    AppContext, Entity, EntityId, ModelContext, ModelHandle, ViewContext, ViewHandle,
    WeakModelHandle, WeakViewHandle, WindowId,
};

/// A type-erased event callback that runs against the subscribing model.
pub(super) trait ModelEventCallback {
    fn call(
        &mut self,
        model: &mut dyn Any,
        event: &dyn Any,
        emitter_id: EntityId,
        model_id: EntityId,
        ctx: &mut AppContext,
    );
}

/// A type-erased event callback that runs against the subscribing view.
pub(super) trait ViewEventCallback {
    fn call(
        &mut self,
        view: &mut dyn Any,
        event: &dyn Any,
        emitter_id: EntityId,
        window_id: WindowId,
        view_id: EntityId,
        ctx: &mut AppContext,
    );
}

/// A strong handle to an entity whose events can be subscribed to.
pub(super) trait EmitterHandle: Sized {
    type Event: 'static;

    /// Returns a handle to the emitter, or `None` if the emitter no longer exists.
    fn upgrade(emitter_id: EntityId, ctx: &AppContext) -> Option<Self>;
}

impl<E> EmitterHandle for ModelHandle<E>
where
    E: Entity,
    E::Event: 'static,
{
    type Event = E::Event;

    fn upgrade(emitter_id: EntityId, ctx: &AppContext) -> Option<Self> {
        WeakModelHandle::new(emitter_id).upgrade(ctx)
    }
}

impl<E> EmitterHandle for ViewHandle<E>
where
    E: Entity,
    E::Event: 'static,
{
    type Event = E::Event;

    fn upgrade(emitter_id: EntityId, ctx: &AppContext) -> Option<Self> {
        WeakViewHandle::new(emitter_id).upgrade(ctx)
    }
}

fn downcast_event<E: 'static>(event: &dyn Any) -> &E {
    event
        .downcast_ref()
        .expect("the typed entry point fixes the event type")
}

/// Adapts a typed event callback, for a subscribing model of type `T` and an emitter with handle
/// type `H`, to [`ModelEventCallback`].
pub(super) struct ModelSubscriber<T, H, F>(pub(super) F, pub(super) PhantomData<fn(&mut T, H)>);

impl<T, H, F> ModelEventCallback for ModelSubscriber<T, H, F>
where
    T: Entity,
    H: EmitterHandle,
    F: FnMut(&mut T, H, &H::Event, &mut ModelContext<T>),
{
    fn call(
        &mut self,
        model: &mut dyn Any,
        event: &dyn Any,
        emitter_id: EntityId,
        model_id: EntityId,
        ctx: &mut AppContext,
    ) {
        let model = downcast_entity(model);
        let event = downcast_event(event);
        let mut ctx = ModelContext::new(ctx, model_id);
        // The emitter is upgraded last so that no other call can unwind while its handle is alive,
        // which would add cleanup code to every copy of this function.
        if let Some(emitter) = H::upgrade(emitter_id, &ctx) {
            (self.0)(model, emitter, event, &mut ctx);
        }
    }
}

/// Adapts a typed event callback, for a subscribing view of type `T` and an emitter with handle
/// type `H`, to [`ViewEventCallback`].
pub(super) struct ViewSubscriber<T, H, F>(pub(super) F, pub(super) PhantomData<fn(&mut T, H)>);

impl<T, H, F> ViewEventCallback for ViewSubscriber<T, H, F>
where
    T: Entity,
    H: EmitterHandle,
    F: FnMut(&mut T, H, &H::Event, &mut ViewContext<T>),
{
    fn call(
        &mut self,
        view: &mut dyn Any,
        event: &dyn Any,
        emitter_id: EntityId,
        window_id: WindowId,
        view_id: EntityId,
        ctx: &mut AppContext,
    ) {
        let view = downcast_entity(view);
        let event = downcast_event(event);
        let mut ctx = ViewContext::new(ctx, window_id, view_id);
        // See the comment in `ModelSubscriber::call` for why the emitter is upgraded last.
        if let Some(emitter) = H::upgrade(emitter_id, &ctx) {
            (self.0)(view, emitter, event, &mut ctx);
        }
    }
}

impl AppContext {
    /// Subscribes a model to the events emitted by the entity with ID `emitter_id`.
    pub(super) fn add_model_subscription(
        &mut self,
        emitter_id: EntityId,
        model_id: EntityId,
        callback: Box<dyn ModelEventCallback>,
    ) {
        // Self-subscriptions are disallowed: `emit_event` temporarily removes the subscriber model
        // from `app.models` before invoking callbacks, so upgrading the emitter's handle would
        // return `None` and silently drop the callback.
        debug_assert_ne!(
            emitter_id, model_id,
            "a model must not subscribe to its own events"
        );
        self.subscriptions
            .entry(emitter_id)
            .or_default()
            .push(Subscription::FromModel { model_id, callback });
    }

    /// Subscribes a view to the events emitted by the entity with ID `emitter_id`.
    pub(super) fn add_view_subscription(
        &mut self,
        emitter_id: EntityId,
        window_id: WindowId,
        view_id: EntityId,
        callback: Box<dyn ViewEventCallback>,
    ) {
        self.subscriptions
            .entry(emitter_id)
            .or_default()
            .push(Subscription::FromView {
                window_id,
                view_id,
                callback,
            });
    }
}
