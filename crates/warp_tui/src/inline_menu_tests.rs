use warp::appearance::Appearance;
use warpui_core::App;
use warpui_core::elements::tui::{Modifier, TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;

use super::{
    TuiInlineMenuHeader, TuiInlineMenuListState, TuiInlineMenuRow, TuiInlineMenuRowStyle,
    TuiInlineMenuSnapshot, TuiInlineMenuStatus, TuiInlineMenuTab, render_inline_menu,
};
use crate::tui_builder::TuiUiBuilder;

fn render_at_size(snapshot: TuiInlineMenuSnapshot, width: u16, height: u16) -> Vec<String> {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(move |ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_inline_menu(&snapshot, &TuiUiBuilder::from_app(ctx)),
                TuiRect::new(0, 0, width, height),
                ctx,
            );
            frame.buffer.to_lines()
        })
    })
}

fn render_at_height(snapshot: TuiInlineMenuSnapshot, height: u16) -> Vec<String> {
    render_at_size(snapshot, 50, height)
}
fn render(snapshot: TuiInlineMenuSnapshot) -> Vec<String> {
    render_at_height(snapshot, 12)
}

fn status_snapshot(status: TuiInlineMenuStatus) -> TuiInlineMenuSnapshot {
    TuiInlineMenuSnapshot {
        header: None,
        rows: Vec::new(),
        selected_index: None,
        scroll_offset: 0,
        max_visible_rows: 8,
        status: Some(status),
    }
}

#[test]
fn renders_loading_and_empty_statuses() {
    let loading = render(status_snapshot(TuiInlineMenuStatus::Loading(
        "Loading conversations…".to_owned(),
    )));
    assert!(
        loading
            .iter()
            .any(|line| line.contains("Loading conversations…"))
    );

    let empty = render(status_snapshot(TuiInlineMenuStatus::Empty(
        "No conversations found".to_owned(),
    )));
    assert!(
        empty
            .iter()
            .any(|line| line.contains("No conversations found"))
    );
}

#[test]
fn renders_only_the_visible_row_window() {
    let lines = render(TuiInlineMenuSnapshot {
        header: None,
        rows: (0..5)
            .map(|index| TuiInlineMenuRow {
                title: format!("Conversation {index}"),
                description: None,
                is_selectable: true,
                style: TuiInlineMenuRowStyle::Default,
            })
            .collect(),
        selected_index: Some(3),
        scroll_offset: 2,
        max_visible_rows: 2,
        status: None,
    });
    let rendered = lines.join("\n");
    assert!(!rendered.contains("Conversation 1"));
    assert!(rendered.contains("Conversation 2"));
    assert!(rendered.contains("Conversation 3"));
    assert!(!rendered.contains("Conversation 4"));
}

#[test]
fn conversation_like_snapshot_reuses_header_tabs_rows_and_selection() {
    let lines = render(TuiInlineMenuSnapshot {
        header: Some(TuiInlineMenuHeader {
            title: Some("Conversations".to_owned()),
            tabs: vec![
                TuiInlineMenuTab {
                    label: "All".to_owned(),
                    is_selected: true,
                },
                TuiInlineMenuTab {
                    label: "Pinned".to_owned(),
                    is_selected: false,
                },
            ],
        }),
        rows: vec![
            TuiInlineMenuRow {
                title: "Current project".to_owned(),
                description: Some("2 minutes ago".to_owned()),
                is_selectable: true,
                style: TuiInlineMenuRowStyle::Default,
            },
            TuiInlineMenuRow {
                title: "Archived".to_owned(),
                description: None,
                is_selectable: false,
                style: TuiInlineMenuRowStyle::Default,
            },
        ],
        selected_index: Some(0),
        scroll_offset: 0,
        max_visible_rows: 8,
        status: None,
    });
    let rendered = lines.join("\n");
    assert!(rendered.contains("Conversations"));
    assert!(rendered.contains("[All]  Pinned"));
    assert!(!rendered.chars().any(|glyph| "┌┐└┘─│".contains(glyph)));
    assert!(rendered.contains("Current project  2 minutes ago"));
    assert!(rendered.contains("Archived"));
}

