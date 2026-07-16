use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use ratatui::style::{Color, Modifier};

use super::{
    page_layout, tab_bar_layout, TuiTab, TuiTabBar, TuiTabBarConfig, TuiTabBarEvent,
    TuiTabBarNavigationDirection, TuiTabBarStyles,
};
use crate::elements::tui::test_support::dispatch_presented_event;
use crate::elements::tui::{
    TuiBufferExt, TuiElement, TuiEvent, TuiPoint, TuiRect, TuiStyle, TuiText,
};
use crate::event::ModifiersState;
use crate::presenter::tui::TuiPresenter;
use crate::{App, AppContext};

type Events = Rc<RefCell<Vec<TuiTabBarEvent>>>;

fn key(key: u8) -> String {
    key.to_string()
}

fn tab(key: u8, label: &str) -> TuiTab {
    TuiTab::new(key.to_string(), label)
}

fn config(tabs: Vec<TuiTab>) -> TuiTabBarConfig {
    let mut config = TuiTabBarConfig::new(tabs);
    config.styles = TuiTabBarStyles {
        bar: TuiStyle::default().bg(Color::Black),
        leading: TuiStyle::default().fg(Color::White),
        chrome: TuiStyle::default().fg(Color::White),
        tab: TuiStyle::default().fg(Color::White),
        selected_focused: TuiStyle::default()
            .fg(Color::Black)
            .bg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        selected_unfocused: TuiStyle::default().add_modifier(Modifier::BOLD),
    };
    config
}

fn render(bar: &TuiTabBar, config: TuiTabBarConfig, events: &Events) -> Box<dyn TuiElement> {
    let events = events.clone();
    bar.render(config, move |event, _, _| {
        events.borrow_mut().push(event);
    })
}

fn layout(config: &TuiTabBarConfig, available_columns: u16) -> super::TabBarLayout {
    tab_bar_layout(config, 0, &vec![0; config.tabs.len()], available_columns)
}

fn page(
    config: &TuiTabBarConfig,
    requested_start: usize,
    available_columns: u16,
) -> super::PageLayout {
    page_layout(
        config,
        &vec![0; config.tabs.len()],
        requested_start,
        available_columns,
    )
}

fn mouse_moved(x: u16) -> TuiEvent {
    TuiEvent::MouseMoved {
        position: TuiPoint::new(x, 0),
        modifiers: ModifiersState::default(),
        is_synthetic: false,
    }
}

fn click(presenter: &mut TuiPresenter, x: u16, app: &AppContext) {
    let down = TuiEvent::LeftMouseDown {
        position: TuiPoint::new(x, 0),
        modifiers: ModifiersState::default(),
        click_count: 1,
        is_first_mouse: false,
    };
    let up = TuiEvent::LeftMouseUp {
        position: TuiPoint::new(x, 0),
        modifiers: ModifiersState::default(),
    };
    assert!(dispatch_presented_event(presenter, &down, app).0);
    assert!(dispatch_presented_event(presenter, &up, app).0);
}

#[test]
fn layout_keeps_main_fixed_and_clamps_missing_anchor() {
    let mut config = config(vec![tab(2, "alpha"), tab(3, "beta"), tab(4, "gamma")]);
    config.leading = Some("Agents:".to_string());
    config.main_tab = Some(tab(1, "main"));
    config.page_anchor = Some(key(3));

    let anchored = layout(&config, 24);
    assert_eq!(
        anchored.navigation.order,
        vec![key(1), key(2), key(3), key(4)]
    );
    assert_eq!(
        anchored.main_tab.as_ref().map(|tab| tab.key.clone()),
        Some(key(1))
    );
    assert!(!anchored.navigation.visible_secondary_keys.contains(&key(2)));
    assert!(anchored.navigation.visible_secondary_keys.contains(&key(3)));

    config.page_anchor = Some("missing".to_string());
    assert_eq!(
        layout(&config, 40).navigation.visible_secondary_keys,
        vec![key(2), key(3), key(4)]
    );
}

