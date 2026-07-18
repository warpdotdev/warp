use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIAgentExchangeId, AIAgentInput, AIAgentOutput, AIAgentOutputMessage, AIAgentOutputMessageType,
    AIAgentText, AIAgentTextSection, AIBlockModel, AIBlockOutputStatus, AIConversationId,
    AIRequestType, Appearance, BlockHeight, BlockHeightItem, CancellationReason, LLMId, MessageId,
    OutputStatusUpdateCallback, RichContentItem, RichContentType, ServerOutputId, Shared,
    TerminalModel, UserQueryMode,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, EntityId, EntityIdMap, ViewHandle};
use warpui_core::elements::tui::{
    TuiBufferExt, TuiConstraint, TuiGridPoint, TuiLayoutContext, TuiRect, TuiSelectionSpan,
    TuiSize, TuiViewportContent, TuiViewportWindow, TuiViewportedElement,
};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, TuiView, TypedActionView, ViewContext};

use super::{AgentBlockRegistry, TuiBlockListViewportItemId, TuiBlockListViewportSource};
use crate::agent_block::{TuiAIBlock, TuiAIBlockAction, TuiAIBlockEvent};
use crate::terminal_block::should_render_terminal_block;
use crate::test_fixtures::{add_test_action_model_and_events, TestHostView};

#[test]
fn tui_block_list_viewport_source_uses_canonical_block_list_order() {
    let mut model = TerminalModel::mock(None, None);
    model.simulate_block("echo 1", "1\r\n");
    model.simulate_block("echo 2", "2\r\n");
    let expected = model
        .block_list()
        .blocks()
        .iter()
        .filter(|block| should_render_terminal_block(block, model.block_list()))
        .map(|block| TuiBlockListViewportItemId::Terminal(block.id().clone()))
        .collect::<Vec<_>>();
    let source = TuiBlockListViewportSource::new(
        Arc::new(FairMutex::new(model)),
        AgentBlockRegistry::new(RefCell::new(HashMap::new())),
    );

    let actual = source.item_ids_for_test();

    assert_eq!(actual, expected);
}

#[test]
fn tui_block_list_viewport_source_slices_terminal_blocks_to_visible_rows() {
    App::test((), |app| async move {
        app.read(|app| {
            let mut model = TerminalModel::mock(None, None);
            model.simulate_block("printf", "one\r\ntwo\r\nthree\r\n");
            let source = TuiBlockListViewportSource::new(
                Arc::new(FairMutex::new(model)),
                AgentBlockRegistry::new(RefCell::new(HashMap::new())),
            );

            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let content = source.visible_items(
                TuiViewportWindow {
                    scroll_top: 1,
                    viewport_height: 1,
                },
                80,
                &mut layout_ctx,
                app,
            );

            assert_eq!(content.items.len(), 1);
            let mut item = content.items.into_iter().next().unwrap();
            assert_eq!(item.origin_y, 1);

            let size = item.element.layout(
                TuiConstraint::loose(TuiSize::new(80, u16::MAX)),
                &mut layout_ctx,
                app,
            );
            assert_eq!(size.height, 1);
        });
    });
}

#[test]
fn viewport_layout_omits_unchanged_agent_block_resize() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (source, model, agent_block) = seeded_agent_block_source(&mut app, 0, 99.0);
        let expected = measured_height(&app, &agent_block);
        model
            .lock()
            .block_list_mut()
            .update_rich_content_heights_in_lines(&HashMap::from([(
                agent_block.id(),
                BlockHeight::from(expected),
            )]));

        request_top_window(&app, &source, 10);

        assert!(source.take_selection_row_resizes().is_empty());
    });
}

/// Verifies read-only extraction preserves cached heights and dirty state.
#[test]
fn read_only_content_does_not_remeasure_agent_blocks() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (source, model, agent_block) = seeded_agent_block_source(&mut app, 0, 99.0);

        source.read_only_content(
            TuiViewportWindow {
                scroll_top: 0,
                viewport_height: 10,
            },
            80,
        );

        assert_eq!(rich_content_height(&model, agent_block.id()), Some(99.0));
    });
}

