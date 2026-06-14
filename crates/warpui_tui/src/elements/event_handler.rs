use warpui_core::keymap::Keystroke;
use warpui_core::{AppContext, Event};

use crate::elements::{TuiElement, TuiPresentationContext};
use crate::{TuiBuffer, TuiConstraint, TuiDispatchEventResult, TuiEventContext, TuiRect, TuiSize};

type TuiEventCallback =
    dyn FnMut(&mut TuiEventContext, &AppContext, &Event) -> TuiDispatchEventResult;
type TuiKeyCallback =
    dyn FnMut(&mut TuiEventContext, &AppContext, &Keystroke) -> TuiDispatchEventResult;

pub struct TuiEventHandler {
    child: Box<dyn TuiElement>,
    event: Option<Box<TuiEventCallback>>,
    key_down: Option<Box<TuiKeyCallback>>,
}

impl TuiEventHandler {
    pub fn new(child: impl TuiElement + 'static) -> Self {
        Self {
            child: Box::new(child),
            event: None,
            key_down: None,
        }
    }

    pub fn on_event<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut TuiEventContext, &AppContext, &Event) -> TuiDispatchEventResult,
    {
        self.event = Some(Box::new(callback));
        self
    }

    pub fn on_key_down<F>(mut self, callback: F) -> Self
    where
        F: 'static + FnMut(&mut TuiEventContext, &AppContext, &Keystroke) -> TuiDispatchEventResult,
    {
        self.key_down = Some(Box::new(callback));
        self
    }
}

impl TuiElement for TuiEventHandler {
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

        if let Event::KeyDown { keystroke, .. } = event {
            if let Some(callback) = self.key_down.as_mut() {
                if matches!(
                    callback(ctx, app, keystroke),
                    TuiDispatchEventResult::StopPropagation
                ) {
                    return true;
                }
            }
        }

        if let Some(callback) = self.event.as_mut() {
            return matches!(
                callback(ctx, app, event),
                TuiDispatchEventResult::StopPropagation
            );
        }

        false
    }
}
