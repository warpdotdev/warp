use std::collections::HashMap;

use itertools::Itertools;
use pathfinder_color::ColorU;

use crate::elements::{
    Align, Container, Fill, Hoverable, MouseStateHandle, ScrollStateHandle, Scrollable,
    ScrollableElement, ScrollbarWidth, Text, UniformList, UniformListState,
};
use crate::{
    AppContext, Element, Entity, EntityId, TypedActionView, View, ViewContext, WeakViewHandle,
    WindowId,
};

/// Turns a map of parent->children into a list of (view, tree_depth) pairs,
/// ordered via depth-first traversal of the input map.
fn populate_view_list(
    view_children_map: &HashMap<EntityId, Vec<EntityId>>,
    current_view_id: EntityId,
    depth: usize,
    view_list: &mut Vec<(EntityId, usize)>,
) {
    view_list.push((current_view_id, depth));
    if let Some(children) = view_children_map.get(&current_view_id) {
        for child in children {
            populate_view_list(view_children_map, *child, depth + 1, view_list);
        }
    }
}

/// Helper structure containing state necessary to render information about a
/// single view in a window's view hierarchy.
#[derive(Debug, Clone)]
struct ViewInfo {
    view_id: EntityId,
    view_depth: usize,
    mouse_state_handle: MouseStateHandle,
}

impl ViewInfo {
    fn render(&self, window_id: WindowId, ctx: &AppContext) -> Box<dyn Element> {
        let spacing = "  ".repeat(self.view_depth);
        let view_id = self.view_id;
        let view_name = ctx
            .view_name(window_id, view_id)
            .expect("view should exist");

        Hoverable::new(self.mouse_state_handle.clone(), |mouse_state| {
            let text = Text::new_inline(
                format!("{spacing}{view_name} ({view_id:?})"),
                // This relies on an expectation that the first font loaded is
                // a reasonable one to draw this view with, but we lack a better
                // method, at the moment, to intentionally select a font.
                // TODO(vorporeal): Don't arbitrarily pick font family 0.
                crate::fonts::FamilyId(0),
                13.,
            );
            let background_color = if mouse_state.is_hovered() {
                ColorU::new(0, 143, 143, 255)
            } else {
                ColorU::transparent_black()
            };
            Container::new(text.finish())
                .with_background_color(background_color)
                .finish()
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ViewTreeDebugAction::HighlightView(window_id, view_id));
        })
        .finish()
    }
}

/// Actions that can be taken within the View Tree debug view.
#[derive(Debug, Clone)]
pub(super) enum ViewTreeDebugAction {
    /// Visually highlights a particular view in a given window.
    HighlightView(WindowId, EntityId),
}

/// A view to help visualize and interact with the view hierarchy for a
/// particular window.
///
/// This only includes views that had been laid out at some point prior to the
/// creation of this view.  At present, this view caches the view hierarchy at
/// creation time and does not dynamically update it as new views are laid out
/// and rendered.
pub(super) struct ViewTreeDebugView {
    handle: WeakViewHandle<Self>,
    target_window_id: WindowId,
    view_info: Vec<ViewInfo>,
    uniform_list_state: UniformListState,
    scroll_state_handle: ScrollStateHandle,
}

impl ViewTreeDebugView {
    pub fn new(
        target_window_id: WindowId,
        view_parent_map: HashMap<EntityId, EntityId>,
        root_view_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let mut view_children_map: HashMap<EntityId, Vec<EntityId>> = Default::default();
        for (child, parent) in view_parent_map.into_iter() {
            view_children_map.entry(parent).or_default().push(child);
        }

        let mut view_list: Vec<(EntityId, usize)> = vec![];
        populate_view_list(&view_children_map, root_view_id, 0, &mut view_list);

        let view_info = view_list
            .into_iter()
            .map(|(view_id, view_depth)| ViewInfo {
                view_id,
                view_depth,
                mouse_state_handle: MouseStateHandle::default(),
            })
            .collect_vec();

        Self {
            handle: ctx.handle(),
            target_window_id,
            view_info,
            uniform_list_state: Default::default(),
            scroll_state_handle: Default::default(),
        }
    }
}

impl Entity for ViewTreeDebugView {
    type Event = ();
}

impl View for ViewTreeDebugView {
    fn ui_name() -> &'static str {
        "ViewTreeDebugView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        let handle = self.handle.clone();
        let window_id = self.target_window_id;

        let list = UniformList::new(
            self.uniform_list_state.clone(),
            self.view_info.len(),
            move |range, ctx| {
                handle
                    .upgrade(ctx)
                    .into_iter()
                    .flat_map(|handle| {
                        handle
                            .as_ref(ctx)
                            .view_info
                            .iter()
                            .skip(range.start)
                            .take(range.len())
                            .cloned()
                    })
                    .map(|view_info| view_info.render(window_id, ctx))
                    .collect_vec()
                    .into_iter()
            },
        );

        let scrollable = Scrollable::vertical(
            self.scroll_state_handle.clone(),
            list.finish_scrollable(),
            ScrollbarWidth::Auto,
            Fill::Solid(ColorU::new(255, 255, 255, 50)),
            Fill::Solid(ColorU::new(255, 255, 255, 150)),
            Fill::Solid(ColorU::black()),
        );

        let view_content = Align::new(
            Container::new(scrollable.finish())
                .with_uniform_padding(8.)
                .finish(),
        )
        .top_left();

        Container::new(view_content.finish())
            .with_background_color(ColorU::black())
            .with_padding_top(25.)
            .finish()
    }
}

impl TypedActionView for ViewTreeDebugView {
    type Action = ViewTreeDebugAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ViewTreeDebugAction::HighlightView(window_id, view_id) => {
                if let Some(presenter) = ctx.presenter(*window_id) {
                    presenter
                        .as_ref()
                        .borrow_mut()
                        .set_highlighted_view(*view_id);
                }
                // This is a hacky way to get the other window to redraw, but it
                // works. :)
                ctx.invalidate_all_views();
            }
        }
    }
}