#[test]
fn conversation_like_snapshot_keeps_selection_visible_within_production_height() {
    let lines = render_at_height(
        TuiInlineMenuSnapshot {
            header: Some(TuiInlineMenuHeader {
                title: Some("Conversations".to_owned()),
                tabs: vec![
                    TuiInlineMenuTab {
                        label: "All".to_owned(),
                        is_selected: true,
                    },
                    TuiInlineMenuTab {
                        label: "Pinned".to_owned(),
                        is_selected: false,
                    },
                ],
            }),
            rows: (0..8)
                .map(|index| TuiInlineMenuRow {
                    title: format!("Conversation {index}"),
                    description: None,
                    is_selectable: true,
                    style: TuiInlineMenuRowStyle::Default,
                })
                .collect(),
            selected_index: Some(7),
            scroll_offset: 0,
            max_visible_rows: 8,
            status: None,
        },
        10,
    );

    assert_eq!(lines.len(), 10);
    let rendered = lines.join("\n");
    assert!(rendered.contains("Conversations"));
    assert!(rendered.contains("[All]  Pinned"));
    assert!(rendered.contains("Conversation 0"));
    assert!(rendered.contains("Conversation 1"));
    assert!(rendered.contains("Conversation 2"));
    assert!(rendered.contains("Conversation 7"));
}

#[test]
fn slash_command_rows_match_figma_layout_and_colors() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let snapshot = TuiInlineMenuSnapshot {
                header: None,
                rows: vec![
                    TuiInlineMenuRow {
                        title: "/agent".to_owned(),
                        description: Some("Start a new agent conversation".to_owned()),
                        is_selectable: true,
                        style: TuiInlineMenuRowStyle::InlineMenuItem,
                    },
                    TuiInlineMenuRow {
                        title: "/plan".to_owned(),
                        description: Some("Create a plan".to_owned()),
                        is_selectable: true,
                        style: TuiInlineMenuRowStyle::InlineMenuItem,
                    },
                ],
                selected_index: Some(0),
                scroll_offset: 0,
                max_visible_rows: 8,
                status: None,
            };
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_inline_menu(&snapshot, &builder),
                TuiRect::new(0, 0, 50, 2),
                ctx,
            );
            let lines = frame.buffer.to_lines();

            assert!(lines[0].starts_with("/agent                       Start"));
            assert!(lines[1].starts_with("/plan                        Create"));
            assert!(
                !lines
                    .iter()
                    .any(|line| line.chars().any(|glyph| "┌┐└┘─│".contains(glyph)))
            );
            assert_eq!(
                frame.buffer[(0, 0)].bg,
                builder.slash_command_selection_background()
            );
            assert_eq!(
                frame.buffer[(0, 0)].fg,
                builder
                    .slash_command_selection_text_style()
                    .fg
                    .expect("selected slash-command text has a foreground")
            );
            assert!(frame.buffer[(0, 0)].modifier.contains(Modifier::BOLD));
            assert_eq!(
                frame.buffer[(0, 1)].fg,
                builder
                    .slash_command_text_style()
                    .fg
                    .expect("slash-command text has a foreground")
            );
            assert_eq!(
                frame.buffer[(29, 1)].fg,
                builder
                    .primary_text_style()
                    .fg
                    .expect("slash-command descriptions use primary text")
            );
        });
    });
}

