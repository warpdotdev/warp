use std::cell::RefCell;
use std::rc::Rc;

use pathfinder_geometry::vector::{Vector2F, vec2f};
use warp_core::ui::appearance::Appearance;
use warpui::elements::new_scrollable::SingleAxisConfig;
use warpui::elements::{ChildView, Clipped, ClippedScrollStateHandle, ConstrainedBox, Fill};
use warpui::platform::WindowStyle;
use warpui::{
    App, Entity, Event, Presenter, TypedActionView, View, ViewContext, ViewHandle, WindowId,
    WindowInvalidation,
};

use super::*;

fn initialize_test_app(app: &mut App) {
    app.add_singleton_model(|_| Appearance::mock());
}

struct TestView {
    buttons: ViewHandle<NumberShortcutButtons>,
    scroll_state: ClippedScrollStateHandle,
    selected_actions: Rc<RefCell<Vec<usize>>>,
}

impl TestView {
    fn new(ctx: &mut ViewContext<Self>) -> Self {
        let scroll_state = ClippedScrollStateHandle::new();
        let selected_actions = Rc::new(RefCell::new(Vec::new()));
        let button_builders = (0..10)
            .map(|index| {
                numbered_shortcut_button(
                    index + 1,
                    format!("Option {}", index + 1),
                    false,
                    false,
                    false,
                    MouseStateHandle::default(),
                    TestAction::Selected(index),
                )
            })
            .collect();
        let buttons = ctx.add_typed_action_view({
            let scroll_state = scroll_state.clone();
            move |ctx| {
                NumberShortcutButtons::new_with_config(
                    button_builders,
                    None,
                    NumberShortcutButtonsConfig::new()
                        .with_keyboard_navigation()
                        .with_scroll_state(scroll_state),
                    ctx,
                )
            }
        });

        Self {
            buttons,
            scroll_state,
            selected_actions,
        }
    }
}

impl Entity for TestView {
    type Event = ();
}

impl View for TestView {
    fn ui_name() -> &'static str {
        "NumberShortcutButtonsTestView"
    }

    fn render(&self, _app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        let scrollable = warpui::elements::NewScrollable::vertical(
            SingleAxisConfig::Clipped {
                handle: self.scroll_state.clone(),
                child: ChildView::new(&self.buttons).finish(),
            },
            Fill::None,
            Fill::None,
            Fill::None,
        )
        .finish();

        ConstrainedBox::new(Clipped::new(scrollable).finish())
            .with_height(96.)
            .finish()
    }
}

impl TypedActionView for TestView {
    type Action = TestAction;

    fn handle_action(&mut self, action: &Self::Action, _ctx: &mut ViewContext<Self>) {
        match action {
            TestAction::Selected(index) => {
                self.selected_actions.borrow_mut().push(*index);
            }
        }
    }
}

#[derive(Clone, Debug)]
enum TestAction {
    Selected(usize),
}

fn button_position_id(buttons: &ViewHandle<NumberShortcutButtons>, index: usize) -> String {
    format!("number_shortcut_buttons_{}_{}", buttons.id(), index)
}

fn mouse_moved_event(position: Vector2F, is_synthetic: bool) -> Event {
    Event::MouseMoved {
        position,
        cmd: false,
        shift: false,
        is_synthetic,
    }
}

fn visible_unselected_button_center(
    app: &App,
    window_id: WindowId,
    buttons: &ViewHandle<NumberShortcutButtons>,
    selected_index: Option<usize>,
) -> (usize, Vector2F) {
    (0..10)
        .filter(|index| Some(*index) != selected_index)
        .find_map(|index| {
            let position_id = button_position_id(buttons, index);
            let position =
                app.read(|ctx| ctx.element_position_by_id_at_last_frame(window_id, &position_id))?;
            (position.max_y() > 0. && position.min_y() < 96.).then_some((index, position.center()))
        })
        .expect("expected a visible unselected option")
}

#[test]
fn first_arrow_down_selects_the_first_button() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, TestView::new);
        let buttons = view.read(&app, |view, _| view.buttons.clone());

        buttons.read(&app, |buttons, _| {
            assert_eq!(buttons.selected_button_index(), None);
        });

        buttons.update(&mut app, |buttons, ctx| {
            buttons.handle_action(&NumberShortcutButtonsAction::ArrowDown, ctx);
        });

        buttons.read(&app, |buttons, _| {
            assert_eq!(buttons.selected_button_index(), Some(0));
        });
    });
}

