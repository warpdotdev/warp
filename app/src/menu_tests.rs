use warp_core::ui::appearance::Appearance;
use warpui::elements::{ParentElement, Stack};
use warpui::platform::WindowStyle;
use warpui::presenter::ChildView;
use warpui::{App, AppContext, Element, Entity, Event, TypedActionView, View, ViewHandle};

use super::{Menu, MenuAction, MenuItem, MenuItemFields, SelectAction, SubMenu};

#[derive(Clone, Debug, PartialEq, Eq)]
enum TestAction {
    Root,
    ChildOne,
    ChildTwo,
}

fn test_submenu_items() -> Vec<MenuItem<TestAction>> {
    vec![
        MenuItem::Submenu {
            fields: MenuItemFields::new_submenu("submenu"),
            menu: SubMenu::new(vec![
                MenuItemFields::new("child one")
                    .with_on_select_action(TestAction::ChildOne)
                    .into_item(),
                MenuItemFields::new("child two")
                    .with_on_select_action(TestAction::ChildTwo)
                    .into_item(),
            ]),
        },
        MenuItemFields::new("root")
            .with_on_select_action(TestAction::Root)
            .into_item(),
    ]
}

#[test]
fn test_menu_item_selectable() {
    assert!(MenuItemFields::<()>::new("normal").into_item().selectable());
    assert!(
        !MenuItemFields::<()>::new("disabled")
            .with_disabled(true)
            .into_item()
            .selectable()
    );
    assert!(!MenuItem::<()>::Separator.selectable());
}

#[test]
fn test_next_and_previous_indexes() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let items = vec![
            MenuItemFields::<()>::new("item1")
                .with_disabled(true)
                .into_item(),
            MenuItemFields::<()>::new("item2").into_item(),
            MenuItemFields::<()>::new("item3")
                .with_disabled(true)
                .into_item(),
            MenuItemFields::<()>::new("item4").into_item(),
            MenuItemFields::<()>::new("item5").into_item(),
        ];

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<()>::new();
            menu.set_items(items, ctx);
            menu
        });

        menu.update(&mut app, |menu, _ctx| {
            assert!(menu.selected_item().is_none());

            menu.menu
                .select_internal(SelectAction::Index { row: 1, item: 0 });
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item2"
            );

            // Make sure we skip the disabled menu items
            menu.menu.select_internal(SelectAction::Next);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item4"
            );

            menu.menu.select_internal(SelectAction::Next);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item5"
            );

            // Make sure we go around
            menu.menu.select_internal(SelectAction::Next);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item2"
            );

            // Makre sure we go around with Prev action too
            menu.menu.select_internal(SelectAction::Previous);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item5"
            );

            menu.menu.select_internal(SelectAction::Previous);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item4"
            );

            // Makre sure we skip the disabled ones for previous as well
            menu.menu.select_internal(SelectAction::Previous);
            assert!(menu.selected_item().is_some());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "item2"
            );
        });
    })
}

#[test]
fn test_right_opens_selected_submenu_and_selects_first_child() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(test_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.set_selected_by_index(0, ctx);
            menu.handle_action(&MenuAction::OpenSubmenu, ctx);

            assert_eq!(menu.selected_index(), Some(0));
            let submenu = menu.menu.selected_submenu().unwrap();
            assert_eq!(submenu.selected_index(), Some(0));
            assert_eq!(
                submenu.selected_item().unwrap().fields().unwrap().label(),
                "child one"
            );
        });
    })
}

#[test]
fn test_up_and_down_navigate_the_active_submenu() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(test_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.set_selected_by_index(0, ctx);
            menu.handle_action(&MenuAction::OpenSubmenu, ctx);
            menu.handle_action(&MenuAction::Select(SelectAction::Next), ctx);

            let submenu = menu.menu.selected_submenu().unwrap();
            assert_eq!(submenu.selected_index(), Some(1));
            assert_eq!(
                submenu.selected_item().unwrap().fields().unwrap().label(),
                "child two"
            );

            menu.handle_action(&MenuAction::Select(SelectAction::Previous), ctx);

            let submenu = menu.menu.selected_submenu().unwrap();
            assert_eq!(submenu.selected_index(), Some(0));
            assert_eq!(
                submenu.selected_item().unwrap().fields().unwrap().label(),
                "child one"
            );
        });
    })
}

#[test]
fn test_enter_uses_the_active_submenu_selection() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(test_submenu_items(), ctx);
            menu
        });

        let mut selected_action = None;
        menu.update(&mut app, |menu, ctx| {
            menu.set_selected_by_index(0, ctx);
            menu.handle_action(&MenuAction::OpenSubmenu, ctx);
            menu.handle_action(&MenuAction::Select(SelectAction::Next), ctx);

            selected_action = menu.menu.selected_action_for_enter(ctx);
        });

        assert_eq!(selected_action, Some(TestAction::ChildTwo));
    })
}

