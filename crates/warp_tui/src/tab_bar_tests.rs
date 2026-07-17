use std::sync::Arc;

use warpui_core::elements::tui::{Color, Modifier, TuiBufferExt, TuiRect, TuiStyle};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, TuiView};

use super::{
    deterministic_pages_at_width, minimum_row_width, page_variant_at_width, TuiTab,
    TuiTabBarConfig, TuiTabBarNavigationDirection, TuiTabBarSecondaryEdge, TuiTabBarStyles,
    TuiTabBarView,
};

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

fn render(
    view: &TuiTabBarView,
    width: u16,
    app: &AppContext,
) -> warpui_core::presenter::tui::TuiFrame {
    TuiPresenter::new().present_element(view.render(app), TuiRect::new(0, 0, width, 1), app)
}

#[test]
fn orchestration_layout_stays_on_the_first_page_when_all_tabs_fit() {
    App::test((), |app| async move {
        app.read(|app| {
            let mut config = config(vec![
                tab(1, "haiku-2").with_leading_text("*", TuiStyle::default()),
                tab(2, "haiku-1").with_leading_text("⊛", TuiStyle::default()),
                tab(3, "haiku-3").with_leading_text("✠", TuiStyle::default()),
            ]);
            config.leading = Some("Agents:".to_owned());
            config.main_tab = Some(TuiTab::new("root", "orchestrator"));
            config.page_anchor = Some(key(1));
            config.selected_key = Some(key(2));
            config.reveal_selected = true;
            config.maximum_label_columns = Some(20);
            config.secondary_gap_columns = 3;
            let narrow_width = minimum_row_width(&config, 0, 1);
            assert_eq!(page_variant_at_width(&config, narrow_width).start, 1);
            assert_eq!(page_variant_at_width(&config, 80).start, 0);

            let frame = render(&TuiTabBarView::new(config), 80, app);
            let lines = frame.buffer.to_lines();
            assert_eq!(lines.len(), 1);
            assert!(lines[0].contains("haiku-2"));
            assert!(lines[0].contains("haiku-1"));
            assert!(lines[0].contains("haiku-3"));
            assert!(!lines[0].contains('←'));
            assert!(!lines[0].contains('→'));
        });
    });
}

#[test]
fn narrow_page_shows_label_content_or_defers_the_tab() {
    App::test((), |app| async move {
        app.read(|app| {
            let first_config = config(vec![
                tab(1, "infrastructure"),
                tab(2, "bravo"),
                tab(3, "charlie"),
            ]);
            let width = minimum_row_width(&first_config, 0, 1).saturating_add(2);
            let line = render(&TuiTabBarView::new(first_config), width, app)
                .buffer
                .to_lines()
                .remove(0);

            assert!(line.contains("..."));
            assert!(line.contains('→'));
            assert!(!line.contains("bravo"));

            let config = config(vec![tab(1, "alpha"), tab(2, "bravo"), tab(3, "charlie")]);
            let ellipsis_only_width = minimum_row_width(&config, 0, 3).saturating_sub(1);
            let page = page_variant_at_width(&config, ellipsis_only_width);
            assert_eq!(page.visible_count, 2);
            assert_eq!(page.next_start, Some(2));
            let line = render(&TuiTabBarView::new(config), ellipsis_only_width, app)
                .buffer
                .to_lines()
                .remove(0);
            assert!(line.contains("alpha"));
            assert!(line.contains("bravo"));
            assert!(!line.contains("..."));
            assert!(line.contains('→'));
        });
    });
}

#[test]
fn overflow_controls_match_the_page_edges() {
    App::test((), |app| async move {
        app.read(|app| {
            let tabs = || config(vec![tab(1, "alpha"), tab(2, "bravo"), tab(3, "charlie")]);

            let start_config = tabs();
            let two_tab_width = minimum_row_width(&start_config, 0, 2);
            let pages = deterministic_pages_at_width(&start_config, two_tab_width);
            assert_eq!(pages.len(), 2);
            assert_eq!(pages[0].start, 0);
            assert_eq!(pages[0].visible_count, 2);
            assert_eq!(pages[1].start, 2);
            assert_eq!(pages[1].previous_start, Some(0));
            let start_width = minimum_row_width(&start_config, 0, 1);
            let start = render(&TuiTabBarView::new(start_config), start_width, app)
                .buffer
                .to_lines()
                .remove(0);
            assert!(!start.contains('←'));
            assert!(start.contains('→'));

            let mut middle_config = tabs();
            middle_config.page_anchor = Some(key(2));
            let middle_width = minimum_row_width(&middle_config, 1, 1);
            let middle = render(&TuiTabBarView::new(middle_config), middle_width, app)
                .buffer
                .to_lines()
                .remove(0);
            assert!(middle.contains('←'));
            assert!(middle.contains('→'));

            let mut end_config = tabs();
            end_config.page_anchor = Some(key(3));
            let end = render(&TuiTabBarView::new(end_config), 40, app)
                .buffer
                .to_lines()
                .remove(0);
            assert!(end.contains('←'));
            assert!(!end.contains('→'));
        });
    });
}

#[test]
fn view_navigation_uses_semantic_order() {
    let mut config = config(vec![tab(2, "two"), tab(3, "three")]);
    config.main_tab = Some(tab(1, "main"));
    config.selected_key = Some(key(1));
    let view = TuiTabBarView::new(config);

    assert_eq!(
        view.navigation_target(TuiTabBarNavigationDirection::Previous),
        Some(key(3))
    );
    assert_eq!(
        view.navigation_target(TuiTabBarNavigationDirection::Next),
        Some(key(2))
    );
    assert_eq!(
        view.secondary_edge_target(TuiTabBarSecondaryEdge::First),
        Some(key(2))
    );
    assert_eq!(
        view.secondary_edge_target(TuiTabBarSecondaryEdge::Last),
        Some(key(3))
    );
}

#[test]
fn render_composes_selected_and_leading_styles() {
    App::test((), |app| async move {
        app.read(|app| {
            let mut config = config(vec![
                tab(1, "one").with_leading_text("*", TuiStyle::default().fg(Color::Yellow))
            ]);
            config.selected_key = Some(key(1));
            config.focused = true;
            let view = TuiTabBarView::new(config);
            let frame = render(&view, 20, app);

            assert!(frame.buffer.to_lines()[0].contains("* one"));
            assert_eq!(frame.buffer[(0, 0)].bg, Color::Magenta);
            assert_eq!(frame.buffer[(1, 0)].fg, Color::Yellow);
            assert!(frame.buffer[(3, 0)].modifier.contains(Modifier::BOLD));
        });
    });
}

#[test]
fn config_updates_reuse_live_mouse_state_and_prune_removed_tabs() {
    let mut view = TuiTabBarView::new(config(vec![tab(1, "one"), tab(2, "two")]));
    let first_handle = view.mouse_states.get(&key(1)).cloned().unwrap();
    view.config = config(vec![tab(1, "one"), tab(3, "three")]);
    view.reconcile_mouse_states();

    assert_eq!(view.mouse_states.len(), 2);
    assert!(!view.mouse_states.contains_key(&key(2)));
    assert!(Arc::ptr_eq(
        &first_handle,
        view.mouse_states.get(&key(1)).unwrap()
    ));
}