/// Verifies layout reports resize records before updating canonical heights.
#[test]
fn viewport_layout_reports_original_agent_block_resize() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (source, _, agent_block) = seeded_agent_block_source(&mut app, 0, 99.0);
        let expected = app.read(|app| {
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            agent_block.as_ref(app).desired_height(80, &mut ctx, app)
        });

        request_top_window(&app, &source, 10);
        let changes = source.take_selection_row_resizes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].old_rows, 0..99);
        assert_eq!(changes[0].new_height, expected);
    });
}

#[test]
fn tui_agent_rich_content_stays_visible_without_gui_agent_view_state() {
    let mut model = TerminalModel::mock(None, None);
    let view_id = EntityId::new();
    model.block_list_mut().append_rich_content(
        RichContentItem::new(Some(RichContentType::AIBlock), view_id, None, false),
        false,
    );
    model
        .block_list_mut()
        .update_rich_content_heights(&HashMap::from([(view_id, 3.0)]));

    let rich_content = model
        .block_list()
        .block_heights()
        .cursor::<(), ()>()
        .find_map(|item| match item {
            BlockHeightItem::RichContent(item) if item.view_id == view_id => Some(item),
            BlockHeightItem::Block(_)
            | BlockHeightItem::Gap(_)
            | BlockHeightItem::RestoredBlockSeparator { .. }
            | BlockHeightItem::InlineBanner { .. }
            | BlockHeightItem::SubshellSeparator { .. }
            | BlockHeightItem::RichContent(_) => None,
        })
        .expect("TUI agent rich content should remain in the canonical block list");

    assert!(!rich_content.should_hide);
    assert!(rich_content.last_laid_out_height.as_f64() > 0.0);
}

#[test]
fn tui_agent_overhang_remeasures_visible_non_dirty_height() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (source, model, agent_block) = seeded_agent_block_source(&mut app, 0, 99.0);
        let expected = measured_height(&app, &agent_block);

        // On the first measure (no recorded width) the visible block is
        // re-measured during `visible_items`, so its stale height is corrected
        // before windowing without any post-layout pass.
        let content = request_top_window(&app, &source, 10);

        assert_ne!(expected, 99.0);
        assert_eq!(content.content_height, expected as usize);
        assert_eq!(
            rich_content_height(&model, agent_block.id()),
            Some(expected)
        );

        // A subsequent frame at the same width does not re-measure: a wrong
        // height seeded without dirtying is preserved rather than corrected
        // every frame.
        seed_clean_height(&app, &model, &agent_block, 1234.0, 80);
        request_top_window(&app, &source, 10);
        assert_eq!(rich_content_height(&model, agent_block.id()), Some(1234.0));
    });
}

#[test]
fn tui_agent_overhang_remeasures_near_offscreen_non_dirty_height() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        // A short terminal block pushes the agent block below a 1-row viewport
        // but within the overhang band.
        let (source, model, agent_block) = seeded_agent_block_source(&mut app, 3, 99.0);
        let expected = measured_height(&app, &agent_block);

        request_top_window(&app, &source, 1);

        assert_ne!(expected, 99.0);
        assert_eq!(
            rich_content_height(&model, agent_block.id()),
            Some(expected)
        );
    });
}

#[test]
fn tui_agent_beyond_overhang_keeps_stale_height() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        // A tall terminal block pushes the agent block beyond the overhang band.
        let (source, model, agent_block) = seeded_agent_block_source(&mut app, 30, 7.0);

        request_top_window(&app, &source, 1);

        // Beyond OVERHANG_ROWS: not re-measured, so the stale height is retained.
        assert_eq!(rich_content_height(&model, agent_block.id()), Some(7.0));
    });
}

#[test]
fn tui_transcript_scroll_reuses_cached_heights_at_stable_width() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (source, model, agent_block) = seeded_agent_block_source(&mut app, 0, 99.0);

        // First frame measures the real height and records the width it was
        // measured at.
        let correct = measured_height(&app, &agent_block);
        request_top_window(&app, &source, 10);
        assert_eq!(rich_content_height(&model, agent_block.id()), Some(correct));
        source.take_selection_row_resizes();

        // A stable-width frame must not re-measure a non-dirty, non-streaming
        // block: a wrong height seeded without dirtying is preserved and no
        // resize is emitted.
        seed_clean_height(&app, &model, &agent_block, 1234.0, 80);
        request_top_window(&app, &source, 10);
        assert_eq!(rich_content_height(&model, agent_block.id()), Some(1234.0));
        assert!(source.take_selection_row_resizes().is_empty());

        // A width change forces a re-measure, correcting the height.
        request_top_window_at_width(&app, &source, 10, 40);
        assert_ne!(rich_content_height(&model, agent_block.id()), Some(1234.0));

        // Dirtying forces a re-measure even at a stable width.
        seed_clean_height(&app, &model, &agent_block, 4321.0, 40);
        model
            .lock()
            .block_list_mut()
            .mark_rich_content_dirty(agent_block.id());
        request_top_window_at_width(&app, &source, 10, 40);
        assert_ne!(rich_content_height(&model, agent_block.id()), Some(4321.0));
    });
}

