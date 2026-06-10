use warpui_core::{AppContext, Event};

use crate::elements::{TuiElement, TuiPresentationContext};
use crate::{TuiBuffer, TuiConstraint, TuiEventContext, TuiRect, TuiSize};

pub struct TuiContainer {
    child: Box<dyn TuiElement>,
    border: bool,
    size: TuiSize,
}

impl TuiContainer {
    pub fn new(child: impl TuiElement + 'static) -> Self {
        Self {
            child: Box::new(child),
            border: false,
            size: TuiSize::default(),
        }
    }

    pub fn with_border(mut self) -> Self {
        self.border = true;
        self
    }

    fn child_area(&self, area: TuiRect) -> TuiRect {
        if self.border {
            area.inset(1)
        } else {
            area
        }
    }

    fn border_size(&self) -> u16 {
        if self.border {
            2
        } else {
            0
        }
    }
}

impl TuiElement for TuiContainer {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        let border_size = self.border_size();
        let child_size = self.child.layout(TuiConstraint::new(
            TuiSize::new(
                constraint.min.width.saturating_sub(border_size),
                constraint.min.height.saturating_sub(border_size),
            ),
            TuiSize::new(
                constraint.max.width.saturating_sub(border_size),
                constraint.max.height.saturating_sub(border_size),
            ),
        ));
        self.size = TuiSize::new(
            child_size.width.saturating_add(border_size),
            child_size.height.saturating_add(border_size),
        );
        self.size
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        if self.border && area.width >= 2 && area.height >= 2 {
            for x in area.x..area.right() {
                buffer.set_symbol(x, area.y, '─');
                buffer.set_symbol(x, area.bottom().saturating_sub(1), '─');
            }
            for y in area.y..area.bottom() {
                buffer.set_symbol(area.x, y, '│');
                buffer.set_symbol(area.right().saturating_sub(1), y, '│');
            }
            buffer.set_symbol(area.x, area.y, '┌');
            buffer.set_symbol(area.right().saturating_sub(1), area.y, '┐');
            buffer.set_symbol(area.x, area.bottom().saturating_sub(1), '└');
            buffer.set_symbol(
                area.right().saturating_sub(1),
                area.bottom().saturating_sub(1),
                '┘',
            );
        }

        self.child.render(self.child_area(area), buffer);
    }

    fn desired_height(&self, width: u16) -> u16 {
        let border_size = self.border_size();
        self.child
            .desired_height(width.saturating_sub(border_size))
            .saturating_add(border_size)
    }

    fn cursor_position(&self, area: TuiRect) -> Option<(u16, u16)> {
        self.child.cursor_position(self.child_area(area))
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &Event,
        area: TuiRect,
        ctx: &mut TuiEventContext,
        app: &AppContext,
    ) -> bool {
        self.child
            .dispatch_event(event, self.child_area(area), ctx, app)
    }
}
