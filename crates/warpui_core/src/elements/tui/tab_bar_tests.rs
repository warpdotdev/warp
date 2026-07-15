use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use ratatui::style::{Color, Modifier};

use super::{
    page_layout, tab_bar_layout, text_width, truncate_with_ellipsis, TuiTab, TuiTabBar,
    TuiTabBarConfig, TuiTabBarEvent, TuiTabBarNavigationDirection, TuiTabBarStyles, TuiTabBarText,
};
use crate::elements::tui::test_support::dispatch_presented_event;
use crate::elements::tui::{TuiBufferExt, TuiEvent, TuiPoint, TuiRect, TuiStyle};
use crate::event::ModifiersState;
use crate::presenter::tui::TuiPresenter;
use crate::App;

type Events = Rc<RefCell<Vec<TuiTabBarEvent<u8>>>>;

fn tab(key: u8, label: &str) -> TuiTab<u8> {
    TuiTab::new(key, label)
}

fn config(tabs: Vec<TuiTab<u8>>, events: &Events) -> TuiTabBarConfig<u8> {
    let events = events.clone();
    let mut config = TuiTabBarConfig::new(tabs, move |event, _, _| {
        events.borrow_mut().push(event);
    });
    config.styles = TuiTabBarStyles {
        bar: TuiStyle::default().bg(Color::Black),
        tab: TuiStyle::default().fg(Color::White),
        selected_focused: TuiStyle::default()
            .fg(Color::Black)
            .bg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        selected_unfocused: TuiStyle::default().add_modifier(Modifier::BOLD),
    };
    config
}

fn left_mouse_down(x: u16) -> TuiEvent {
    TuiEvent::LeftMouseDown {
        position: TuiPoint::new(x, 0),
        modifiers: ModifiersState::default(),
        click_count: 1,
        is_first_mouse: false,
    }
}

fn left_mouse_up(x: u16) -> TuiEvent {
    TuiEvent::LeftMouseUp {
        position: TuiPoint::new(x, 0),
        modifiers: ModifiersState::default(),
    }
}

#[test]
fn truncates_by_display_columns_without_splitting_graphemes() {
    assert_eq!(truncate_with_ellipsis("infrastructure", 8), "infra...");
    assert_eq!(truncate_with_ellipsis("abcdef", 2), "..");
    assert_eq!(truncate_with_ellipsis("界界界界", 7), "界界...");
    assert_eq!(truncate_with_ellipsis("e\u{301}clair", 5), "e\u{301}c...");
    assert_eq!(text_width("界界..."), 7);
}

#[test]
fn main_tab_is_fixed_while_secondary_tabs_page() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut config = config(
        vec![tab(2, "alpha"), tab(3, "beta"), tab(4, "gamma")],
        &events,
    );
    config.leading = Some(TuiTabBarText::new("Agents:", TuiStyle::default()));
    config.main_tab = Some(tab(1, "main"));
    config.divider = Some(TuiTabBarText::new("|", TuiStyle::default()));
    config.page_anchor = Some(3);

    let layout = tab_bar_layout(&config, 24);
    assert_eq!(layout.order, vec![1, 2, 3, 4]);
    assert!(layout
        .pieces
        .iter()
        .any(|piece| matches!(piece, super::LayoutPiece::Tab(tab) if tab.tab.key == 1)));
    assert!(!layout.visible_secondary_keys.contains(&2));
    assert!(layout.visible_secondary_keys.contains(&3));
}

#[test]
fn page_layout_reserves_overflow_and_truncates_the_last_visible_tab() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut config = config(
        vec![
            tab(1, "alpha"),
            tab(2, "bravo"),
            tab(3, "charlie-long"),
            tab(4, "delta"),
        ],
        &events,
    );
    config.maximum_label_columns = Some(20);
    let page = page_layout(&config, 0, 22);

    assert_eq!(page.start, 0);
    assert!(page.next_anchor.is_some());
    assert!(!page.tabs.is_empty());
    assert_ne!(page.tabs.last().unwrap().label, "charlie-long");
    let tabs_width: u16 = page.tabs.iter().map(|tab| tab.width).sum();
    assert!(tabs_width <= 22);
}

#[test]
fn invalid_page_anchor_clamps_to_first_page() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut config = config(vec![tab(1, "one"), tab(2, "two")], &events);
    config.page_anchor = Some(99);

    let layout = tab_bar_layout(&config, 40);
    assert_eq!(layout.visible_secondary_keys, vec![1, 2]);
}

#[test]
fn selected_tab_reveal_preserves_page_until_selection_crosses_boundary() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut config = config(
        vec![
            tab(1, "alpha"),
            tab(2, "bravo"),
            tab(3, "charlie"),
            tab(4, "delta"),
        ],
        &events,
    );
    config.reveal_selected = true;
    config.selected_key = Some(1);
    let first_page = tab_bar_layout(&config, 20);
    assert!(first_page.visible_secondary_keys.len() >= 2);

    config.selected_key = Some(2);
    let same_page = tab_bar_layout(&config, 20);
    assert_eq!(
        same_page.visible_secondary_keys, first_page.visible_secondary_keys,
        "selection within a page must not re-anchor the list"
    );

    let first_hidden_index = page_layout(&config, 0, 20).end;
    let first_hidden_key = config.tabs[first_hidden_index].key;
    config.selected_key = Some(first_hidden_key);
    let next_page = tab_bar_layout(&config, 20);
    assert_eq!(
        next_page.visible_secondary_keys.first(),
        Some(&first_hidden_key),
        "crossing the boundary reveals the stable next page"
    );
}