#[test]
fn tui_agent_streaming_block_remeasured_at_stable_width() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        // A streaming block's height can grow without a per-update
        // invalidation, so it must be re-measured at a stable width.
        let (source, model, agent_block) = streaming_agent_block_source(&mut app);

        request_top_window(&app, &source, 10);
        source.take_selection_row_resizes();

        // Seed a wrong height at the same width without dirtying; the streaming
        // block is still re-measured, correcting it.
        seed_clean_height(&app, &model, &agent_block, 1234.0, 80);
        request_top_window(&app, &source, 10);
        assert_ne!(rich_content_height(&model, agent_block.id()), Some(1234.0));
    });
}

#[test]
fn completed_markdown_output_update_refreshes_cached_scroll_extent() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (source, model, agent_block, block_model) =
            updating_agent_block_source(&mut app, completed_markdown_status("Short response"));
        let width = 40;

        let initial_height = request_top_window_at_width(&app, &source, 4, width).content_height;
        source.take_selection_row_resizes();

        block_model.update_status(
            completed_markdown_status(
                "# Expanded response\n\n- first item\n- second item\n- third item\n\n\
                 ## Details\n\nThis paragraph adds enough semantic Markdown rows to grow the block.",
            ),
            &agent_block,
            &mut app,
        );
        let updated_height = request_top_window_at_width(&app, &source, 4, width).content_height;
        let resizes = source.take_selection_row_resizes();

        assert!(
            updated_height > initial_height,
            "completed Markdown growth must increase the canonical scroll extent \
             ({updated_height} vs {initial_height})"
        );
        assert_eq!(resizes.len(), 1);
        assert_eq!(resizes[0].old_rows, 0..initial_height);
        assert_eq!(resizes[0].new_height, updated_height);
        assert_eq!(
            rich_content_height(&model, agent_block.id()),
            Some(updated_height as f64)
        );
    });
}

#[test]
fn tui_transcript_toggle_expands_and_remeasures_block_at_stable_width() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (source, _model, agent_block) = reasoning_agent_block_source(&mut app);

        // A finished thinking section renders collapsed (header only).
        assert_eq!(
            render_block_lines(&app, &agent_block, 40),
            vec!["Thought for 2 seconds ▸".to_owned()],
        );

        // First frame measures and caches the collapsed height at width 80.
        let collapsed = request_top_window(&app, &source, 20).content_height;
        source.take_selection_row_resizes();

        // Without any interaction, a second stable-width frame reuses the cached
        // height (the fix's steady-state no-op still holds).
        assert_eq!(
            request_top_window(&app, &source, 20).content_height,
            collapsed
        );

        // Expanding the thinking section must invalidate
        // the cached height so the next frame re-measures even though the width
        // is unchanged and the block is not streaming.
        expand_thinking_section(&mut app, &agent_block);
        let expanded = request_top_window(&app, &source, 20).content_height;
        assert!(
            expanded > collapsed,
            "expanding the thinking section must grow the re-measured height ({expanded} vs {collapsed})"
        );

        // The expanded block renders the reasoning body beneath the header.
        let expanded_lines = render_block_lines(&app, &agent_block, 40);
        assert_eq!(expanded_lines[0], "Thought for 2 seconds ▾");
        assert!(
            expanded_lines
                .iter()
                .any(|line| line.contains("reasoning line two")),
            "{expanded_lines:?}"
        );
    });
}

