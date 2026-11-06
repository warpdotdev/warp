use std::sync::Arc;

use warpui_core::elements::tui::{Color, Modifier, TuiBufferExt, TuiRect, TuiStyle};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, TuiView};

use super::{
    TuiTab, TuiTabBarConfig, TuiTabBarConfigError, TuiTabBarNarrowVariant,
    TuiTabBarNavigationDirection, TuiTabBarPagingState, TuiTabBarSecondaryEdge, TuiTabBarStyles,
    TuiTabBarView, deterministic_pages_at_width, fixed_prefix_width, minimum_label_width,
    minimum_row_width, natural_tab_width, page_variant_at_width, validated_live_keys,
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
        background: Some(Color::Black),
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

fn view(config: TuiTabBarConfig) -> TuiTabBarView {
    TuiTabBarView::new(config).unwrap()
}

#[test]
fn tab_availability_is_derived_from_the_retained_config() {
    let empty = view(config(Vec::new()));
    assert!(!empty.has_tabs());

    let secondary = view(config(vec![tab(1, "one")]));
    assert!(secondary.has_tabs());

    let mut main_only = config(Vec::new());
    main_only.main_tab = Some(tab(1, "main"));
    assert!(view(main_only).has_tabs());
}
#[test]
fn paging_state_preserves_only_a_valid_explicit_anchor() {
    let mut state = TuiTabBarPagingState::default();
    let automatic = state.resolve(Some(key(1)), |_| false);
    assert_eq!(automatic.page_anchor, Some(key(1)));
    assert!(automatic.reveal_selected);

    state.set_explicit_anchor(key(2));
    let explicit = state.resolve(Some(key(1)), |anchor| anchor == "2");
    assert_eq!(explicit.page_anchor, Some(key(2)));
    assert!(!explicit.reveal_selected);

    let invalid = state.resolve(Some(key(1)), |_| false);
    assert_eq!(invalid.page_anchor, Some(key(1)));
    assert!(invalid.reveal_selected);

    let cleared = TuiTabBarPagingState::<String>::default().resolve(Some(key(1)), |_| true);
    assert_eq!(cleared.page_anchor, Some(key(1)));
    assert!(cleared.reveal_selected);
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

            let frame = render(&view(config), 80, app);
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
            let line = render(&view(first_config), width, app)
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
            let line = render(&view(config), ellipsis_only_width, app)
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
            let start = render(&view(start_config), start_width, app)
                .buffer
                .to_lines()
                .remove(0);
            assert!(!start.contains('←'));
            assert!(start.contains('→'));

            let mut middle_config = tabs();
            middle_config.page_anchor = Some(key(2));
            let middle_width = minimum_row_width(&middle_config, 1, 1);
            let middle = render(&view(middle_config), middle_width, app)
                .buffer
                .to_lines()
                .remove(0);
            assert!(middle.contains('←'));
            assert!(middle.contains('→'));

            let mut end_config = tabs();
            end_config.page_anchor = Some(key(3));
            let end = render(&view(end_config), 40, app)
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
    let view = view(config);

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
fn view_navigation_includes_breadcrumbs_in_row_order() {
    let mut config = config(vec![tab(3, "child-a"), tab(4, "child-b")]);
    config.breadcrumb_tabs = vec![tab(1, "orchestrator")];
    config.main_tab = Some(tab(2, "anchor"));
    config.selected_key = Some(key(2));
    let view = view(config);

    // Row order is breadcrumb, anchor, children — wrapping across the ends.
    assert_eq!(
        view.navigation_target(TuiTabBarNavigationDirection::Previous),
        Some(key(1))
    );
    assert_eq!(
        view.navigation_target(TuiTabBarNavigationDirection::Next),
        Some(key(3))
    );
    // Shift-edges stay scoped to the level's children.
    assert_eq!(
        view.secondary_edge_target(TuiTabBarSecondaryEdge::First),
        Some(key(3))
    );
    assert_eq!(
        view.secondary_edge_target(TuiTabBarSecondaryEdge::Last),
        Some(key(4))
    );
}

#[test]
fn view_navigation_skips_non_selectable_tabs_in_both_directions() {
    let non_selectable_anchor_config = |selected: String| {
        let mut config = config(vec![tab(3, "child-a"), tab(4, "child-b")]);
        config.breadcrumb_tabs = vec![tab(1, "orchestrator")];
        // A sessionless drilled-down anchor renders but cannot be selected.
        config.main_tab = Some(tab(2, "anchor").with_selectable(false));
        config.selected_key = Some(selected);
        config
    };

    // Previous from the first child skips the anchor and lands on the
    // breadcrumb; Next from the breadcrumb skips it in the other direction.
    let from_child = view(non_selectable_anchor_config(key(3)));
    assert_eq!(
        from_child.navigation_target(TuiTabBarNavigationDirection::Previous),
        Some(key(1))
    );
    let from_breadcrumb = view(non_selectable_anchor_config(key(1)));
    assert_eq!(
        from_breadcrumb.navigation_target(TuiTabBarNavigationDirection::Next),
        Some(key(3))
    );
}

#[test]
fn tree_root_key_prefers_the_first_breadcrumb() {
    let mut cfg = config(vec![tab(2, "two"), tab(3, "three")]);
    cfg.main_tab = Some(tab(1, "main"));
    let bar = view(cfg);
    assert_eq!(bar.tree_root_key(), Some(key(1)));

    let mut drilled = config(vec![tab(3, "child")]);
    drilled.breadcrumb_tabs = vec![tab(1, "orchestrator")];
    drilled.main_tab = Some(tab(2, "anchor"));
    let drilled_bar = view(drilled);
    assert_eq!(drilled_bar.tree_root_key(), Some(key(1)));

    let no_main = view(config(vec![tab(1, "one")]));
    assert_eq!(no_main.tree_root_key(), None);
}

#[test]
fn breadcrumbs_render_in_the_fixed_prefix_and_reserve_width() {
    App::test((), |app| async move {
        app.read(|app| {
            let mut cfg = config(vec![tab(3, "crawler"), tab(4, "indexer")]);
            cfg.leading = Some("   Agents:   ".to_owned());
            cfg.breadcrumb_tabs =
                vec![tab(1, "orchestrator").with_leading_text("‹", TuiStyle::default())];
            cfg.main_tab = Some(tab(2, "researcher").with_leading_text("●", TuiStyle::default()));
            cfg.secondary_gap_columns = 3;

            // The fixed prefix accounts for the breadcrumb chip plus its gap.
            let breadcrumb_width = natural_tab_width(&cfg.breadcrumb_tabs[0], &cfg);
            let mut without_breadcrumbs = cfg.clone();
            without_breadcrumbs.breadcrumb_tabs.clear();
            assert_eq!(
                fixed_prefix_width(&cfg),
                fixed_prefix_width(&without_breadcrumbs) + breadcrumb_width + 1
            );

            let line = render(&view(cfg), 100, app).buffer.to_lines().remove(0);
            assert!(
                line.contains("‹ orchestrator   ● researcher"),
                "breadcrumb chip should precede the anchor with a one-cell gap: {line}"
            );
            assert!(line.contains('|'), "divider still renders: {line}");
        });
    });
}

#[test]
fn trailing_badge_renders_after_the_label_and_reserves_width() {
    App::test((), |app| async move {
        app.read(|app| {
            let plain = tab(1, "researcher");
            let badged = tab(1, "researcher").with_trailing_text("▸3", TuiStyle::default());
            let cfg = config(vec![badged.clone()]);
            assert_eq!(
                natural_tab_width(&badged, &cfg),
                // Badge text (2 cells) plus its separating space.
                natural_tab_width(&plain, &cfg) + 3
            );

            let line = render(&view(cfg), 40, app).buffer.to_lines().remove(0);
            assert!(
                line.contains("researcher ▸3"),
                "badge should trail the label: {line}"
            );
        });
    });
}

#[test]
fn narrow_variants_swap_presentation_below_their_width_bounds() {
    App::test((), |app| async move {
        app.read(|app| {
            let mut base = config(vec![
                tab(2, "crawler").with_trailing_text("▸2", TuiStyle::default()),
                tab(3, "indexer"),
            ]);
            base.leading = Some("   Agents:   ".to_owned());
            base.main_tab = Some(tab(1, "researcher").with_leading_text("●", TuiStyle::default()));
            base.secondary_gap_columns = 3;

            let mut narrow = base.clone();
            narrow.leading = Some("  ".to_owned());
            narrow.main_tab =
                Some(TuiTab::new(key(1), "").with_leading_text("●", TuiStyle::default()));
            narrow.tabs = vec![
                tab(2, "crawler").with_trailing_text("▸", TuiStyle::default()),
                tab(3, "indexer"),
            ];
            base.narrow_variants = vec![TuiTabBarNarrowVariant {
                max_width_exclusive: 64,
                config: narrow,
            }];

            let bar = view(base);
            let wide = render(&bar, 100, app).buffer.to_lines().remove(0);
            assert!(
                wide.contains("Agents:"),
                "wide row keeps the leading: {wide}"
            );
            assert!(
                wide.contains("researcher"),
                "wide row keeps the anchor label: {wide}"
            );
            assert!(wide.contains("▸2"), "wide row keeps the full badge: {wide}");

            let narrow_line = render(&bar, 60, app).buffer.to_lines().remove(0);
            assert!(
                !narrow_line.contains("Agents:"),
                "narrow row sheds the leading: {narrow_line}"
            );
            assert!(
                !narrow_line.contains("researcher"),
                "narrow row collapses the anchor to its glyph: {narrow_line}"
            );
            assert!(
                narrow_line.contains('●'),
                "the anchor glyph survives every tier: {narrow_line}"
            );
            assert!(
                narrow_line.contains("crawler ▸") && !narrow_line.contains("▸2"),
                "the badge collapses to its marker: {narrow_line}"
            );
        });
    });
}

#[test]
fn render_composes_selected_and_leading_styles() {
    App::test((), |app| async move {
        app.read(|app| {
            let mut config = config(vec![
                tab(1, "one").with_leading_text("*", TuiStyle::default().fg(Color::Yellow)),
            ]);
            config.selected_key = Some(key(1));
            config.focused = true;
            let view = view(config);
            let frame = render(&view, 20, app);

            assert!(frame.buffer.to_lines()[0].contains("* one"));
            assert_eq!(frame.buffer[(0, 0)].bg, Color::Magenta);
            assert_eq!(frame.buffer[(1, 0)].fg, Color::Yellow);
            assert!(frame.buffer[(3, 0)].modifier.contains(Modifier::BOLD));
            assert_eq!(frame.buffer[(19, 0)].bg, Color::Black);
        });
    });
}

#[test]
fn minimum_label_width_measures_the_first_grapheme() {
    let config = config(vec![]);
    let tab = TuiTab::new("flag", "🇺🇸long");

    assert_eq!(minimum_label_width(&tab, &config), 5);
}

#[test]
fn config_rejects_a_label_cap_that_can_only_render_ellipsis() {
    let mut config = config(vec![tab(1, "infrastructure")]);
    config.maximum_label_columns = Some(3);
    assert_eq!(
        TuiTabBarView::new(config).err(),
        Some(TuiTabBarConfigError::LabelWidthTooSmall {
            key: key(1),
            configured: 3,
            required: 4,
        })
    );
}

#[test]
fn config_rejects_duplicate_keys_across_main_and_secondary_tabs() {
    let mut config = config(vec![tab(1, "secondary")]);
    config.main_tab = Some(tab(1, "main"));
    assert_eq!(
        TuiTabBarView::new(config).err(),
        Some(TuiTabBarConfigError::DuplicateKey(key(1)))
    );
}
#[test]
fn config_updates_reuse_live_mouse_state_and_prune_removed_tabs() {
    let mut view = view(config(vec![tab(1, "one"), tab(2, "two")]));
    let first_handle = view.mouse_states.get(&key(1)).cloned().unwrap();
    view.config = config(vec![tab(1, "one"), tab(3, "three")]);
    let live_keys = validated_live_keys(&view.config).unwrap();
    view.reconcile_mouse_states(live_keys);

    assert_eq!(view.mouse_states.len(), 2);
    assert!(!view.mouse_states.contains_key(&key(2)));
    assert!(Arc::ptr_eq(
        &first_handle,
        view.mouse_states.get(&key(1)).unwrap()
    ));
}