#[test]
fn explicit_page_can_keep_the_selected_tab_off_page() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut config = config(
        vec![
            tab(1, "alpha"),
            tab(2, "bravo"),
            tab(3, "charlie"),
            tab(4, "delta"),
        ],
        &events,
    );
    config.page_anchor = Some(3);
    config.selected_key = Some(1);
    config.reveal_selected = false;

    let layout = tab_bar_layout(&config, 20);
    assert!(!layout.visible_secondary_keys.contains(&1));
    assert_eq!(layout.visible_secondary_keys.first(), Some(&3));
}

#[test]
fn navigation_wraps_when_selected_tab_is_visible() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let events = Rc::new(RefCell::new(Vec::new()));
            let bar = TuiTabBar::new();
            let mut config = config(vec![tab(2, "two"), tab(3, "three")], &events);
            config.main_tab = Some(tab(1, "main"));
            config.selected_key = Some(1);
            let mut presenter = TuiPresenter::new();
            presenter.present_element(bar.render(config), TuiRect::new(0, 0, 40, 1), app_ctx);

            assert_eq!(
                bar.navigation_target(TuiTabBarNavigationDirection::Previous),
                Some(3)
            );
            assert_eq!(
                bar.navigation_target(TuiTabBarNavigationDirection::Next),
                Some(2)
            );
        });
    });
}

#[test]
fn focused_selection_uses_the_caller_supplied_style() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let events = Rc::new(RefCell::new(Vec::new()));
            let bar = TuiTabBar::new();
            let mut config = config(vec![tab(1, "one"), tab(2, "two")], &events);
            config.selected_key = Some(1);
            config.focused = true;
            let mut presenter = TuiPresenter::new();
            let frame =
                presenter.present_element(bar.render(config), TuiRect::new(0, 0, 20, 1), app_ctx);

            let selected = &frame.buffer[(1, 0)];
            assert_eq!(selected.bg, Color::Magenta);
            assert!(selected.modifier.contains(Modifier::BOLD));
        });
    });
}

#[test]
fn navigation_uses_visible_boundaries_when_selection_is_off_page() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let events = Rc::new(RefCell::new(Vec::new()));
            let bar = TuiTabBar::new();
            let mut config = config(
                vec![
                    tab(2, "alpha"),
                    tab(3, "bravo"),
                    tab(4, "charlie"),
                    tab(5, "delta"),
                ],
                &events,
            );
            config.main_tab = Some(tab(1, "main"));
            config.selected_key = Some(2);
            config.page_anchor = Some(4);
            let mut presenter = TuiPresenter::new();
            presenter.present_element(bar.render(config), TuiRect::new(0, 0, 24, 1), app_ctx);
            let state = bar.state.borrow();
            let visible = &state
                .settled_layout
                .as_ref()
                .unwrap()
                .visible_secondary_keys;
            assert!(!visible.contains(&2));
            let first = visible.first().copied();
            let last = visible.last().copied();
            drop(state);

            assert_eq!(
                bar.navigation_target(TuiTabBarNavigationDirection::Previous),
                last
            );
            assert_eq!(
                bar.navigation_target(TuiTabBarNavigationDirection::Next),
                first
            );
        });
    });
}

#[test]
fn clicking_tab_emits_only_select_event() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let events = Rc::new(RefCell::new(Vec::new()));
            let bar = TuiTabBar::new();
            let config = config(vec![tab(1, "one"), tab(2, "two")], &events);
            let mut presenter = TuiPresenter::new();
            let frame =
                presenter.present_element(bar.render(config), TuiRect::new(0, 0, 20, 1), app_ctx);
            assert!(frame.buffer.to_lines()[0].contains("one"));

            assert!(dispatch_presented_event(&mut presenter, &left_mouse_down(2), app_ctx).0);
            assert!(dispatch_presented_event(&mut presenter, &left_mouse_up(2), app_ctx).0);
            assert_eq!(*events.borrow(), vec![TuiTabBarEvent::SelectTab(1)]);
        });
    });
}

#[test]
fn clicking_overflow_emits_page_change_without_selection() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let events = Rc::new(RefCell::new(Vec::new()));
            let bar = TuiTabBar::new();
            let config = config(
                vec![tab(1, "alpha"), tab(2, "bravo"), tab(3, "charlie")],
                &events,
            );
            let mut presenter = TuiPresenter::new();
            let frame =
                presenter.present_element(bar.render(config), TuiRect::new(0, 0, 12, 1), app_ctx);
            let line = &frame.buffer.to_lines()[0];
            let arrow = line.find('→').expect("next overflow is visible") as u16;

            assert!(dispatch_presented_event(&mut presenter, &left_mouse_down(arrow), app_ctx).0);
            assert!(dispatch_presented_event(&mut presenter, &left_mouse_up(arrow), app_ctx).0);
            assert!(matches!(
                events.borrow().as_slice(),
                [TuiTabBarEvent::PageChanged(_)]
            ));
        });
    });
}

#[test]
fn retained_mouse_state_is_reused_and_removed_keys_are_pruned() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let bar = TuiTabBar::new();
    let _ = bar.render(config(vec![tab(1, "one"), tab(2, "two")], &events));
    let first_handle = bar.state.borrow().mouse_states.get(&1).cloned().unwrap();

    let _ = bar.render(config(vec![tab(1, "one"), tab(3, "three")], &events));
    let state = bar.state.borrow();
    assert_eq!(state.mouse_states.len(), 2);
    assert!(!state.mouse_states.contains_key(&2));
    assert!(Arc::ptr_eq(
        &first_handle,
        state.mouse_states.get(&1).unwrap()
    ));
}