/// A selection covering the whole wrapped input section copies the original
/// query verbatim (logical text), not the per-row grid scrape. Guards the TUI
/// copy bug where a multi-row selection pasted with inserted newlines.
#[test]
fn selection_logical_text_returns_query_for_full_input_selection() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        // `seeded_agent_block_source` registers a block whose input query is
        // "hello world from rust".
        let query = "hello world from rust";
        let (source, _model, agent_block) = seeded_agent_block_source(&mut app, 0, 99.0);
        let width = 12u16;
        let block_height = app.read(|app| {
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            agent_block.as_ref(app).desired_height(width, &mut ctx, app)
        });
        // The block is input-only, so its height is the top padding row plus the
        // wrapped input; a wrapped input spans more than one row here.
        assert!(
            block_height > 2,
            "input should wrap at width {width}: {block_height}"
        );
        let selection = TuiSelectionSpan {
            start: TuiGridPoint { row: 1, col: 0 },
            end: TuiGridPoint {
                row: block_height - 1,
                col: width,
            },
        };

        let text = app.read(|app| source.selection_logical_text(selection, width, app));

        assert_eq!(text.as_deref(), Some(query));
    });
}

/// A selection that does not cover the whole input section (here starting
/// mid-line) returns `None`, so copy falls back to the per-row grid path.
#[test]
fn selection_logical_text_falls_back_for_partial_input_selection() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (source, _model, _agent_block) = seeded_agent_block_source(&mut app, 0, 99.0);
        let width = 12u16;
        let selection = TuiSelectionSpan {
            start: TuiGridPoint { row: 1, col: 1 },
            end: TuiGridPoint { row: 2, col: width },
        };

        let text = app.read(|app| source.selection_logical_text(selection, width, app));

        assert_eq!(text, None);
    });
}

/// A selection that ends part-way through the section's final wrapped row (the
/// end column stops short of the last glyph) falls back to per-row grid text,
/// so the whole logical section is not returned and unselected trailing text is
/// never copied.
#[test]
fn selection_logical_text_falls_back_for_partial_end_selection() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (source, _model, agent_block) = seeded_agent_block_source(&mut app, 0, 99.0);
        let width = 12u16;
        let block_height = app.read(|app| {
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            agent_block.as_ref(app).desired_height(width, &mut ctx, app)
        });
        assert!(
            block_height > 2,
            "input should wrap at width {width}: {block_height}"
        );
        // Start at the section's first column, but end at column 1 of the last
        // rendered row — short of the last glyph.
        let selection = TuiSelectionSpan {
            start: TuiGridPoint { row: 1, col: 0 },
            end: TuiGridPoint {
                row: block_height - 1,
                col: 1,
            },
        };

        let text = app.read(|app| source.selection_logical_text(selection, width, app));

        assert_eq!(text, None);
    });
}

/// A selection over an agent block's input and plain-text response sections
/// copies both as logical text joined by a newline — agent *output* text is
/// covered too, not just user input.
#[test]
fn selection_logical_text_covers_agent_plain_text_output() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let output = "the agent replied with a fairly long answer";
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let agent_block = add_agent_block_with(
            &mut app,
            vec![query_input("hi")],
            plain_text_output_status(output),
            terminal_model.clone(),
        );
        let view_id = agent_block.id();
        {
            let mut model = terminal_model.lock();
            model.block_list_mut().append_rich_content(
                RichContentItem::new(Some(RichContentType::AIBlock), view_id, None, false),
                false,
            );
            model.block_list_mut().take_dirty_rich_content_items();
            model
                .block_list_mut()
                .update_rich_content_heights(&HashMap::from([(view_id, 99.0)]));
        }
        let agent_blocks = AgentBlockRegistry::new(RefCell::new(HashMap::from([(
            view_id,
            agent_block.clone(),
        )])));
        let source = TuiBlockListViewportSource::new(terminal_model.clone(), agent_blocks);

        let width = 16u16;
        let block_height = app.read(|app| {
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            agent_block.as_ref(app).desired_height(width, &mut ctx, app)
        });
        // Select the whole block (both sections), from the blank top padding row
        // through the last rendered row.
        let selection = TuiSelectionSpan {
            start: TuiGridPoint { row: 0, col: 0 },
            end: TuiGridPoint {
                row: block_height - 1,
                col: width,
            },
        };

        let text = app.read(|app| source.selection_logical_text(selection, width, app));

        assert_eq!(text, Some(format!("hi\n{output}")));
    });
}

