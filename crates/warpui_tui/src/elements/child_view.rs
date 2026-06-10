use warpui_core::{AppContext, EntityId, Event, TuiView, TuiViewHandle};

use crate::elements::{TuiElement, TuiPresentationContext};
use crate::{TuiBuffer, TuiConstraint, TuiEventContext, TuiRect, TuiSize};

pub struct TuiChildView {
    view_id: EntityId,
    child: Box<dyn TuiElement>,
}

impl TuiChildView {
    pub fn new<V>(handle: &TuiViewHandle<V>, app: &AppContext) -> Self
    where
        V: TuiView<RenderOutput = Box<dyn TuiElement>>,
    {
        Self {
            view_id: handle.id(),
            child: handle.read(app, |view, ctx| view.render_tui(ctx)),
        }
    }
}

impl TuiElement for TuiChildView {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        self.child.layout(constraint)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        self.child.render(area, buffer);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.child.desired_height(width)
    }

    fn cursor_position(&self, area: TuiRect) -> Option<(u16, u16)> {
        self.child.cursor_position(area)
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        ctx.enter_child(self.view_id);
        self.child.present(ctx);
        ctx.exit_child();
    }

    fn dispatch_event(
        &mut self,
        event: &Event,
        area: TuiRect,
        ctx: &mut TuiEventContext,
        app: &AppContext,
    ) -> bool {
        let previous_origin = ctx.set_origin_view(Some(self.view_id));
        let handled = self.child.dispatch_event(event, area, ctx, app);
        ctx.set_origin_view(previous_origin);
        handled
    }
}
