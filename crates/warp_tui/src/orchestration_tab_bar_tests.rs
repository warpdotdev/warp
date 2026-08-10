use warp::appearance::Appearance;
use warp::tui_export::{AIConversationId, ConversationStatus, LoadedSubtreeRollup};
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, TuiView};

use super::orchestration_tab_bar_config;
use crate::orchestration_model::{
    ORCHESTRATOR_TAB_LABEL, TuiOrchestrationBreadcrumb, TuiOrchestrationChild,
    TuiOrchestrationSnapshot,
};
use crate::tab_bar::TuiTabBarView;
use crate::tui_builder::TuiUiBuilder;

fn child(
    conversation_id: AIConversationId,
    label: &str,
    spawn_index: usize,
    subtree_rollup: Option<LoadedSubtreeRollup>,
) -> TuiOrchestrationChild {
    TuiOrchestrationChild {
        conversation_id,
        label: label.to_owned(),
        spawn_index,
        status: ConversationStatus::InProgress,
        subtree_rollup,
    }
}

/// The spec's reference level: drilled into `researcher` (a child of the
/// root), whose level holds a group child `crawler` (subtree of 2) plus two
/// leaves.
fn drilled_snapshot() -> TuiOrchestrationSnapshot {
    let root = AIConversationId::new();
    let researcher = AIConversationId::new();
    let crawler = AIConversationId::new();
    let indexer = AIConversationId::new();
    let ranker = AIConversationId::new();
    let children = vec![
        child(
            crawler,
            "crawler",
            0,
            Some(LoadedSubtreeRollup {
                descendant_count: 2,
                status: ConversationStatus::InProgress,
            }),
        ),
        child(indexer, "indexer", 1, None),
        child(ranker, "ranker", 2, None),
    ];
    TuiOrchestrationSnapshot {
        root_conversation_id: root,
        anchor_conversation_id: researcher,
        anchor_label: "researcher".to_owned(),
        anchor_status: Some(ConversationStatus::InProgress),
        breadcrumbs: vec![TuiOrchestrationBreadcrumb {
            conversation_id: root,
            label: ORCHESTRATOR_TAB_LABEL.to_owned(),
        }],
        selected_conversation_id: researcher,
        children,
        page_anchor: Some(crawler),
        reveal_selected: true,
    }
}

/// A flag-off flat snapshot: root anchored, no breadcrumbs, no rollups.
fn flat_snapshot() -> TuiOrchestrationSnapshot {
    let root = AIConversationId::new();
    let alpha = AIConversationId::new();
    let beta = AIConversationId::new();
    let children = vec![child(alpha, "alpha", 0, None), child(beta, "beta", 1, None)];
    TuiOrchestrationSnapshot {
        root_conversation_id: root,
        anchor_conversation_id: root,
        anchor_label: ORCHESTRATOR_TAB_LABEL.to_owned(),
        anchor_status: None,
        breadcrumbs: Vec::new(),
        selected_conversation_id: root,
        children,
        page_anchor: Some(alpha),
        reveal_selected: true,
    }
}

fn render_line(snapshot: &TuiOrchestrationSnapshot, width: u16, app: &AppContext) -> String {
    let config = orchestration_tab_bar_config(snapshot, false, &TuiUiBuilder::from_app(app));
    let view = TuiTabBarView::new(config).expect("valid tab bar config");
    TuiPresenter::new()
        .present_element(view.render(app), TuiRect::new(0, 0, width, 1), app)
        .buffer
        .to_lines()
        .remove(0)
}

#[test]
fn full_width_row_shows_breadcrumb_anchor_glyph_and_rollup_badge() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let line = render_line(&drilled_snapshot(), 100, ctx);
            assert!(
                line.contains("   Agents:    ‹ orchestrator   ● researcher  |   ● crawler ▸2"),
                "T0 row must show the breadcrumb chip, glyph-bearing anchor, and full badge: \
                 {line:?}"
            );
            assert!(line.contains("● indexer"), "leaf child renders: {line:?}");
            assert!(line.contains("● ranker"), "leaf child renders: {line:?}");
        });
    });
}

#[test]
fn t2_row_collapses_the_leading_and_caps_breadcrumb_labels() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let line = render_line(&drilled_snapshot(), 80, ctx);
            assert!(
                !line.contains("Agents:"),
                "T2 sheds the Agents: leading: {line:?}"
            );
            assert!(
                line.contains("‹ orche..."),
                "T2 caps the breadcrumb label at 8 cells: {line:?}"
            );
            assert!(
                line.contains("● researcher"),
                "the anchor keeps its label at T2: {line:?}"
            );
            assert!(line.contains("▸2"), "the full badge survives T2: {line:?}");
        });
    });
}

#[test]
fn t4_row_keeps_marker_only_breadcrumb_glyph_anchor_and_badge_marker() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let line = render_line(&drilled_snapshot(), 60, ctx);
            assert!(
                line.contains("‹   ●  |"),
                "T4 keeps the breadcrumb marker, the anchor glyph, and the divider: {line:?}"
            );
            assert!(
                !line.contains("researcher"),
                "T4 collapses the anchor to its glyph: {line:?}"
            );
            assert!(
                line.contains("crawler ▸") && !line.contains("▸2"),
                "T4 shrinks the badge to its marker: {line:?}"
            );
        });
    });
}

#[test]
fn ladder_variants_attach_only_to_the_multi_level_presentation() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            let drilled = orchestration_tab_bar_config(&drilled_snapshot(), false, &builder);
            assert_eq!(drilled.narrow_variants.len(), 5);

            let flat = orchestration_tab_bar_config(&flat_snapshot(), false, &builder);
            assert!(flat.narrow_variants.is_empty());
            assert!(flat.breadcrumb_tabs.is_empty());
        });
    });
}

#[test]
fn flat_snapshot_renders_the_historical_root_anchored_row() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let line = render_line(&flat_snapshot(), 100, ctx);
            assert!(
                line.contains("   Agents:    orchestrator  |   ● alpha"),
                "the flag-off flat projection is unchanged: {line:?}"
            );
            assert!(line.contains("● beta"), "{line:?}");
            assert!(!line.contains('‹'), "no breadcrumbs while flat: {line:?}");
            assert!(!line.contains('▸'), "no badges while flat: {line:?}");
        });
    });
}
