use std::collections::HashMap;

use warpui_core::{App, AppContext, EntityId, Event, WindowId};

use crate::elements::{TuiElement, TuiPresentationContext};
use crate::{
    TuiBuffer, TuiConstraint, TuiEventContext, TuiEventDispatchResult, TuiRect, TuiSize, TuiView,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiFrame {
    pub buffer: TuiBuffer,
    pub cursor_position: Option<(u16, u16)>,
}

pub struct TuiPresenter {
    frame_count: usize,
    root: Option<Box<dyn TuiElement>>,
    root_area: TuiRect,
    window_id: Option<WindowId>,
    root_view_id: Option<EntityId>,
    parent_by_child: HashMap<EntityId, EntityId>,
}

impl TuiPresenter {
    pub fn new() -> Self {
        Self {
            frame_count: 0,
            root: None,
            root_area: TuiRect::default(),
            window_id: None,
            root_view_id: None,
            parent_by_child: HashMap::new(),
        }
    }

    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    pub fn render_view(
        &mut self,
        view: &impl TuiView<RenderOutput = Box<dyn TuiElement>>,
        app: &AppContext,
        size: TuiSize,
    ) -> TuiFrame {
        self.render_root(view.render_tui(app), size, None)
    }

    pub fn render_window(
        &mut self,
        window_id: WindowId,
        root_view_id: EntityId,
        view: &impl TuiView<RenderOutput = Box<dyn TuiElement>>,
        app: &AppContext,
        size: TuiSize,
    ) -> TuiFrame {
        self.render_root(
            view.render_tui(app),
            size,
            Some((window_id, root_view_id)),
        )
    }

    fn render_root(
        &mut self,
        mut root: Box<dyn TuiElement>,
        size: TuiSize,
        identity: Option<(WindowId, EntityId)>,
    ) -> TuiFrame {
        root.layout(TuiConstraint::tight(size));
        self.parent_by_child.clear();
        if let Some((window_id, root_view_id)) = identity {
            self.window_id = Some(window_id);
            self.root_view_id = Some(root_view_id);
            root.present(&mut TuiPresentationContext::new(
                root_view_id,
                &mut self.parent_by_child,
            ));
        } else {
            self.window_id = None;
            self.root_view_id = None;
        }

        let mut buffer = TuiBuffer::new(size);
        let area = TuiRect::new(0, 0, size.width, size.height);
        root.render(area, &mut buffer);
        let cursor_position = root.cursor_position(area);
        self.root = Some(root);
        self.root_area = area;
        self.frame_count += 1;
        TuiFrame {
            buffer,
            cursor_position,
        }
    }
    pub fn responder_chain(&self, view_id: EntityId) -> Vec<EntityId> {
        let Some(root_view_id) = self.root_view_id else {
            return Vec::new();
        };
        let mut chain = vec![view_id];
        while let Some(parent_id) = self.parent_by_child.get(chain.last().unwrap()) {
            chain.push(*parent_id);
        }
        if chain.last().copied() != Some(root_view_id) {
            return vec![root_view_id];
        }
        chain.reverse();
        chain
    }

    pub fn sync_focus(&self, app: &mut App) {
        let (Some(window_id), Some(root_view_id)) = (self.window_id, self.root_view_id) else {
            return;
        };
        let focused_view_id = app.read(|ctx| ctx.focused_tui_view_id(window_id));
        if focused_view_id.is_none_or(|view_id| {
            view_id != root_view_id && !self.parent_by_child.contains_key(&view_id)
        }) {
            app.focus_tui_view(window_id, root_view_id);
        }
    }

    pub fn dispatch_event(&mut self, event: &Event, app: &mut App) -> TuiEventDispatchResult {
        self.sync_focus(app);
        if let (Some(window_id), Some(root_view_id)) = (self.window_id, self.root_view_id) {
            if app.key_bindings_dispatching_enabled(window_id) {
                if let Event::KeyDown { keystroke, .. } = event {
                    let focused_view_id = app
                        .read(|ctx| ctx.focused_tui_view_id(window_id))
                        .filter(|view_id| {
                            *view_id == root_view_id || self.parent_by_child.contains_key(view_id)
                        })
                        .unwrap_or(root_view_id);
                    if app.read(|ctx| ctx.focused_tui_view_id(window_id)) != Some(focused_view_id) {
                        app.focus_tui_view(window_id, focused_view_id);
                    }
                    if app
                        .dispatch_tui_keystroke(
                            window_id,
                            &self.responder_chain(focused_view_id),
                            keystroke,
                        )
                        .unwrap_or(false)
                    {
                        return TuiEventDispatchResult { handled: true };
                    }
                }
            }
        }
        let Some(mut root) = self.root.take() else {
            return TuiEventDispatchResult { handled: false };
        };

        let mut event_ctx = TuiEventContext::default();
        if let Some(root_view_id) = self.root_view_id {
            event_ctx.set_origin_view(Some(root_view_id));
        }
        let mut handled =
            app.read(|ctx| root.dispatch_event(event, self.root_area, &mut event_ctx, ctx));
        self.root = Some(root);
        if let Some(window_id) = self.window_id {
            for dispatched_action in event_ctx.take_typed_actions() {
                handled |= app.dispatch_tui_typed_action(
                    window_id,
                    &self.responder_chain(dispatched_action.origin_view_id),
                    dispatched_action.action.as_ref(),
                );
            }
        }

        for update in event_ctx.take_updates() {
            update(app);
        }

        TuiEventDispatchResult { handled }
    }
}

impl Default for TuiPresenter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use warpui_core::event::KeyEventDetails;
    use warpui_core::geometry::vector::vec2f;
    use warpui_core::keymap::{ContextPredicate, FixedBinding, Keystroke};
    use warpui_core::{
        App, Entity, Event, ModelHandle, TuiTypedActionView, TuiViewContext, TuiViewHandle,
    };

    use super::*;
    use crate::elements::{
        TuiChildView, TuiContainer, TuiEventHandler, TuiMouseArea, TuiMouseStateHandle, TuiText,
    };
    use crate::TuiDispatchEventResult;

    struct GreetingModel {
        greeting: String,
    }

    impl Entity for GreetingModel {
        type Event = ();
    }

    struct GreetingView {
        model: ModelHandle<GreetingModel>,
    }

    impl Entity for GreetingView {
        type Event = ();
    }

    impl TuiView for GreetingView {
        type RenderOutput = Box<dyn crate::elements::TuiElement>;
        fn ui_name() -> &'static str {
            "GreetingView"
        }

        fn render_tui(&self, app: &AppContext) -> Box<dyn crate::elements::TuiElement> {
            let greeting = self.model.read(app, |model, _| model.greeting.clone());
            Box::new(TuiContainer::new(TuiText::new(greeting)).with_border())
        }
    }

    struct EventHandlingView {
        model: ModelHandle<GreetingModel>,
    }

    impl Entity for EventHandlingView {
        type Event = ();
    }

    impl TuiView for EventHandlingView {
        type RenderOutput = Box<dyn crate::elements::TuiElement>;

        fn ui_name() -> &'static str {
            "EventHandlingView"
        }

        fn render_tui(&self, _: &AppContext) -> Box<dyn crate::elements::TuiElement> {
            let model = self.model.clone();
            Box::new(
                TuiEventHandler::new(TuiText::new("press enter")).on_key_down(
                    move |ctx, _, keystroke| {
                        if keystroke.is_unmodified_enter() {
                            let model = model.clone();
                            ctx.dispatch_app_update(move |app| {
                                model.update(app, |model, _| {
                                    model.greeting = "event handled".to_owned();
                                });
                            });
                            TuiDispatchEventResult::StopPropagation
                        } else {
                            TuiDispatchEventResult::PropagateToParent
                        }
                    },
                ),
            )
        }
    }

    struct ClickModel {
        click_count: usize,
    }

    impl Entity for ClickModel {
        type Event = ();
    }

    struct ClickView {
        model: ModelHandle<ClickModel>,
        mouse_state: TuiMouseStateHandle,
    }

    impl Entity for ClickView {
        type Event = ();
    }

    impl TuiView for ClickView {
        type RenderOutput = Box<dyn crate::elements::TuiElement>;

        fn ui_name() -> &'static str {
            "ClickView"
        }

        fn render_tui(&self, _: &AppContext) -> Box<dyn crate::elements::TuiElement> {
            let model = self.model.clone();
            Box::new(
                TuiMouseArea::new(self.mouse_state.clone(), |state| {
                    let label = if state.is_clicked() {
                        "pressed"
                    } else {
                        "click me"
                    };
                    Box::new(TuiText::new(label))
                })
                .on_click(move |ctx, _, _| {
                    let model = model.clone();
                    ctx.dispatch_app_update(move |app| {
                        model.update(app, |model, _| {
                            model.click_count += 1;
                        });
                    });
                }),
            )
        }
    }

    #[derive(Debug)]
    struct PointerAction;

    struct PointerParentView {
        child: TuiViewHandle<PointerChildView>,
        handled: usize,
    }

    impl Entity for PointerParentView {
        type Event = ();
    }

    impl TuiView for PointerParentView {
        type RenderOutput = Box<dyn crate::elements::TuiElement>;

        fn ui_name() -> &'static str {
            "PointerParentView"
        }

        fn render_tui(&self, app: &AppContext) -> Self::RenderOutput {
            Box::new(TuiChildView::new(&self.child, app))
        }
    }

    impl TuiTypedActionView for PointerParentView {
        type Action = PointerAction;

        fn handle_action(&mut self, _: &Self::Action, _: &mut TuiViewContext<Self>) {
            self.handled += 1;
        }
    }

    struct PointerChildView {
        mouse_state: TuiMouseStateHandle,
        handled: usize,
    }

    impl Entity for PointerChildView {
        type Event = ();
    }

    impl TuiView for PointerChildView {
        type RenderOutput = Box<dyn crate::elements::TuiElement>;

        fn ui_name() -> &'static str {
            "PointerChildView"
        }

        fn render_tui(&self, _: &AppContext) -> Self::RenderOutput {
            Box::new(
                TuiMouseArea::new(self.mouse_state.clone(), |_| Box::new(TuiText::new("child")))
                    .on_click(|ctx, _, _| ctx.dispatch_typed_action(PointerAction)),
            )
        }
    }

    impl TuiTypedActionView for PointerChildView {
        type Action = PointerAction;

        fn handle_action(&mut self, _: &Self::Action, _: &mut TuiViewContext<Self>) {
            self.handled += 1;
        }
    }

    #[derive(Debug)]
    struct KeyAction;

    struct RawKeyModel {
        count: usize,
    }

    impl Entity for RawKeyModel {
        type Event = ();
    }

    struct KeyParentView {
        child: TuiViewHandle<KeyChildView>,
        child_visible: bool,
    }

    impl Entity for KeyParentView {
        type Event = ();
    }

    impl TuiView for KeyParentView {
        type RenderOutput = Box<dyn crate::elements::TuiElement>;

        fn ui_name() -> &'static str {
            "KeyParentView"
        }

        fn render_tui(&self, app: &AppContext) -> Self::RenderOutput {
            if self.child_visible {
                Box::new(TuiChildView::new(&self.child, app))
            } else {
                Box::new(TuiText::new("hidden"))
            }
        }
    }

    struct KeyChildView {
        raw_keys: ModelHandle<RawKeyModel>,
        actions: usize,
    }

    impl Entity for KeyChildView {
        type Event = ();
    }

    impl TuiView for KeyChildView {
        type RenderOutput = Box<dyn crate::elements::TuiElement>;

        fn ui_name() -> &'static str {
            "KeyChildView"
        }

        fn render_tui(&self, _: &AppContext) -> Self::RenderOutput {
            let raw_keys = self.raw_keys.clone();
            Box::new(TuiEventHandler::new(TuiText::new("keys")).on_key_down(
                move |ctx, _, _| {
                    let raw_keys = raw_keys.clone();
                    ctx.dispatch_app_update(move |app| {
                        raw_keys.update(app, |model, _| model.count += 1);
                    });
                    TuiDispatchEventResult::StopPropagation
                },
            ))
        }
    }

    impl TuiTypedActionView for KeyChildView {
        type Action = KeyAction;

        fn handle_action(&mut self, _: &Self::Action, _: &mut TuiViewContext<Self>) {
            self.actions += 1;
        }
    }

    fn key_down(key: &str) -> Event {
        Event::KeyDown {
            keystroke: Keystroke {
                key: key.to_owned(),
                ..Default::default()
            },
            chars: key.to_owned(),
            details: KeyEventDetails::default(),
            is_composing: false,
        }
    }
    #[test]
    fn renders_view_from_shared_model_state() {
        App::test((), |mut app| async move {
            let model = app.add_model(|_| GreetingModel {
                greeting: "hello tui".to_string(),
            });
            let (_, view) = app.add_tui_window(|_| GreetingView { model });

            let mut presenter = TuiPresenter::new();
            let frame = app.read(|ctx| {
                view.read(ctx, |view, ctx| {
                    presenter.render_view(view, ctx, TuiSize::new(12, 3))
                })
            });

            assert_eq!(
                frame.buffer.lines(),
                vec![
                    "┌──────────┐".to_string(),
                    "│hello tui │".to_string(),
                    "└──────────┘".to_string(),
                ]
            );
            assert_eq!(presenter.frame_count(), 1);
        });
    }

    #[test]
    fn dispatches_events_through_rendered_element_tree() {
        App::test((), |mut app| async move {
            let model = app.add_model(|_| GreetingModel {
                greeting: "hello tui".to_string(),
            });
            let (_, view) = app.add_tui_window(|_| EventHandlingView {
                model: model.clone(),
            });

            let mut presenter = TuiPresenter::new();
            app.read(|ctx| {
                view.read(ctx, |view, ctx| {
                    presenter.render_view(view, ctx, TuiSize::new(12, 3));
                })
            });

            let event = Event::KeyDown {
                keystroke: Keystroke {
                    key: "enter".to_owned(),
                    ..Default::default()
                },
                chars: String::new(),
                details: KeyEventDetails::default(),
                is_composing: false,
            };

            let result = presenter.dispatch_event(&event, &mut app);

            assert!(result.handled);
            assert_eq!(
                model.read(&app, |model, _| model.greeting.clone()),
                "event handled"
            );
        });
    }

    #[test]
    fn dispatches_clicks_to_mouse_area() {
        App::test((), |mut app| async move {
            let model = app.add_model(|_| ClickModel { click_count: 0 });
            let mouse_state = TuiMouseStateHandle::default();
            let (_, view) = app.add_tui_window(|_| ClickView {
                model: model.clone(),
                mouse_state,
            });

            let mut presenter = TuiPresenter::new();
            app.read(|ctx| {
                view.read(ctx, |view, ctx| {
                    presenter.render_view(view, ctx, TuiSize::new(8, 1));
                })
            });

            let down = Event::LeftMouseDown {
                position: vec2f(2.0, 0.0),
                modifiers: Default::default(),
                click_count: 1,
                is_first_mouse: false,
            };
            let up = Event::LeftMouseUp {
                position: vec2f(2.0, 0.0),
                modifiers: Default::default(),
            };

            assert!(presenter.dispatch_event(&down, &mut app).handled);
            assert_eq!(model.read(&app, |model, _| model.click_count), 0);
            assert!(presenter.dispatch_event(&up, &mut app).handled);
            assert_eq!(model.read(&app, |model, _| model.click_count), 1);
        });
    }

    #[test]
    fn pointer_actions_use_the_visible_child_responder_chain() {
        App::test((), |mut app| async move {
            let (window_id, root) = app.add_tui_typed_action_window(|ctx| {
                let child = ctx.add_tui_typed_action_view(|_| PointerChildView {
                    mouse_state: TuiMouseStateHandle::default(),
                    handled: 0,
                });
                PointerParentView { child, handled: 0 }
            });
            let child = root.read(&app, |root, _| root.child.clone());
            let root_id = root.id();

            let mut presenter = TuiPresenter::new();
            app.read(|ctx| {
                root.read(ctx, |root, ctx| {
                    presenter.render_window(window_id, root_id, root, ctx, TuiSize::new(8, 1));
                })
            });

            assert_eq!(presenter.responder_chain(child.id()), vec![root.id(), child.id()]);
            assert!(presenter
                .dispatch_event(
                    &Event::LeftMouseDown {
                        position: vec2f(2.0, 0.0),
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    &mut app,
                )
                .handled);
            assert!(presenter
                .dispatch_event(
                    &Event::LeftMouseUp {
                        position: vec2f(2.0, 0.0),
                        modifiers: Default::default(),
                    },
                    &mut app,
                )
                .handled);
            assert_eq!(child.read(&app, |child, _| child.handled), 1);
            assert_eq!(root.read(&app, |root, _| root.handled), 0);
        });
    }

    #[test]
    fn focused_keybindings_suppress_raw_keys_and_hidden_focus_falls_back_to_root() {
        App::test((), |mut app| async move {
            let raw_keys = app.add_model(|_| RawKeyModel { count: 0 });
            let (window_id, root) = app.add_tui_window(|ctx| {
                let child = ctx.add_tui_typed_action_view(|_| KeyChildView {
                    raw_keys: raw_keys.clone(),
                    actions: 0,
                });
                KeyParentView {
                    child,
                    child_visible: true,
                }
            });
            let child = root.read(&app, |root, _| root.child.clone());
            let root_id = root.id();
            app.update(|ctx| {
                ctx.register_fixed_bindings([FixedBinding::new(
                    "enter a",
                    KeyAction,
                    ContextPredicate::Identifier("KeyChildView"),
                )]);
            });
            app.focus_tui_view(window_id, child.id());

            let mut presenter = TuiPresenter::new();
            app.read(|ctx| {
                root.read(ctx, |root, ctx| {
                    presenter.render_window(window_id, root_id, root, ctx, TuiSize::new(8, 1));
                })
            });

            assert!(presenter.dispatch_event(&key_down("enter"), &mut app).handled);
            assert_eq!(raw_keys.read(&app, |model, _| model.count), 0);
            assert_eq!(child.read(&app, |child, _| child.actions), 0);
            assert!(presenter.dispatch_event(&key_down("a"), &mut app).handled);
            assert_eq!(raw_keys.read(&app, |model, _| model.count), 0);
            assert_eq!(child.read(&app, |child, _| child.actions), 1);

            root.update(&mut app, |root, _| root.child_visible = false);
            app.read(|ctx| {
                root.read(ctx, |root, ctx| {
                    presenter.render_window(window_id, root_id, root, ctx, TuiSize::new(8, 1));
                })
            });
            presenter.sync_focus(&mut app);
            assert_eq!(
                app.read(|ctx| ctx.focused_tui_view_id(window_id)),
                Some(root_id)
            );
        });
    }
}
