//! GUI-bound tests of the shared application core. These depend on the GUI
//! backend: render-pass view embeddings discovered by the presenter layout
//! walk, the GUI test harness's auto-build-scene render loop, or GUI-only
//! APIs like `App::dispatch_custom_action`.

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;

use super::super::super::*;
use crate::elements::*;
use crate::keymap::macros::*;
use crate::keymap::Keystroke;

struct NestedView {
    children: Vec<ViewHandle<NestedView>>,
    name: String,
    events: Rc<RefCell<Vec<String>>>,
}

impl NestedView {
    fn new(
        name: String,
        children: Vec<ViewHandle<NestedView>>,
        events: Rc<RefCell<Vec<String>>>,
    ) -> Self {
        Self {
            name,
            events,
            children,
        }
    }

    fn set_children(&mut self, children: Vec<ViewHandle<NestedView>>, ctx: &mut ViewContext<Self>) {
        self.children = children;
        ctx.notify();
    }
}

impl Entity for NestedView {
    type Event = String;
}

impl super::super::super::View for NestedView {
    fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
        Flex::column()
            .with_children(
                self.children
                    .iter()
                    .map(|child| ChildView::new(child).finish()),
            )
            .finish()
    }

    fn ui_name() -> &'static str {
        "View"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, _ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.events
                .borrow_mut()
                .push(format!("{} self focused", self.name));
        } else {
            self.events
                .borrow_mut()
                .push(format!("{} child focused", self.name));
        }
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.events
                .borrow_mut()
                .push(format!("{} self blurred", self.name));
            ctx.emit("blurred".into());
        } else {
            self.events
                .borrow_mut()
                .push(format!("{} child blurred", self.name));
        }
    }
}

impl TypedActionView for NestedView {
    type Action = ();
}

#[test]
fn test_nested_focus() {
    App::test((), |mut app| async move {
        // Test that focusing child views call the focus callbacks of the ancestor views.
        // View heirarchy
        // View 1
        // - View 2
        // - View 3
        //  - View 4

        let events: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
        let app = &mut app;
        let (window_id, view_1) = app.add_window(WindowStyle::NotStealFocus, |_| {
            NestedView::new("View 1".to_string(), vec![], events.clone())
        });
        let view_2 = app.add_view(window_id, |_| {
            NestedView::new("View 2".to_string(), vec![], events.clone())
        });
        let view_4 = app.add_view(window_id, |_| {
            NestedView::new("View 4".to_string(), vec![], events.clone())
        });
        let view_3 = app.add_view(window_id, |_| {
            NestedView::new("View 3".to_string(), vec![view_4.clone()], events.clone())
        });

        assert_eq!(events.take(), ["View 1 self focused".to_string(),],);

        view_1.update(app, |view_1, ctx| {
            view_1.set_children(vec![view_2.clone(), view_3.clone()], ctx);
        });

        view_1.update(app, |_view, ctx| {
            ctx.focus(&view_2);
        });

        view_1.update(app, |_, ctx| {
            ctx.focus(&view_1);
        });

        assert_eq!(
            events.take(),
            [
                "View 1 self blurred".to_string(),
                "View 2 self focused".to_string(),
                "View 1 child focused".to_string(),
                "View 2 self blurred".to_string(),
                "View 1 child blurred".to_string(),
                "View 1 self focused".to_string(),
            ],
        );

        view_4.update(app, |_, ctx| {
            ctx.focus(&view_4);
        });

        assert_eq!(
            events.take(),
            [
                "View 1 self blurred".to_string(),
                "View 4 self focused".to_string(),
                "View 3 child focused".to_string(),
                "View 1 child focused".to_string(),
            ],
        );
    });
}

