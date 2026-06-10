use warpui_core::{AppContext, Event};

use crate::elements::{TuiElement, TuiPresentationContext};
use crate::{TuiBuffer, TuiConstraint, TuiEventContext, TuiRect, TuiSize};

pub struct TuiColumn {
    children: Vec<Box<dyn TuiElement>>,
    size: TuiSize,
}

impl TuiColumn {
    pub fn new(children: impl IntoIterator<Item = Box<dyn TuiElement>>) -> Self {
        Self {
            children: children.into_iter().collect(),
            size: TuiSize::default(),
        }
    }

    pub fn push(&mut self, child: impl TuiElement + 'static) {
        self.children.push(Box::new(child));
    }
}

impl TuiElement for TuiColumn {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        let mut remaining_height = constraint.max.height;
        let mut total_height: u16 = 0;
        for child in &mut self.children {
            let child_size = child.layout(TuiConstraint::new(
                TuiSize::new(constraint.min.width, 0),
                TuiSize::new(constraint.max.width, remaining_height),
            ));
            remaining_height = remaining_height.saturating_sub(child_size.height);
            total_height = total_height.saturating_add(child_size.height);
        }

        self.size = TuiSize::new(
            constraint.max.width,
            total_height.max(constraint.min.height),
        );
        self.size
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        let mut y = area.y;
        for child in &self.children {
            if y >= area.bottom() {
                break;
            }

            let child_height = child
                .desired_height(area.width)
                .min(area.bottom().saturating_sub(y));
            child.render(TuiRect::new(area.x, y, area.width, child_height), buffer);
            y = y.saturating_add(child_height);
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.children
            .iter()
            .map(|child| child.desired_height(width))
            .sum()
    }

    fn cursor_position(&self, area: TuiRect) -> Option<(u16, u16)> {
        let mut y = area.y;
        for child in &self.children {
            let child_height = child.desired_height(area.width);
            let child_area = TuiRect::new(area.x, y, area.width, child_height);
            if let Some(position) = child.cursor_position(child_area) {
                return Some(position);
            }
            y = y.saturating_add(child_height);
        }
        None
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        for child in &mut self.children {
            child.present(ctx);
        }
    }

    fn dispatch_event(
        &mut self,
        event: &Event,
        area: TuiRect,
        ctx: &mut TuiEventContext,
        app: &AppContext,
    ) -> bool {
        let mut child_areas = Vec::with_capacity(self.children.len());
        let mut y = area.y;
        for child in &self.children {
            if y >= area.bottom() {
                break;
            }

            let child_height = child
                .desired_height(area.width)
                .min(area.bottom().saturating_sub(y));
            child_areas.push(TuiRect::new(area.x, y, area.width, child_height));
            y = y.saturating_add(child_height);
        }

        self.children
            .iter_mut()
            .zip(child_areas)
            .rev()
            .any(|(child, child_area)| child.dispatch_event(event, child_area, ctx, app))
    }
}
