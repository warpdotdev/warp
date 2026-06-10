use std::cell::RefCell;
use std::rc::Rc;

use warpui_core::geometry::vector::Vector2F;
use warpui_core::{AppContext, Event};

use crate::elements::{TuiElement, TuiPresentationContext};
use crate::{TuiBuffer, TuiConstraint, TuiEventContext, TuiRect, TuiSize};

type TuiMouseHandler = dyn FnMut(&mut TuiEventContext, &AppContext, Vector2F);

#[derive(Clone, Debug, Default)]
pub struct TuiMouseState {
    click_count: Option<u32>,
}

impl TuiMouseState {
    pub fn is_clicked(&self) -> bool {
        self.click_count.is_some()
    }

    pub fn click_count(&self) -> Option<u32> {
        self.click_count
    }

    fn set_clicked(&mut self, click_count: u32) {
        self.click_count = Some(click_count);
    }

    fn clear_clicked(&mut self) {
        self.click_count = None;
    }
}

pub type TuiMouseStateHandle = Rc<RefCell<TuiMouseState>>;

pub struct TuiMouseArea {
    child: Box<dyn TuiElement>,
    state: TuiMouseStateHandle,
    click_handler: Option<Box<TuiMouseHandler>>,
    mouse_down_handler: Option<Box<TuiMouseHandler>>,
    disabled: bool,
}

impl TuiMouseArea {
    pub fn new<F>(state: TuiMouseStateHandle, build_child: F) -> Self
    where
        F: FnOnce(&TuiMouseState) -> Box<dyn TuiElement>,
    {
        let child = build_child(&state.borrow());
        Self {
            child,
            state,
            click_handler: None,
            mouse_down_handler: None,
            disabled: false,
        }
    }

    pub fn on_click<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut TuiEventContext, &AppContext, Vector2F),
    {
        self.click_handler = Some(Box::new(callback));
        self
    }

    pub fn on_mouse_down<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut TuiEventContext, &AppContext, Vector2F),
    {
        self.mouse_down_handler = Some(Box::new(callback));
        self
    }

    pub fn disable(mut self) -> Self {
        self.disabled = true;
        self
    }

    fn handle_left_mouse_down(
        &mut self,
        area: TuiRect,
        position: Vector2F,
        click_count: u32,
        ctx: &mut TuiEventContext,
        app: &AppContext,
    ) -> bool {
        if !area.contains_position(position)
            || (self.click_handler.is_none() && self.mouse_down_handler.is_none())
        {
            return false;
        }

        self.state.borrow_mut().set_clicked(click_count);
        if let Some(handler) = self.mouse_down_handler.as_mut() {
            handler(ctx, app, position);
        }
        true
    }

    fn handle_left_mouse_up(
        &mut self,
        area: TuiRect,
        position: Vector2F,
        ctx: &mut TuiEventContext,
        app: &AppContext,
    ) -> bool {
        let was_clicked = self.state.borrow().is_clicked();
        self.state.borrow_mut().clear_clicked();
        if !was_clicked || !area.contains_position(position) {
            return false;
        }

        if let Some(handler) = self.click_handler.as_mut() {
            handler(ctx, app, position);
            true
        } else {
            false
        }
    }
}

impl TuiElement for TuiMouseArea {
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
        self.child.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &Event,
        area: TuiRect,
        ctx: &mut TuiEventContext,
        app: &AppContext,
    ) -> bool {
        if self.child.dispatch_event(event, area, ctx, app) {
            return true;
        }

        if self.disabled {
            return false;
        }

        match event {
            Event::LeftMouseDown {
                position,
                click_count,
                ..
            } => self.handle_left_mouse_down(area, *position, *click_count, ctx, app),
            Event::LeftMouseUp { position, .. } => {
                self.handle_left_mouse_up(area, *position, ctx, app)
            }
            _ => false,
        }
    }
}