#[test]
fn page_layout_truncates_and_never_points_next_to_itself() {
    let mut config = config(vec![
        tab(1, "alpha"),
        tab(2, "bravo"),
        tab(3, "charlie-long"),
        tab(4, "delta"),
    ]);
    config.maximum_label_columns = Some(20);

    let first = page(&config, 0, 22);
    assert!(first.next_start.is_some());
    assert_ne!(first.tabs.last().unwrap().label, "charlie-long");
    assert!(first.tabs.iter().map(|tab| tab.width).sum::<u16>() <= 22);

    let narrow_last = page(&config, config.tabs.len() - 1, 2);
    assert!(narrow_last.tabs.is_empty());
    assert_eq!(narrow_last.next_start, None);
}

#[test]
fn selected_reveal_respects_explicit_pages() {
    let mut config = config(vec![
        tab(1, "alpha"),
        tab(2, "bravo"),
        tab(3, "charlie"),
        tab(4, "delta"),
    ]);
    config.page_anchor = Some(key(3));
    config.selected_key = Some(key(1));

    let explicit = layout(&config, 20);
    assert_eq!(
        explicit.navigation.visible_secondary_keys.first(),
        Some(&key(3))
    );
    assert!(!explicit.navigation.visible_secondary_keys.contains(&key(1)));

    config.reveal_selected = true;
    let revealed = layout(&config, 20);
    assert_eq!(
        revealed.navigation.visible_secondary_keys.first(),
        Some(&key(1))
    );

    config.selected_key = Some(key(2));
    assert_eq!(
        layout(&config, 20).navigation.visible_secondary_keys,
        revealed.navigation.visible_secondary_keys
    );
}

#[test]
fn navigation_wraps_and_enters_the_visible_page() {
    App::test((), |app| async move {
        app.read(|app| {
            let events = Rc::new(RefCell::new(Vec::new()));
            let bar = TuiTabBar::new();
            let mut presenter = TuiPresenter::new();

            let mut wrap_config = config(vec![tab(2, "two"), tab(3, "three")]);
            wrap_config.main_tab = Some(tab(1, "main"));
            wrap_config.selected_key = Some(key(1));
            presenter.present_element(
                render(&bar, wrap_config, &events),
                TuiRect::new(0, 0, 40, 1),
                app,
            );
            assert_eq!(
                bar.navigation_target(TuiTabBarNavigationDirection::Previous),
                Some(key(3))
            );
            assert_eq!(
                bar.navigation_target(TuiTabBarNavigationDirection::Next),
                Some(key(2))
            );

            let mut page_config = config(vec![
                tab(1, "alpha"),
                tab(2, "bravo"),
                tab(3, "charlie"),
                tab(4, "delta"),
            ]);
            page_config.selected_key = Some(key(1));
            page_config.page_anchor = Some(key(3));
            presenter.present_element(
                render(&bar, page_config, &events),
                TuiRect::new(0, 0, 16, 1),
                app,
            );
            let visible = bar
                .state
                .borrow()
                .settled_navigation
                .as_ref()
                .unwrap()
                .visible_secondary_keys
                .clone();
            assert!(!visible.contains(&key(1)));
            assert_eq!(
                bar.navigation_target(TuiTabBarNavigationDirection::Previous),
                visible.last().cloned()
            );
            assert_eq!(
                bar.navigation_target(TuiTabBarNavigationDirection::Next),
                visible.first().cloned()
            );
        });
    });
}

#[test]
fn renders_selected_style_and_leading_element_once() {
    App::test((), |app| async move {
        app.read(|app| {
            let events = Rc::new(RefCell::new(Vec::new()));
            let bar = TuiTabBar::new();
            let build_count = Rc::new(Cell::new(0));
            let leading_build_count = build_count.clone();
            let mut config = config(vec![tab(1, "one").with_leading_element(move || {
                leading_build_count.set(leading_build_count.get() + 1);
                TuiText::new("*")
                    .with_style(TuiStyle::default().fg(Color::Yellow))
                    .finish()
            })]);
            config.selected_key = Some(key(1));
            config.focused = true;

            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render(&bar, config, &events),
                TuiRect::new(0, 0, 20, 1),
                app,
            );
            assert!(frame.buffer.to_lines()[0].contains("* one"));
            assert_eq!(frame.buffer[(0, 0)].bg, Color::Magenta);
            assert!(frame.buffer[(0, 0)].modifier.contains(Modifier::BOLD));
            assert_eq!(frame.buffer[(1, 0)].fg, Color::Yellow);
            assert_eq!(build_count.get(), 1);
        });
    });
}