#[test]
fn test_dispatch_custom_action_triggers_typed_action() -> Result<()> {
    struct View {
        id: usize,
        keymap_context: keymap::Context,
        action_count: usize,
        keydown_count: Rc<RefCell<usize>>,
    }

    impl Entity for View {
        type Event = ();
    }

    impl super::super::super::View for View {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            let keydown_count = self.keydown_count.clone();
            EventHandler::new(Empty::new().finish())
                .on_keydown(move |_, _, _| {
                    *keydown_count.borrow_mut() += 1;
                    DispatchEventResult::StopPropagation
                })
                .finish()
        }

        fn ui_name() -> &'static str {
            "View"
        }

        fn keymap_context(&self, _: &AppContext) -> keymap::Context {
            self.keymap_context.clone()
        }
    }

    #[derive(Debug)]
    struct Action(String);

    impl TypedActionView for View {
        type Action = Action;

        fn handle_action(&mut self, action: &Action, _: &mut ViewContext<Self>) {
            assert_eq!(self.id, 1);
            assert_eq!(action.0, "a");
            self.action_count += 1;
        }
    }

    impl View {
        fn new(id: usize, context: &'static str) -> Self {
            let mut instance = View {
                id,
                keymap_context: Default::default(),
                action_count: 0,
                keydown_count: Rc::new(RefCell::new(0)),
            };

            instance.keymap_context.set.insert(context);
            instance
        }
    }

    App::test((), |mut app| async move {
        let (window_id, view_1) = app.add_window(WindowStyle::NotStealFocus, |_| View::new(1, "a"));
        let custom_tag = 123_isize;
        let binding = keymap::FixedBinding::custom(
            custom_tag,
            Action("a".into()),
            "test custom action",
            id!("a"),
        );
        app.update(|ctx| {
            ctx.register_default_keystroke_triggers_for_custom_actions(|_| {
                Some(Keystroke::parse("ctrl-1").expect("failed to parse keystroke"))
            });
            ctx.register_fixed_bindings(vec![binding]);
        });
        view_1.update(&mut app, |_, ctx| {
            ctx.focus_self();
        });
        app.dispatch_custom_action(custom_tag, window_id);
        assert_eq!(view_1.read(&app, |view, _| view.action_count), 1);
        assert_eq!(view_1.read(&app, |view, _| *view.keydown_count.borrow()), 0);

        app.update(|ctx| {
            ctx.disable_key_bindings(window_id);
        });
        app.dispatch_custom_action(custom_tag, window_id);
        assert_eq!(view_1.read(&app, |view, _| view.action_count), 1);
        assert_eq!(view_1.read(&app, |view, _| *view.keydown_count.borrow()), 1);

        app.update(|ctx| {
            ctx.enable_key_bindings(window_id);
        });
        app.dispatch_custom_action(custom_tag, window_id);
        assert_eq!(view_1.read(&app, |view, _| view.action_count), 2);
        assert_eq!(view_1.read(&app, |view, _| *view.keydown_count.borrow()), 1);

        Ok(())
    })
}

#[test]
fn test_ui_and_window_updates() {
    struct View {
        count: usize,
    }

    impl Entity for View {
        type Event = ();
    }

    impl super::super::super::View for View {
        fn render<'a>(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "View"
        }
    }

    impl TypedActionView for View {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| View { count: 3 });
        let view_1 = app.add_view(window_id, |_| View { count: 1 });
        let view_2 = app.add_view(window_id, |_| View { count: 2 });

        // Ensure that registering for UI updates after mutating the app still gives us all the
        // updates.

        let window_invalidations = Rc::new(RefCell::new(Vec::new()));
        let window_invalidations_ = window_invalidations.clone();
        app.on_window_invalidated(window_id, move |window_id, ctx| {
            window_invalidations_
                .borrow_mut()
                .push(ctx.take_all_invalidations_for_window(window_id))
        });

        let view_2_id = view_2.id();
        view_1.update(&mut app, |view, ctx| {
            view.count = 7;
            ctx.notify();
            drop(view_2);
        });

        let invalidation = window_invalidations.borrow_mut().drain(..).next().unwrap();
        assert_eq!(invalidation.updated.len(), 1);
        assert!(invalidation.updated.contains(&view_1.id()));
        assert_eq!(invalidation.removed.len(), 1);
        assert!(invalidation.removed.contains(&view_2_id));

        let view_3 = view_1.update(&mut app, |_, ctx| ctx.add_view(|_| View { count: 8 }));

        let invalidation = window_invalidations.borrow_mut().drain(..).next().unwrap();
        assert_eq!(invalidation.updated.len(), 1);
        assert!(invalidation.updated.contains(&view_3.id()));
        assert!(invalidation.removed.is_empty());

        let (tx, rx) = futures::channel::oneshot::channel();
        view_3.update(&mut app, move |_, ctx| {
            ctx.spawn(async { 9 }, move |me, output, ctx| {
                tx.send(()).unwrap();
                me.count = output;
                ctx.notify();
            })
        });

        rx.await.unwrap();

        let invalidation = window_invalidations.borrow_mut().drain(..).next().unwrap();
        assert_eq!(invalidation.updated.len(), 1);
        assert!(invalidation.updated.contains(&view_3.id()));
        assert!(invalidation.removed.is_empty());
    });
}

