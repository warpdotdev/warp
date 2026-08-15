use warp::appearance::Appearance;
use warp::tui_export::{AIConversationId, ConversationStatus, LoadedSubtreeRollup};
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, TuiView};

use super::{orchestration_tab_bar_config, rollup_badge_style};
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
        anchor_navigable: true,
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

/// The spec's depth-3 reference level: drilled into `crawler`
/// (root → researcher → crawler), so the bar carries TWO breadcrumb chips —
/// one back to the root and one to the parent (PRODUCT.md "Drilled into
/// crawler, depth 2" mockup).
fn depth_three_snapshot() -> TuiOrchestrationSnapshot {
    let root = AIConversationId::new();
    let researcher = AIConversationId::new();
    let crawler = AIConversationId::new();
    let fetch_a = AIConversationId::new();
    let fetch_b = AIConversationId::new();
    let children = vec![
        child(fetch_a, "fetch-a", 0, None),
        child(fetch_b, "fetch-b", 1, None),
    ];
    TuiOrchestrationSnapshot {
        root_conversation_id: root,
        anchor_conversation_id: crawler,
        anchor_label: "crawler".to_owned(),
        anchor_status: Some(ConversationStatus::InProgress),
        anchor_navigable: true,
        breadcrumbs: vec![
            TuiOrchestrationBreadcrumb {
                conversation_id: root,
                label: ORCHESTRATOR_TAB_LABEL.to_owned(),
            },
            TuiOrchestrationBreadcrumb {
                conversation_id: researcher,
                label: "researcher".to_owned(),
            },
        ],
        selected_conversation_id: crawler,
        children,
        page_anchor: Some(fetch_a),
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
        anchor_navigable: true,
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
fn depth_three_row_renders_two_breadcrumbs_through_the_ladder() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let snapshot = depth_three_snapshot();
            // T0 (≥ 96): both chips at full labels — the spec's depth-2 mockup
            // rendered for real (inter-tab gaps come from the bar's actual
            // accounting).
            let t0 =
                "   Agents:    ‹ orchestrator   ‹ researcher   ● crawler  |   ● fetch-a     ● fetch-b";
            assert_eq!(render_line(&snapshot, 100, ctx), format!("{t0:<100}"));
            // T1 (< 96): both chips cap at 8 cells and keep distinct labels.
            let t1 =
                "   Agents:    ‹ orche...   ‹ resea...   ● crawler  |   ● fetch-a     ● fetch-b";
            assert_eq!(render_line(&snapshot, 90, ctx), format!("{t1:<90}"));
            // T2 (< 84): the leading collapses; two capped chips still fit.
            let t2 = "   ‹ orche...   ‹ resea...   ● crawler  |   ● fetch-a     ● fetch-b";
            assert_eq!(render_line(&snapshot, 80, ctx), format!("{t2:<80}"));
            // T3 (< 72): both chips collapse to their markers; the anchor
            // keeps its label. Two `‹` cells preserve the two ascent targets.
            let t3 = "   ‹   ‹   ● crawler  |   ● fetch-a     ● fetch-b";
            assert_eq!(render_line(&snapshot, 70, ctx), format!("{t3:<70}"));
            // T4 (< 64): glyph-only anchor; the whole two-chip prefix costs 17
            // cells and both children still fit at 60 columns.
            let t4 = "   ‹   ‹   ●  |   ● fetch-a     ● fetch-b";
            assert_eq!(render_line(&snapshot, 60, ctx), format!("{t4:<60}"));
            // T5 (< 56): same row — the ladder holds with two chips down to
            // the narrowest tier for the spec's reference tree.
            assert_eq!(render_line(&snapshot, 50, ctx), format!("{t4:<50}"));
        });
    });
}

#[test]
fn rollup_badge_colors_follow_the_design_mapping() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            let builder = TuiUiBuilder::from_app(ctx);
            // Yellow while anything underneath is working or stuck.
            for status in [
                ConversationStatus::InProgress,
                ConversationStatus::TransientError,
                ConversationStatus::WaitingForEvents,
                ConversationStatus::Blocked {
                    blocked_action: "approval".to_owned(),
                },
            ] {
                assert_eq!(
                    rollup_badge_style(&status, &builder),
                    builder.attention_glyph_style(),
                    "{status:?} must read yellow"
                );
            }
            // Red when the settled subtree contains a failure.
            assert_eq!(
                rollup_badge_style(&ConversationStatus::Error, &builder),
                builder.error_text_style()
            );
            // neutral_7 when everything settled without one.
            for status in [ConversationStatus::Success, ConversationStatus::Cancelled] {
                assert_eq!(
                    rollup_badge_style(&status, &builder),
                    builder.neutral_7_text_style(),
                    "{status:?} must read neutral_7"
                );
            }
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
            // Byte-for-byte flag-off equivalence with the historical row:
            // 13-cell leading, padded `orchestrator` main tab, `pad(1)+|+pad(2)`
            // divider, child tabs `pad(1)+glyph+space+label+pad(1)` with a
            // 3-cell gap, and background fill to the full width.
            let expected = format!(
                "{:<100}",
                "   Agents:    orchestrator  |   ● alpha     ● beta"
            );
            assert_eq!(
                line, expected,
                "the flag-off flat projection must be unchanged"
            );
        });
    });
}
