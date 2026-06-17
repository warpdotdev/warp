mod context;
mod handle;

use std::any::Any;

pub use context::TuiViewContext;
pub use handle::{ReadTuiView, TuiViewAsRef, TuiViewHandle, UpdateTuiView, WeakTuiViewHandle};

use crate::{keymap, Action, AppContext, Entity, EntityId, WindowId};

pub trait TuiView: Entity {
    type RenderOutput: Any;

    fn ui_name() -> &'static str;

    fn render_tui(&self, app: &AppContext) -> Self::RenderOutput;

    fn keymap_context(&self, _: &AppContext) -> keymap::Context {
        let mut ctx = keymap::Context::default();
        ctx.set.insert(Self::ui_name());
        ctx
    }
}

pub trait TuiTypedActionView: TuiView {
    type Action: Action;

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut TuiViewContext<Self>) {}
}

pub trait AnyTuiView {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn ui_name(&self) -> &'static str;
    fn render_tui(&self, app: &AppContext) -> Box<dyn Any>;
    fn keymap_context(&self, app: &AppContext) -> keymap::Context;
}

impl<T> AnyTuiView for T
where
    T: TuiView,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn ui_name(&self) -> &'static str {
        T::ui_name()
    }

    fn render_tui(&self, app: &AppContext) -> Box<dyn Any> {
        Box::new(TuiView::render_tui(self, app))
    }

    fn keymap_context(&self, app: &AppContext) -> keymap::Context {
        TuiView::keymap_context(self, app)
    }
}

pub(in crate::core) struct AnyTuiViewHandle {
    pub(super) window_id: WindowId,
    pub(super) view_id: EntityId,
    pub(super) view_type: std::any::TypeId,
    pub(super) ref_counts: std::sync::Weak<parking_lot::Mutex<crate::core::RefCounts>>,
}