#[test]
fn long_slash_command_titles_are_ellipsized_before_the_description() {
    let lines = render(TuiInlineMenuSnapshot {
        header: None,
        rows: vec![TuiInlineMenuRow {
            title: "/respond-to-pr-comments-in-blocklist".to_owned(),
            description: Some("Walk users through PR review comments".to_owned()),
            is_selectable: true,
            style: TuiInlineMenuRowStyle::InlineMenuItem,
        }],
        selected_index: Some(0),
        scroll_offset: 0,
        max_visible_rows: 8,
        status: None,
    });

    assert!(lines[0].starts_with("/respond-to-pr-comments-i... Walk users"));
}

#[test]
fn wide_slash_command_rows_expand_to_show_long_titles() {
    let lines = render_at_size(
        TuiInlineMenuSnapshot {
            header: None,
            rows: vec![TuiInlineMenuRow {
                title: "/respond-to-pr-comments-in-blocklist".to_owned(),
                description: Some("Walk users through PR review comments".to_owned()),
                is_selectable: true,
                style: TuiInlineMenuRowStyle::InlineMenuItem,
            }],
            selected_index: Some(0),
            scroll_offset: 0,
            max_visible_rows: 8,
            status: None,
        },
        80,
        1,
    );

    assert!(
        lines[0].starts_with(
            "/respond-to-pr-comments-in-blocklist Walk users through PR review comments"
        )
    );
}

#[test]
fn boundary_width_preserves_useful_title_and_description_columns() {
    let lines = render_at_size(
        TuiInlineMenuSnapshot {
            header: None,
            rows: vec![TuiInlineMenuRow {
                title: "/agent".to_owned(),
                description: Some("Start a new agent conversation".to_owned()),
                is_selectable: true,
                style: TuiInlineMenuRowStyle::InlineMenuItem,
            }],
            selected_index: Some(0),
            scroll_offset: 0,
            max_visible_rows: 8,
            status: None,
        },
        20,
        1,
    );

    assert!(lines[0].starts_with("/agent  Start a new"));
}

#[test]
fn narrow_slash_command_rows_use_the_full_width_for_titles() {
    let lines = render_at_size(
        TuiInlineMenuSnapshot {
            header: None,
            rows: vec![TuiInlineMenuRow {
                title: "/12345678901234567890".to_owned(),
                description: Some("Description hidden at narrow widths".to_owned()),
                is_selectable: true,
                style: TuiInlineMenuRowStyle::InlineMenuItem,
            }],
            selected_index: Some(0),
            scroll_offset: 0,
            max_visible_rows: 8,
            status: None,
        },
        19,
        1,
    );

    assert_eq!(lines[0], "/123456789012345...");
}

#[test]
fn shared_list_navigation_wraps_skips_disabled_rows_and_scrolls() {
    let mut list = TuiInlineMenuListState::default();
    list.replace_rows(vec![true, false, true, true], false, Some(0), 2, |row| *row);

    list.select_next(2, |row| *row);
    assert_eq!(list.selected_index(), Some(2));
    assert_eq!(list.scroll_offset(), 1);

    list.select_next(2, |row| *row);
    assert_eq!(list.selected_index(), Some(3));
    assert_eq!(list.scroll_offset(), 2);

    list.select_next(2, |row| *row);
    assert_eq!(list.selected_index(), Some(0));
    assert_eq!(list.scroll_offset(), 0);

    list.select_previous(2, |row| *row);
    assert_eq!(list.selected_index(), Some(3));
    assert_eq!(list.scroll_offset(), 2);
}

#[test]
fn shared_list_preserves_ready_rows_while_a_mixer_query_loads() {
    let mut list = TuiInlineMenuListState::default();
    list.replace_rows(vec!["ready"], false, Some(0), 2, |_| true);

    let update = list.reconcile_mixer_rows(vec!["pending"], true, 2, |_| true);

    assert_eq!(
        update,
        warp_search_core::inline_menu::InlineMenuResultsUpdate::Loading
    );
    assert_eq!(list.rows(), &["ready"]);
    assert_eq!(list.selected_index(), Some(0));
    assert!(list.is_loading());
}