#[test]
fn hover_bolds_tabs_and_overflow_arrows() {
    App::test((), |app| async move {
        app.read(|app| {
            let events = Rc::new(RefCell::new(Vec::new()));
            let mut presenter = TuiPresenter::new();

            let tab_bar = TuiTabBar::new();
            let frame = presenter.present_element(
                render(&tab_bar, config(vec![tab(1, "one")]), &events),
                TuiRect::new(0, 0, 20, 1),
                app,
            );
            assert!(!frame.buffer[(1, 0)].modifier.contains(Modifier::BOLD));
            dispatch_presented_event(&mut presenter, &mouse_moved(1), app);
            let frame = presenter.present_element(
                render(&tab_bar, config(vec![tab(1, "one")]), &events),
                TuiRect::new(0, 0, 20, 1),
                app,
            );
            assert!(frame.buffer[(1, 0)].modifier.contains(Modifier::BOLD));

            let overflow_bar = TuiTabBar::new();
            let tabs = || config(vec![tab(1, "alpha"), tab(2, "bravo"), tab(3, "charlie")]);
            let frame = presenter.present_element(
                render(&overflow_bar, tabs(), &events),
                TuiRect::new(0, 0, 12, 1),
                app,
            );
            let arrow = frame.buffer.to_lines()[0]
                .find('→')
                .expect("next overflow is visible") as u16;
            assert!(!frame.buffer[(arrow, 0)].modifier.contains(Modifier::BOLD));
            dispatch_presented_event(&mut presenter, &mouse_moved(arrow), app);
            let frame = presenter.present_element(
                render(&overflow_bar, tabs(), &events),
                TuiRect::new(0, 0, 12, 1),
                app,
            );
            assert!(frame.buffer[(arrow, 0)].modifier.contains(Modifier::BOLD));
        });
    });
}

#[test]
fn clicks_emit_semantic_events() {
    App::test((), |app| async move {
        app.read(|app| {
            let events = Rc::new(RefCell::new(Vec::new()));
            let bar = TuiTabBar::new();
            let mut presenter = TuiPresenter::new();

            presenter.present_element(
                render(&bar, config(vec![tab(1, "one"), tab(2, "two")]), &events),
                TuiRect::new(0, 0, 20, 1),
                app,
            );
            click(&mut presenter, 2, app);
            assert_eq!(*events.borrow(), vec![TuiTabBarEvent::SelectTab(key(1))]);

            events.borrow_mut().clear();
            let frame = presenter.present_element(
                render(
                    &bar,
                    config(vec![tab(1, "alpha"), tab(2, "bravo"), tab(3, "charlie")]),
                    &events,
                ),
                TuiRect::new(0, 0, 12, 1),
                app,
            );
            let arrow = frame.buffer.to_lines()[0]
                .find('→')
                .expect("next overflow is visible") as u16;
            click(&mut presenter, arrow, app);
            assert!(matches!(
                events.borrow().as_slice(),
                [TuiTabBarEvent::PageChanged(_)]
            ));
        });
    });
}

#[test]
fn retained_mouse_state_is_reused_and_pruned() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let bar = TuiTabBar::new();
    let _ = render(&bar, config(vec![tab(1, "one"), tab(2, "two")]), &events);
    let first_handle = bar
        .state
        .borrow()
        .mouse_states
        .get(&key(1))
        .cloned()
        .unwrap();

    let _ = render(&bar, config(vec![tab(1, "one"), tab(3, "three")]), &events);
    let state = bar.state.borrow();
    assert_eq!(state.mouse_states.len(), 2);
    assert!(!state.mouse_states.contains_key(&key(2)));
    assert!(Arc::ptr_eq(
        &first_handle,
        state.mouse_states.get(&key(1)).unwrap()
    ));
}