#[test]
fn first_arrow_up_selects_the_first_button() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, TestView::new);
        let buttons = view.read(&app, |view, _| view.buttons.clone());

        buttons.update(&mut app, |buttons, ctx| {
            buttons.handle_action(&NumberShortcutButtonsAction::ArrowUp, ctx);
        });

        buttons.read(&app, |buttons, _| {
            assert_eq!(buttons.selected_button_index(), Some(0));
        });
    });
}

#[test]
fn number_shortcut_activates_without_leaving_the_option_selected() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, TestView::new);
        let buttons = view.read(&app, |view, _| view.buttons.clone());

        buttons.update(&mut app, |buttons, ctx| {
            buttons.handle_action(&NumberShortcutButtonsAction::ArrowDown, ctx);
        });
        buttons.read(&app, |buttons, _| {
            assert_eq!(buttons.selected_button_index(), Some(0));
        });

        buttons.update(&mut app, |buttons, ctx| {
            buttons.handle_action(&NumberShortcutButtonsAction::NumberSelect(4), ctx);
        });

        buttons.read(&app, |buttons, _| {
            assert_eq!(buttons.selected_button_index(), None);
        });
        view.read(&app, |view, _| {
            assert_eq!(*view.selected_actions.borrow(), vec![4]);
        });
    });
}

#[test]
fn number_shortcut_scrolls_activated_button_into_view() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, TestView::new);
        let buttons = view.read(&app, |view, _| view.buttons.clone());
        let scroll_state = view.read(&app, |view, _| view.scroll_state.clone());
        let root_view_id = app
            .root_view_id(window_id)
            .expect("window should have a root view");

        let mut presenter = Presenter::new(window_id);
        let invalidation = WindowInvalidation {
            updated: [root_view_id, buttons.id()].into_iter().collect(),
            ..Default::default()
        };

        app.update(|ctx| {
            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(vec2f(320., 240.), 1., None, ctx);

            buttons.update(ctx, |buttons, ctx| {
                buttons.handle_action(&NumberShortcutButtonsAction::NumberSelect(6), ctx);
            });

            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(320., 240.), 1., None, ctx);

            assert!(
                scroll_state.scroll_start().as_f32() > 0.,
                "expected the activated option to be scrolled into view",
            );
        });

        buttons.read(&app, |buttons, _| {
            assert_eq!(buttons.selected_button_index(), None);
        });
        view.read(&app, |view, _| {
            assert_eq!(*view.selected_actions.borrow(), vec![6]);
        });
    });
}

#[test]
fn arrow_navigation_scrolls_selected_button_into_view() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, TestView::new);
        let buttons = view.read(&app, |view, _| view.buttons.clone());
        let scroll_state = view.read(&app, |view, _| view.scroll_state.clone());
        let root_view_id = app
            .root_view_id(window_id)
            .expect("window should have a root view");

        let mut presenter = Presenter::new(window_id);
        let invalidation = WindowInvalidation {
            updated: [root_view_id, buttons.id()].into_iter().collect(),
            ..Default::default()
        };

        app.update(|ctx| {
            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(vec2f(320., 240.), 1., None, ctx);

            buttons.update(ctx, |buttons, ctx| {
                for _ in 0..6 {
                    buttons.handle_action(&NumberShortcutButtonsAction::ArrowDown, ctx);
                }
            });

            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(320., 240.), 1., None, ctx);

            assert!(
                scroll_state.scroll_start().as_f32() > 0.,
                "expected the selected option to be scrolled into view",
            );
        });
    });
}