#[test]
fn test_right_is_a_noop_for_leaf_items() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (_, menu) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut menu = Menu::<TestAction>::new();
            menu.set_items(test_submenu_items(), ctx);
            menu
        });

        menu.update(&mut app, |menu, ctx| {
            menu.set_selected_by_index(1, ctx);
            menu.handle_action(&MenuAction::OpenSubmenu, ctx);

            assert_eq!(menu.selected_index(), Some(1));
            assert!(menu.menu.selected_submenu().is_none());
            assert_eq!(
                menu.selected_item().unwrap().fields().unwrap().label(),
                "root"
            );
        });
    });
}

/// A minimal host view whose only job is to embed a [`Menu`] inside a
/// [`Stack`]. `SavePosition` (which every menu row wraps itself in) only
/// records a position while a `Stack` layer is active on the paint stack;
/// without one, the row positions this test reads back would silently no-op.
struct MenuDragTestHost {
    menu: ViewHandle<Menu<TestAction>>,
}

impl Entity for MenuDragTestHost {
    type Event = ();
}

impl View for MenuDragTestHost {
    fn ui_name() -> &'static str {
        "MenuDragTestHost"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        let mut stack = Stack::new();
        stack.add_child(ChildView::new(&self.menu).finish());
        stack.finish()
    }
}

impl TypedActionView for MenuDragTestHost {
    type Action = ();
}

/// A held left-mouse drag from one row onto another must move the row
/// highlight (`Hoverable`'s own hover state, opted into drag-tracking via
/// `with_hover_tracks_drag`) and `hovered_row_index` (which feeds a sidecar
/// preview panel) together. If they disagree, the menu visibly contradicts
/// itself: one visual driven by the drag position, the other stuck.
#[test]
fn test_drag_across_rows_keeps_highlight_and_hovered_index_in_agreement() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        let (window_id, host) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let menu = ctx.add_typed_action_view(|ctx| {
                let mut menu = Menu::<TestAction>::new().with_hover_tracks_drag();
                menu.set_items(
                    vec![
                        MenuItemFields::new("item one")
                            .with_on_select_action(TestAction::Root)
                            .into_item(),
                        MenuItemFields::new("item two")
                            .with_on_select_action(TestAction::ChildOne)
                            .into_item(),
                    ],
                    ctx,
                );
                menu
            });
            MenuDragTestHost { menu }
        });
        let menu = host.read(&app, |host, _| host.menu.clone());

        // `App::add_window` doesn't itself flush pending effects, so the
        // window's initial invalidation is still queued at this point. In
        // unit tests, `on_window_invalidated` eagerly builds the scene on the
        // window's own auto-registered presenter (see
        // `AppContext::insert_window_internal`) once a flush actually runs;
        // that's what promotes each row's `SavePosition` into the "last
        // frame" cache. Force that flush here rather than driving a
        // `Presenter` by hand.
        menu.update(&mut app, |_menu, ctx| ctx.notify());

        let (row0_center, row1_center) = app.read(|ctx| {
            let row0 = ctx
                .element_position_by_id_at_last_frame(window_id, "item one")
                .expect("expected a saved position for the first row");
            let row1 = ctx
                .element_position_by_id_at_last_frame(window_id, "item two")
                .expect("expected a saved position for the second row");
            (row0.center(), row1.center())
        });
        let presenter = app
            .presenter(window_id)
            .expect("window should have a presenter");

        app.update(move |ctx| {
            let hover_event = Event::MouseMoved {
                position: row0_center,
                cmd: false,
                shift: false,
                is_synthetic: false,
            };
            ctx.simulate_window_event(hover_event.clone(), window_id, presenter.clone());
            ctx.set_last_mouse_move_event(window_id, hover_event);

            ctx.simulate_window_event(
                Event::LeftMouseDown {
                    position: row0_center,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );

            // Drag, while still holding, from the first row onto the second.
            ctx.simulate_window_event(
                Event::LeftMouseDragged {
                    position: row1_center,
                    modifiers: Default::default(),
                },
                window_id,
                presenter,
            );
        });

        menu.read(&app, |menu, _| {
            assert_eq!(
                menu.hovered_index(),
                Some(1),
                "hovered_row_index (which feeds the sidecar) should track the drag position"
            );

            let is_row_hovered = |index: usize| match &menu.items()[index] {
                MenuItem::Item(fields) => fields.mouse_state.lock().unwrap().is_hovered(),
                other => panic!("expected a plain item, got {other:?}"),
            };
            assert!(
                !is_row_hovered(0),
                "the origin row's highlight should have moved off during the drag"
            );
            assert!(
                is_row_hovered(1),
                "the highlight should track the drag onto the newly hovered row, matching hovered_row_index"
            );
        });
    });
}