/// Builds a source over one registered agent block seeded with a stale,
/// non-dirty cached height. When `preceding_rows > 0`, a terminal block of that
/// many output rows precedes it, controlling how far below the top it sits.
fn seeded_agent_block_source(
    app: &mut App,
    preceding_rows: usize,
    stale_height: f64,
) -> (
    TuiBlockListViewportSource,
    Arc<FairMutex<TerminalModel>>,
    ViewHandle<TuiAIBlock>,
) {
    seeded_agent_block_source_impl(app, preceding_rows, stale_height, false)
}

/// Like [`seeded_agent_block_source`] but the registered agent block is still
/// streaming, so its height can grow without a per-update invalidation.
fn streaming_agent_block_source(
    app: &mut App,
) -> (
    TuiBlockListViewportSource,
    Arc<FairMutex<TerminalModel>>,
    ViewHandle<TuiAIBlock>,
) {
    seeded_agent_block_source_impl(app, 0, 99.0, true)
}

fn seeded_agent_block_source_impl(
    app: &mut App,
    preceding_rows: usize,
    stale_height: f64,
    streaming: bool,
) -> (
    TuiBlockListViewportSource,
    Arc<FairMutex<TerminalModel>>,
    ViewHandle<TuiAIBlock>,
) {
    let mut model = TerminalModel::mock(None, None);
    if preceding_rows > 0 {
        model.simulate_block("printf", &"x\r\n".repeat(preceding_rows));
    }
    let terminal_model = Arc::new(FairMutex::new(model));
    let agent_block = add_agent_block(app, "hello world from rust", streaming);
    let view_id = agent_block.id();
    {
        let mut model = terminal_model.lock();
        model.block_list_mut().append_rich_content(
            RichContentItem::new(Some(RichContentType::AIBlock), view_id, None, false),
            false,
        );
        // Clear the dirty flag and seed a stale height so only re-measurement
        // (not the dirty path) can correct it.
        model.block_list_mut().take_dirty_rich_content_items();
        model
            .block_list_mut()
            .update_rich_content_heights(&HashMap::from([(view_id, stale_height)]));
    }
    let agent_blocks = AgentBlockRegistry::new(RefCell::new(HashMap::from([(
        view_id,
        agent_block.clone(),
    )])));
    let source = TuiBlockListViewportSource::new(terminal_model.clone(), agent_blocks);
    (source, terminal_model, agent_block)
}

/// Runs the overhang + windowing pass for a top-anchored viewport at width 80.
fn request_top_window(
    app: &App,
    source: &TuiBlockListViewportSource,
    viewport_height: u16,
) -> TuiViewportContent {
    request_top_window_at_width(app, source, viewport_height, 80)
}

/// Runs the overhang + windowing pass for a top-anchored viewport at `width`.
fn request_top_window_at_width(
    app: &App,
    source: &TuiBlockListViewportSource,
    viewport_height: u16,
    width: u16,
) -> TuiViewportContent {
    app.read(|app| {
        let mut rendered_views = EntityIdMap::default();
        let mut ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        source.visible_items(
            TuiViewportWindow {
                scroll_top: 0,
                viewport_height,
            },
            width,
            &mut ctx,
            app,
        )
    })
}

/// Seeds a rich-content height at `width` without marking it dirty, so a later
/// stable-width frame re-measures it only if the width-gating logic decides to
/// (e.g. because the block is streaming).
fn seed_clean_height(
    app: &App,
    model: &Arc<FairMutex<TerminalModel>>,
    agent_block: &ViewHandle<TuiAIBlock>,
    height: f64,
    width: u16,
) {
    let mut model = model.lock();
    model.block_list_mut().take_dirty_rich_content_items();
    model
        .block_list_mut()
        .update_rich_content_heights_in_lines(&HashMap::from([(
            agent_block.id(),
            BlockHeight::from(height),
        )]));
    drop(model);
    app.read(|app| agent_block.as_ref(app).record_height_measurement(width));
}