#[test]
fn test_key_bindings_for_view() {
    use keymap::FixedBinding;
    struct ViewA;
    struct ViewB;
    impl Entity for ViewA {
        type Event = ();
    }
    impl Entity for ViewB {
        type Event = ();
    }
    impl View for ViewA {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "ViewA"
        }
    }
    impl View for ViewB {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }

        fn ui_name() -> &'static str {
            "ViewB"
        }
    }

    struct Container {
        child_a: ViewHandle<ViewA>,
        child_b: ViewHandle<ViewB>,
    }
    impl Entity for Container {
        type Event = ();
    }
    impl View for Container {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            let mut stack = Stack::new();
            stack.add_child(ChildView::new(&self.child_a).finish());
            stack.add_child(ChildView::new(&self.child_b).finish());

            stack.finish()
        }
        fn ui_name() -> &'static str {
            "Container"
        }
    }
    impl Container {
        fn new(ctx: &mut ViewContext<Self>) -> Self {
            let child_a = ctx.add_view(|_| ViewA);
            let child_b = ctx.add_view(|_| ViewB);

            Self { child_a, child_b }
        }
    }

    struct Root {
        child: ViewHandle<Container>,
    }
    impl Entity for Root {
        type Event = ();
    }
    impl View for Root {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            ChildView::new(&self.child).finish()
        }
        fn ui_name() -> &'static str {
            "Root"
        }
    }

    impl TypedActionView for Root {
        type Action = ();
    }

    impl Root {
        fn new(ctx: &mut ViewContext<Self>) -> Self {
            let child = ctx.add_view(Container::new);

            Self { child }
        }
    }

    #[derive(Debug)]
    enum Action {
        A,
        EmptyRoot,
        BContainer,
        BLeaf,
        C,
        EmptyB,
    }

    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.register_fixed_bindings(vec![
                FixedBinding::new("a", Action::A, id!("Root")),
                FixedBinding::empty("description", Action::EmptyRoot, id!("Root")),
                FixedBinding::new("b", Action::BContainer, id!("Container")),
                FixedBinding::new("b", Action::BLeaf, id!("ViewA")),
                FixedBinding::new("c", Action::C, id!("ViewB")),
                FixedBinding::empty("other description", Action::EmptyB, id!("ViewB")),
            ]);
        });

        /*
            Builds a View Hierarchy that looks like (with bound keys in parentheses)

                    Root (a)
                     |
                 Container (b)
                  |     |
            (b) ViewA ViewB (c)
        */

        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, Root::new);
        let view_id_a = app.views_of_type::<ViewA>(window_id).unwrap()[0].id();
        let view_id_b = app.views_of_type::<ViewB>(window_id).unwrap()[0].id();

        // Force an update so the child view hierarchy is processed
        app.views_of_type::<Root>(window_id).unwrap()[0].update(&mut app, |_, ctx| ctx.notify());

        app.update(|ctx| {
            // Binding actions available to ViewA should be 'A', 'BLeaf', and 'EmptyRoot', since
            // the key binding on 'b' overlaps between ViewA and Container, we should only get the
            // highest precedence one ('BLeaf')
            let bindings_a = ctx
                .key_bindings_for_view(window_id, view_id_a)
                .into_iter()
                .map(|binding| format!("{:?}", binding.action))
                .collect::<Vec<_>>();

            assert_eq!(bindings_a, ["BLeaf", "EmptyRoot", "A"]);

            // Binding actions available to ViewB should be 'A', 'EmptyRoot', 'BContainer', 'C',
            // and 'EmptyB', as the only binding overlaps are empty, which aren't treated as
            // overlapping each other
            let bindings_b = ctx
                .key_bindings_for_view(window_id, view_id_b)
                .into_iter()
                .map(|binding| format!("{:?}", binding.action))
                .collect::<Vec<_>>();

            assert_eq!(bindings_b, ["EmptyB", "C", "BContainer", "EmptyRoot", "A"]);
        });
    });
}

