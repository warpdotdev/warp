use warp::appearance::Appearance;
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::App;

use super::{
    render_inline_menu, TuiInlineMenuHeader, TuiInlineMenuRow, TuiInlineMenuSnapshot,
    TuiInlineMenuStatus, TuiInlineMenuTab,
};
use crate::tui_builder::TuiUiBuilder;

fn render_at_height(snapshot: TuiInlineMenuSnapshot, height: u16) -> Vec<String> {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(move |ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_inline_menu(&snapshot, &TuiUiBuilder::from_app(ctx)),
                TuiRect::new(0, 0, 50, height),
                ctx,
            );
            frame.buffer.to_lines()
        })
    })
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
    assert!(loading
        .iter()
        .any(|line| line.contains("Loading conversations…")));

    let empty = render(status_snapshot(TuiInlineMenuStatus::Empty(
        "No conversations found".to_owned(),
    )));
    assert!(empty
        .iter()
        .any(|line| line.contains("No conversations found")));
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
            },
            TuiInlineMenuRow {
                title: "Archived".to_owned(),
                description: None,
                is_selectable: false,
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
    assert!(!rendered.contains("Conversation 0"));
    assert!(!rendered.contains("Conversation 1"));
    assert!(rendered.contains("Conversation 2"));
    assert!(rendered.contains("Conversation 7"));
}