/// Builds a source over one finished agent block with a collapsible thought.
fn reasoning_agent_block_source(
    app: &mut App,
) -> (
    TuiBlockListViewportSource,
    Arc<FairMutex<TerminalModel>>,
    ViewHandle<TuiAIBlock>,
) {
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    let agent_block = add_agent_block_with(
        app,
        Vec::new(),
        finished_reasoning_status("reasoning line one\nreasoning line two\nreasoning line three"),
        terminal_model.clone(),
    );
    let view_id = agent_block.id();
    let model_for_layout_events = terminal_model.clone();
    app.update(|ctx| {
        ctx.subscribe_to_view(&agent_block, move |_, event, _| match event {
            TuiAIBlockEvent::LayoutInvalidated => {
                model_for_layout_events
                    .lock()
                    .block_list_mut()
                    .mark_rich_content_dirty(view_id);
            }
            TuiAIBlockEvent::BlockingStateChanged => {}
        });
    });
    {
        let mut model = terminal_model.lock();
        model.block_list_mut().append_rich_content(
            RichContentItem::new(Some(RichContentType::AIBlock), view_id, None, false),
            false,
        );
        model.block_list_mut().take_dirty_rich_content_items();
    }
    let agent_blocks = AgentBlockRegistry::new(RefCell::new(HashMap::from([(
        view_id,
        agent_block.clone(),
    )])));
    let source = TuiBlockListViewportSource::new(terminal_model.clone(), agent_blocks);
    (source, terminal_model, agent_block)
}

/// Expands the finished thinking section through the owning view action.
fn expand_thinking_section(app: &mut App, agent_block: &ViewHandle<TuiAIBlock>) {
    agent_block.update(app, |block, ctx| {
        block.handle_action(
            &TuiAIBlockAction::SetSectionCollapsed {
                message_id: MessageId::new("reasoning-1".to_owned()),
                collapsed: false,
            },
            ctx,
        );
    });
}

/// Renders the agent block to trimmed, non-empty terminal lines at `width` — a
/// `TuiBuffer::to_lines` snapshot of what the transcript paints for the block.
fn render_block_lines(app: &App, agent_block: &ViewHandle<TuiAIBlock>, width: u16) -> Vec<String> {
    app.read(|app| {
        let mut rendered_views = EntityIdMap::default();
        let mut ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        let height = agent_block
            .as_ref(app)
            .desired_height(width, &mut ctx, app)
            .max(1) as u16;
        let mut presenter = TuiPresenter::new();
        let frame = presenter.present_element(
            agent_block.as_ref(app).render(app),
            TuiRect::new(0, 0, width, height),
            app,
        );
        frame
            .buffer
            .to_lines()
            .into_iter()
            .map(|line| line.trim_end().to_owned())
            .filter(|line| !line.is_empty())
            .collect()
    })
}

/// Measures the agent block at width 80 by laying out its rendered element.
fn measured_height(app: &App, agent_block: &ViewHandle<TuiAIBlock>) -> f64 {
    app.read(|app| {
        let mut rendered_views = EntityIdMap::default();
        let mut ctx = TuiLayoutContext {
            rendered_views: &mut rendered_views,
        };
        let mut element = agent_block.as_ref(app).render(app);
        let height = element
            .layout(
                TuiConstraint::loose(TuiSize::new(80, u16::MAX)),
                &mut ctx,
                app,
            )
            .height;
        f64::from(height)
    })
}

/// Adds a `TuiAIBlock` backed by a single-query model in a fresh TUI
/// window and returns its handle. `streaming` controls whether the block's
/// model reports itself as still streaming.
fn add_agent_block(app: &mut App, query: &str, streaming: bool) -> ViewHandle<TuiAIBlock> {
    let status = if streaming {
        AIBlockOutputStatus::Pending
    } else {
        non_streaming_status()
    };
    add_agent_block_with(
        app,
        vec![query_input(query)],
        status,
        Arc::new(FairMutex::new(TerminalModel::mock(None, None))),
    )
}

/// Adds a `TuiAIBlock` over `inputs`/`status`, backed by `terminal_model`, in a
/// fresh TUI window. Passing the same `terminal_model` the viewport source uses
/// lets the block's interaction toggles dirty the source's canonical rich
/// content.
fn add_agent_block_with(
    app: &mut App,
    inputs: Vec<AIAgentInput>,
    status: AIBlockOutputStatus,
    terminal_model: Arc<FairMutex<TerminalModel>>,
) -> ViewHandle<TuiAIBlock> {
    let (action_model, model_events) = add_test_action_model_and_events(app);
    app.update(|ctx| {
        let (window_id, _) = ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| TestHostView,
        );
        ctx.add_typed_action_tui_view(window_id, move |ctx| {
            TuiAIBlock::new(
                AIConversationId::new(),
                AIAgentExchangeId::new(),
                Rc::new(QueryAgentBlockModel { inputs, status }),
                action_model,
                &model_events,
                terminal_model,
                ctx,
            )
        })
    })
}