#[test]
fn test_editable_binding_getters() {
    use keymap::EditableBinding;
    struct ViewA;
    struct ViewB;
    struct Container {
        child_a: ViewHandle<ViewA>,
        child_b: ViewHandle<ViewB>,
    }
    struct Root {
        child: ViewHandle<Container>,
    }
    impl Entity for ViewA {
        type Event = ();
    }
    impl Entity for ViewB {
        type Event = ();
    }
    impl Entity for Container {
        type Event = ();
    }
    impl Entity for Root {
        type Event = ();
    }
    impl View for ViewA {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }
        fn ui_name() -> &'static str {
            "ViewA"
        }
    }
    impl View for ViewB {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Empty::new().finish()
        }
        fn ui_name() -> &'static str {
            "ViewB"
        }
    }
    impl View for Container {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            Stack::new()
                .with_child(ChildView::new(&self.child_a).finish())
                .with_child(ChildView::new(&self.child_b).finish())
                .finish()
        }
        fn ui_name() -> &'static str {
            "Container"
        }
    }
    impl Container {
        fn new(ctx: &mut ViewContext<Self>) -> Self {
            let child_a = ctx.add_view(|_| ViewA);
            let child_b = ctx.add_view(|_| ViewB);

            Self { child_a, child_b }
        }
    }
    impl View for Root {
        fn render(&self, _: &AppContext) -> Box<dyn Element> {
            ChildView::new(&self.child).finish()
        }
        fn ui_name() -> &'static str {
            "Root"
        }
    }

    impl TypedActionView for Root {
        type Action = ();
    }

    impl Root {
        fn new(ctx: &mut ViewContext<Self>) -> Self {
            let child = ctx.add_view(Container::new);

            Self { child }
        }
    }
    #[derive(Debug)]
    struct Action(#[allow(dead_code)] String);

    App::test((), |mut app| async move {
        use crate::keymap::macros::*;
        app.update(|ctx| {
            ctx.register_editable_bindings([
                EditableBinding::new("a", "Action a", Action("a".into()))
                    .with_context_predicate(id!("Root")),
                EditableBinding::new("b-container", "Action b in Container", Action("b".into()))
                    .with_context_predicate(id!("Container")),
                EditableBinding::new("b-leaf", "Action b in Leaf", Action("b".into()))
                    .with_context_predicate(id!("ViewA")),
                EditableBinding::new("c", "Action c", Action("c".into()))
                    .with_context_predicate(id!("ViewB")),
            ]);
        });

        /*
            Builds a View Hierarchy that looks like (with bound keys in parentheses)

                    Root (a)
                     |
                 Container (b)
                  |     |
            (b) ViewA ViewB (c)
        */

        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, Root::new);
        let view_id_a = app.views_of_type::<ViewA>(window_id).unwrap()[0].id();
        let view_id_b = app.views_of_type::<ViewB>(window_id).unwrap()[0].id();

        // Force an update so the child view hierarchy is processed
        app.views_of_type::<Root>(window_id).unwrap()[0].update(&mut app, |_, ctx| ctx.notify());

        // Editable Bindings available to ViewA should be 'a', 'b-container', and 'b-leaf'
        // since those are all tied to context ViewA or its ancestors
        app.update(|ctx| {
            let actions_a = ctx
                .editable_bindings_for_view(window_id, view_id_a)
                .into_iter()
                .map(|action| action.name.to_owned())
                .collect::<Vec<_>>();
            assert_eq!(actions_a, ["b-leaf", "b-container", "a"]);

            // Editable bindings available to ViewB should be 'a', 'b-container', and 'c' since
            // those are all tied to ViewB or its ancestors
            let actions_b = ctx
                .editable_bindings_for_view(window_id, view_id_b)
                .into_iter()
                .map(|action| action.name.to_owned())
                .collect::<Vec<_>>();
            assert_eq!(actions_b, ["c", "b-container", "a"]);

            // Calling `editable_bindings` should get _all_ editable bindings
            let all_actions = ctx
                .editable_bindings()
                .map(|action| action.name.to_owned())
                .collect::<Vec<_>>();
            assert_eq!(all_actions, ["c", "b-leaf", "b-container", "a"]);
        });
    });
}
