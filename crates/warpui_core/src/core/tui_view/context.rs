use std::marker::PhantomData;

use super::TuiTypedActionView;
use crate::core::{Observation, Subscription};
use crate::{
    AppContext, Effect, Entity, EntityId, ModelContext, ModelHandle, TuiView, TuiViewHandle,
    WeakTuiViewHandle, WindowId,
};

pub struct TuiViewContext<'a, T: ?Sized> {
    app: &'a mut AppContext,
    window_id: WindowId,
    view_id: EntityId,
    view_type: PhantomData<T>,
}

impl<'a, T: TuiView> TuiViewContext<'a, T> {
    pub(in crate::core) fn new(
        app: &'a mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) -> Self {
        Self {
            app,
            window_id,
            view_id,
            view_type: PhantomData,
        }
    }

    pub fn handle(&self) -> WeakTuiViewHandle<T> {
        WeakTuiViewHandle::new(self.view_id)
    }

    pub fn window_id(&self) -> WindowId {
        self.window_id
    }

    pub fn view_id(&self) -> EntityId {
        self.view_id
    }

    pub fn add_model<S, F>(&mut self, build_model: F) -> ModelHandle<S>
    where
        S: Entity,
        F: FnOnce(&mut ModelContext<S>) -> S,
    {
        self.app.add_model(build_model)
    }
    pub fn add_tui_view<S, F>(&mut self, build_view: F) -> TuiViewHandle<S>
    where
        S: TuiView,
        F: FnOnce(&mut TuiViewContext<S>) -> S,
    {
        self.app.add_tui_view(self.window_id, build_view)
    }

    pub fn add_tui_typed_action_view<S, F>(&mut self, build_view: F) -> TuiViewHandle<S>
    where
        S: TuiTypedActionView,
        F: FnOnce(&mut TuiViewContext<S>) -> S,
    {
        self.app
            .add_tui_typed_action_view(self.window_id, build_view)
    }

    pub fn subscribe_to_model<S: Entity, F>(&mut self, handle: &ModelHandle<S>, mut callback: F)
    where
        S::Event: 'static,
        F: 'static + FnMut(&mut T, ModelHandle<S>, &S::Event, &mut TuiViewContext<T>),
    {
        let emitter_handle = handle.downgrade();
        self.app
            .subscriptions
            .entry(handle.id())
            .or_default()
            .push(Subscription::FromView {
                window_id: self.window_id,
                view_id: self.view_id,
                callback: Box::new(move |view, payload, app, window_id, view_id| {
                    if let Some(emitter_handle) = emitter_handle.upgrade(app) {
                        let view = view.downcast_mut().expect("downcast is type safe");
                        let payload = payload.downcast_ref().expect("downcast is type safe");
                        let mut ctx = TuiViewContext::new(app, window_id, view_id);
                        callback(view, emitter_handle, payload, &mut ctx);
                    }
                }),
            });
    }

    pub fn observe<S, F>(&mut self, handle: &ModelHandle<S>, mut callback: F)
    where
        S: Entity,
        F: 'static + FnMut(&mut T, ModelHandle<S>, &mut TuiViewContext<T>),
    {
        self.app
            .observations
            .entry(handle.id())
            .or_default()
            .push(Observation::FromView {
                window_id: self.window_id,
                view_id: self.view_id,
                callback: Box::new(move |view, observed_id, app, window_id, view_id| {
                    let view = view.downcast_mut().expect("downcast is type safe");
                    let observed = ModelHandle::new(observed_id, &app.ref_counts);
                    let mut ctx = TuiViewContext::new(app, window_id, view_id);
                    callback(view, observed, &mut ctx);
                }),
            });
    }

    pub fn emit(&mut self, payload: T::Event)
    where
        T::Event: 'static,
    {
        self.app.pending_effects.push_back(Effect::Event {
            entity_id: self.view_id,
            payload: Box::new(payload),
        });
    }

    pub fn notify(&mut self) {
        self.app
            .pending_effects
            .push_back(Effect::ViewNotification {
                window_id: self.window_id,
                view_id: self.view_id,
            });
    }

    pub fn focus_self(&mut self) {
        self.app.pending_effects.push_back(Effect::Focus {
            window_id: self.window_id,
            view_id: self.view_id,
        });
    }

    pub fn focus<S: TuiView>(&mut self, handle: &TuiViewHandle<S>) {
        self.app.pending_effects.push_back(Effect::Focus {
            window_id: handle.window_id(self.app),
            view_id: handle.id(),
        });
    }
}