fn updating_agent_block_source(
    app: &mut App,
    status: AIBlockOutputStatus,
) -> (
    TuiBlockListViewportSource,
    Arc<FairMutex<TerminalModel>>,
    ViewHandle<TuiAIBlock>,
    Rc<UpdatingAgentBlockModel>,
) {
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    let block_model = Rc::new(UpdatingAgentBlockModel {
        status: RefCell::new(status),
        callback: RefCell::new(None),
    });
    let (action_model, model_events) = add_test_action_model_and_events(app);
    let terminal_model_for_block = terminal_model.clone();
    let block_model_for_block = block_model.clone();
    let agent_block = app.update(|ctx| {
        let (window_id, _) = ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| TestHostView,
        );
        ctx.add_typed_action_tui_view(window_id, move |ctx| {
            TuiAIBlock::new(
                AIConversationId::new(),
                AIAgentExchangeId::new(),
                block_model_for_block,
                action_model,
                &model_events,
                terminal_model_for_block,
                ctx,
            )
        })
    });
    let view_id = agent_block.id();
    let terminal_model_for_events = terminal_model.clone();
    app.update(|ctx| {
        ctx.subscribe_to_view(&agent_block, move |_, event, _| match event {
            TuiAIBlockEvent::LayoutInvalidated => {
                terminal_model_for_events
                    .lock()
                    .block_list_mut()
                    .mark_rich_content_dirty(view_id);
            }
            TuiAIBlockEvent::BlockingStateChanged => {}
        });
    });
    terminal_model.lock().block_list_mut().append_rich_content(
        RichContentItem::new(Some(RichContentType::AIBlock), view_id, None, false),
        false,
    );
    let agent_blocks = AgentBlockRegistry::new(RefCell::new(HashMap::from([(
        view_id,
        agent_block.clone(),
    )])));
    (
        TuiBlockListViewportSource::new(terminal_model.clone(), agent_blocks),
        terminal_model,
        agent_block,
        block_model,
    )
}

fn completed_markdown_status(markdown: &str) -> AIBlockOutputStatus {
    AIBlockOutputStatus::Complete {
        output: Shared::new(AIAgentOutput {
            messages: vec![AIAgentOutputMessage {
                id: MessageId::new("markdown-1".to_owned()),
                message: AIAgentOutputMessageType::Text(AIAgentText {
                    sections: vec![AIAgentTextSection::PlainText {
                        text: markdown.to_owned().into(),
                    }],
                }),
                citations: Vec::new(),
            }],
            ..Default::default()
        }),
    }
}
/// A finished (cancelled) status: the block is not streaming, so the viewport's
/// width-gating alone decides whether to re-measure it.
fn non_streaming_status() -> AIBlockOutputStatus {
    AIBlockOutputStatus::Cancelled {
        partial_output: None,
        reason: CancellationReason::ManuallyCancelled,
    }
}

/// A completed output carrying a single finished reasoning ("thinking")
/// section, which renders collapsed by default and expands to show `body`.
fn finished_reasoning_status(body: &str) -> AIBlockOutputStatus {
    AIBlockOutputStatus::Complete {
        output: Shared::new(AIAgentOutput {
            messages: vec![AIAgentOutputMessage {
                id: MessageId::new("reasoning-1".to_owned()),
                message: AIAgentOutputMessageType::Reasoning {
                    text: AIAgentText {
                        sections: vec![AIAgentTextSection::PlainText {
                            text: body.to_owned().into(),
                        }],
                    },
                    finished_duration: Some(Duration::from_secs(2)),
                },
                citations: Vec::new(),
            }],
            ..Default::default()
        }),
    }
}

/// A completed output carrying a single plain-text response section.
fn plain_text_output_status(text: &str) -> AIBlockOutputStatus {
    AIBlockOutputStatus::Complete {
        output: Shared::new(AIAgentOutput {
            messages: vec![AIAgentOutputMessage {
                id: MessageId::new("text-1".to_owned()),
                message: AIAgentOutputMessageType::Text(AIAgentText {
                    sections: vec![AIAgentTextSection::PlainText {
                        text: text.to_owned().into(),
                    }],
                }),
                citations: Vec::new(),
            }],
            ..Default::default()
        }),
    }
}

