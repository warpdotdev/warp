use std::collections::HashMap;

use warpui_core::EntityId;

mod child_view;
mod column;
mod container;
mod event_handler;
mod mouse_area;
mod text;

pub use child_view::TuiChildView;
pub use column::TuiColumn;
pub use container::TuiContainer;
pub use event_handler::TuiEventHandler;
pub use mouse_area::{TuiMouseArea, TuiMouseState, TuiMouseStateHandle};
pub use text::TuiText;
use warpui_core::{AppContext, Event};

use crate::{TuiBuffer, TuiConstraint, TuiEventContext, TuiRect, TuiSize};
pub struct TuiPresentationContext<'a> {
    parent_by_child: &'a mut HashMap<EntityId, EntityId>,
    view_stack: Vec<EntityId>,
}

impl<'a> TuiPresentationContext<'a> {
    pub(crate) fn new(
        root_view_id: EntityId,
        parent_by_child: &'a mut HashMap<EntityId, EntityId>,
    ) -> Self {
        Self {
            parent_by_child,
            view_stack: vec![root_view_id],
        }
    }

    pub(crate) fn enter_child(&mut self, child_view_id: EntityId) {
        let parent_view_id = *self
            .view_stack
            .last()
            .expect("the TUI presentation stack contains a root view");
        self.parent_by_child
            .insert(child_view_id, parent_view_id);
        self.view_stack.push(child_view_id);
    }

    pub(crate) fn exit_child(&mut self) {
        self.view_stack
            .pop()
            .expect("a child view is entered before it is exited");
    }
}

pub trait TuiElement {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize;

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer);

    fn desired_height(&self, width: u16) -> u16;

    fn cursor_position(&self, _area: TuiRect) -> Option<(u16, u16)> {
        None
    }
    fn present(&mut self, _ctx: &mut TuiPresentationContext<'_>) {}

    fn dispatch_event(
        &mut self,
        _event: &Event,
        _area: TuiRect,
        _ctx: &mut TuiEventContext,
        _app: &AppContext,
    ) -> bool {
        false
    }
}

impl TuiElement for () {
    fn layout(&mut self, _: TuiConstraint) -> TuiSize {
        TuiSize::default()
    }

    fn render(&self, _: TuiRect, _: &mut TuiBuffer) {}

    fn desired_height(&self, _: u16) -> u16 {
        0
    }
}