#[test]
fn synthetic_mouse_move_does_not_override_keyboard_selection() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, TestView::new);
        let buttons = view.read(&app, |view, _| view.buttons.clone());
        let root_view_id = app
            .root_view_id(window_id)
            .expect("window should have a root view");

        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        let invalidation = WindowInvalidation {
            updated: [root_view_id, buttons.id()].into_iter().collect(),
            ..Default::default()
        };

        app.update({
            let buttons = buttons.clone();
            let presenter = presenter.clone();
            let invalidation = invalidation.clone();
            move |ctx| {
                presenter.borrow_mut().invalidate(invalidation.clone(), ctx);
                presenter
                    .borrow_mut()
                    .build_scene(vec2f(320., 240.), 1., None, ctx);

                buttons.update(ctx, |buttons, ctx| {
                    for _ in 0..6 {
                        buttons.handle_action(&NumberShortcutButtonsAction::ArrowDown, ctx);
                    }
                });

                presenter.borrow_mut().invalidate(invalidation, ctx);
                presenter
                    .borrow_mut()
                    .build_scene(vec2f(320., 240.), 1., None, ctx);
            }
        });

        let selected_index = buttons.read(&app, |buttons, _| buttons.selected_button_index());
        let (_hovered_index, hovered_position) =
            visible_unselected_button_center(&app, window_id, &buttons, selected_index);

        app.update({
            let presenter = presenter.clone();
            move |ctx| {
                ctx.simulate_window_event(
                    mouse_moved_event(hovered_position, true),
                    window_id,
                    presenter,
                );
            }
        });
        app.update({
            let presenter = presenter.clone();
            let invalidation = invalidation.clone();
            move |ctx| {
                presenter.borrow_mut().invalidate(invalidation, ctx);
                presenter
                    .borrow_mut()
                    .build_scene(vec2f(320., 240.), 1., None, ctx);
            }
        });

        buttons.read(&app, |buttons, _| {
            assert_eq!(selected_index, buttons.selected_button_index());
        });
    });
}

#[test]
fn hover_action_takes_over_after_keyboard_navigation() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, TestView::new);
        let buttons = view.read(&app, |view, _| view.buttons.clone());
        let root_view_id = app
            .root_view_id(window_id)
            .expect("window should have a root view");

        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        let invalidation = WindowInvalidation {
            updated: [root_view_id, buttons.id()].into_iter().collect(),
            ..Default::default()
        };

        app.update({
            let buttons = buttons.clone();
            let presenter = presenter.clone();
            let invalidation = invalidation.clone();
            move |ctx| {
                presenter.borrow_mut().invalidate(invalidation.clone(), ctx);
                presenter
                    .borrow_mut()
                    .build_scene(vec2f(320., 240.), 1., None, ctx);

                buttons.update(ctx, |buttons, ctx| {
                    for _ in 0..6 {
                        buttons.handle_action(&NumberShortcutButtonsAction::ArrowDown, ctx);
                    }
                });

                presenter.borrow_mut().invalidate(invalidation, ctx);
                presenter
                    .borrow_mut()
                    .build_scene(vec2f(320., 240.), 1., None, ctx);
            }
        });

        let selected_index = buttons.read(&app, |buttons, _| buttons.selected_button_index());
        let (hovered_index, _hovered_position) =
            visible_unselected_button_center(&app, window_id, &buttons, selected_index);

        buttons.update(&mut app, |buttons, ctx| {
            buttons.handle_action(&NumberShortcutButtonsAction::RowHovered(hovered_index), ctx);
        });

        buttons.read(&app, |buttons, _| {
            assert_eq!(buttons.selected_button_index(), Some(hovered_index));
        });
    });
}

#[test]
fn hovered_out_clears_selection() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, TestView::new);
        let buttons = view.read(&app, |view, _| view.buttons.clone());

        buttons.update(&mut app, |buttons, ctx| {
            buttons.handle_action(&NumberShortcutButtonsAction::RowHovered(3), ctx);
        });
        buttons.read(&app, |buttons, _| {
            assert_eq!(buttons.selected_button_index(), Some(3));
        });

        buttons.update(&mut app, |buttons, ctx| {
            buttons.handle_action(&NumberShortcutButtonsAction::ListUnhovered, ctx);
        });

        buttons.read(&app, |buttons, _| {
            assert_eq!(buttons.selected_button_index(), None);
        });

        buttons.update(&mut app, |buttons, ctx| {
            buttons.handle_action(&NumberShortcutButtonsAction::RowHovered(1), ctx);
        });
        buttons.read(&app, |buttons, _| {
            assert_eq!(buttons.selected_button_index(), Some(1));
        });
    });
}