struct QueryAgentBlockModel {
    inputs: Vec<AIAgentInput>,
    /// The output status this fake model reports: `Pending` models a streaming
    /// block, `Cancelled` a finished (non-streaming) one, and `Complete`
    /// carries rendered output such as a reasoning section.
    status: AIBlockOutputStatus,
}

impl AIBlockModel for QueryAgentBlockModel {
    type View = TuiAIBlock;

    fn status(&self, _app: &AppContext) -> AIBlockOutputStatus {
        self.status.clone()
    }

    fn server_output_id(&self, _app: &AppContext) -> Option<ServerOutputId> {
        None
    }

    fn model_id(&self, _app: &AppContext) -> Option<LLMId> {
        None
    }

    fn base_model<'a>(&'a self, _app: &'a AppContext) -> Option<&'a LLMId> {
        None
    }

    fn inputs_to_render<'a>(&'a self, _app: &'a AppContext) -> &'a [AIAgentInput] {
        &self.inputs
    }

    fn conversation_id(&self, _app: &AppContext) -> Option<AIConversationId> {
        None
    }

    fn on_updated_output(
        &self,
        _callback: OutputStatusUpdateCallback<Self::View>,
        _ctx: &mut ViewContext<Self::View>,
    ) {
    }

    fn request_type(&self, _app: &AppContext) -> AIRequestType {
        AIRequestType::Active
    }
}

struct UpdatingAgentBlockModel {
    status: RefCell<AIBlockOutputStatus>,
    callback: RefCell<Option<OutputStatusUpdateCallback<TuiAIBlock>>>,
}

impl UpdatingAgentBlockModel {
    fn update_status(
        &self,
        status: AIBlockOutputStatus,
        view: &ViewHandle<TuiAIBlock>,
        app: &mut App,
    ) {
        *self.status.borrow_mut() = status;
        view.update(app, |view, ctx| {
            self.callback
                .borrow_mut()
                .as_mut()
                .expect("agent block registered its output callback")(view, ctx);
        });
    }
}

impl AIBlockModel for UpdatingAgentBlockModel {
    type View = TuiAIBlock;

    fn status(&self, _app: &AppContext) -> AIBlockOutputStatus {
        self.status.borrow().clone()
    }

    fn server_output_id(&self, _app: &AppContext) -> Option<ServerOutputId> {
        None
    }

    fn model_id(&self, _app: &AppContext) -> Option<LLMId> {
        None
    }

    fn base_model<'a>(&'a self, _app: &'a AppContext) -> Option<&'a LLMId> {
        None
    }

    fn inputs_to_render<'a>(&'a self, _app: &'a AppContext) -> &'a [AIAgentInput] {
        &[]
    }

    fn conversation_id(&self, _app: &AppContext) -> Option<AIConversationId> {
        None
    }

    fn on_updated_output(
        &self,
        callback: OutputStatusUpdateCallback<Self::View>,
        _ctx: &mut ViewContext<Self::View>,
    ) {
        *self.callback.borrow_mut() = Some(callback);
    }

    fn request_type(&self, _app: &AppContext) -> AIRequestType {
        AIRequestType::Active
    }
}

/// Builds one user-query input for wrapping-height tests.
fn query_input(query: &str) -> AIAgentInput {
    AIAgentInput::UserQuery {
        query: query.to_owned(),
        context: Default::default(),
        static_query_type: None,
        referenced_attachments: Default::default(),
        user_query_mode: UserQueryMode::default(),
        running_command: None,
        intended_agent: None,
    }
}

/// Returns the cached rich-content height for a view ID.
fn rich_content_height(model: &Arc<FairMutex<TerminalModel>>, view_id: EntityId) -> Option<f64> {
    model
        .lock()
        .block_list()
        .block_heights()
        .cursor::<(), ()>()
        .find_map(|item| match item {
            BlockHeightItem::RichContent(item) if item.view_id == view_id => {
                Some(item.last_laid_out_height.as_f64())
            }
            BlockHeightItem::Block(_)
            | BlockHeightItem::Gap(_)
            | BlockHeightItem::RestoredBlockSeparator { .. }
            | BlockHeightItem::InlineBanner { .. }
            | BlockHeightItem::SubshellSeparator { .. }
            | BlockHeightItem::RichContent(_) => None,
        })
}
