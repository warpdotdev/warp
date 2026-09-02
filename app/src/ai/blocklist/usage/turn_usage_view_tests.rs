//! Regression tests for [`TurnUsageView`]'s close handler, following the
//! same `handle_action`-driven pattern as `conversation_usage_view_tests.rs`,
//! plus a layout-alignment regression test for `build_label_value_columns`.

use warp_core::ui::appearance::Appearance;
use warpui::elements::{Flex, ParentElement};
use warpui::platform::WindowStyle;
use warpui::{App, Element, SingletonEntity};

use super::*;
use crate::persistence::model::PersistedModelTokenCost;
use crate::settings::UsageDisplayUnit;
use crate::test_util::settings::initialize_settings_for_tests;

fn placeholder_usage_info() -> TurnUsageInfo {
    TurnUsageInfo {
        models: vec![TurnModelUsage {
            model_id: "auto (cost-efficient)".to_string(),
            usage: PersistedModelTokenCost {
                total_input: 3,
                output: 1,
                input_cost_in_cents: 40.0,
                output_cost_in_cents: 20.0,
                ..Default::default()
            },
        }],
        context_window_usage: 0.001,
        platform_usage_in_cents: None,
        inference_credits_spent_for_last_block: None,
        platform_credits_spent_for_last_block: None,
        tool_calls: 2,
        files_changed: 1,
        lines_added: 4,
        lines_removed: 1,
        commands_executed: 1,
    }
}

fn initialize_test_app(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| Appearance::mock());
}

fn build_view(ctx: &mut warpui::ViewContext<TurnUsageView>) -> TurnUsageView {
    TurnUsageView::new(placeholder_usage_info(), None, ctx)
}

#[test]
fn close_action_emits_close_requested_event() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, build_view);

        let received = std::rc::Rc::new(std::cell::Cell::new(false));
        let received_clone = received.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&view, move |_, event, _| {
                if matches!(event, TurnUsageViewEvent::CloseRequested) {
                    received_clone.set(true);
                }
            });
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(&TurnUsageViewAction::Close, ctx);
        });

        assert!(
            received.get(),
            "Close action should emit TurnUsageViewEvent::CloseRequested"
        );
    });
}

/// Verifies row-by-row alignment between the label and value columns: every
/// label row must have a paired value row (a dropped/misaligned row changes
/// the `Flex::debug_text_content` line count, since `Empty` contributes no
/// line at all while a real `Text` contributes an empty line), and spot-
/// checks specific rows line up.
#[test]
fn build_label_value_columns_keeps_every_row_aligned_across_sections() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);

        let usage_info = TurnUsageInfo {
            models: vec![
                TurnModelUsage {
                    model_id: "claude-sonnet".to_string(),
                    usage: PersistedModelTokenCost {
                        total_input: 80,
                        output: 20,
                        input_cost_in_cents: 12.0,
                        ..Default::default()
                    },
                },
                TurnModelUsage {
                    model_id: "gpt-5".to_string(),
                    usage: PersistedModelTokenCost {
                        total_input: 40,
                        output: 10,
                        input_cost_in_cents: 6.0,
                        ..Default::default()
                    },
                },
            ],
            context_window_usage: 0.25,
            platform_usage_in_cents: Some(8.0),
            inference_credits_spent_for_last_block: None,
            platform_credits_spent_for_last_block: None,
            tool_calls: 3,
            files_changed: 2,
            lines_added: 5,
            lines_removed: 1,
            commands_executed: 4,
        };
        let timing_info = TimingInfo {
            time_to_first_token_ms: 500,
            total_agent_response_time_ms: 1500,
            wall_to_wall_response_time_ms: Some(2000),
        };
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TurnUsageView::new(usage_info, Some(timing_info), ctx)
        });

        view.read(&app, |view, ctx| {
            let appearance = Appearance::as_ref(ctx);
            let (labels, values) =
                view.build_label_value_columns(appearance, UsageDisplayUnit::Dollars);

            assert_eq!(
                labels.len(),
                values.len(),
                "each label row must have a paired value row"
            );

            let labels_text = Flex::column()
                .with_children(labels)
                .finish()
                .debug_text_content()
                .unwrap_or_default();
            let values_text = Flex::column()
                .with_children(values)
                .finish()
                .debug_text_content()
                .unwrap_or_default();

            let label_lines: Vec<&str> = labels_text.lines().collect();
            let value_lines: Vec<&str> = values_text.lines().collect();

            assert_eq!(
                label_lines.len(),
                value_lines.len(),
                "label column and value column must render the same number of text \
                 rows -- a mismatch here (e.g. a row backed by `Empty` on one side) \
                 causes every subsequent row to shift out of alignment with its \
                 counterpart.\nlabels:\n{labels_text}\n\nvalues:\n{values_text}"
            );

            let model_usage_header_index = label_lines
                .iter()
                .position(|line| *line == "INFERENCE USAGE")
                .expect("INFERENCE USAGE header should be present");
            assert_eq!(
                value_lines[model_usage_header_index], "150 tokens  /  $0.18",
                "the INFERENCE USAGE header's value should be the turn's total tokens \
                 and cost, summed across all models"
            );

            let context_window_index = label_lines
                .iter()
                .position(|line| *line == "Context window usage")
                .expect("Context window usage row should be present");
            assert_eq!(value_lines[context_window_index], "25%");

            let platform_usage_index = label_lines
                .iter()
                .position(|line| *line == "PLATFORM USAGE")
                .expect("PLATFORM USAGE section header should be present");
            assert_eq!(value_lines[platform_usage_index], "$0.08");
            assert!(
                platform_usage_index < context_window_index,
                "PLATFORM USAGE should be listed before Context window usage"
            );

            let tool_calls_index = label_lines
                .iter()
                .position(|line| *line == "Tool calls")
                .expect("Tool calls row should be present");
            assert_eq!(value_lines[tool_calls_index], "3");
        });
    });
}

/// Toggling a model row's expanded state should insert its input/output/
/// cache breakdown rows immediately after that row, and collapsing it again
/// should remove them.
#[test]
fn toggle_model_expanded_shows_and_hides_breakdown_rows() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);

        let usage_info = TurnUsageInfo {
            models: vec![TurnModelUsage {
                model_id: "claude-sonnet".to_string(),
                usage: PersistedModelTokenCost {
                    total_input: 80,
                    output: 20,
                    input_cache_read: 5,
                    input_cache_write: 3,
                    input_cost_in_cents: 100.0,
                    output_cost_in_cents: 200.0,
                    input_cache_read_cost_in_cents: 30.0,
                    input_cache_write_cost_in_cents: 20.0,
                    web_search_count: 2,
                    web_search_cost_in_cents: 300.0,
                },
            }],
            context_window_usage: 0.1,
            platform_usage_in_cents: None,
            inference_credits_spent_for_last_block: None,
            platform_credits_spent_for_last_block: None,
            tool_calls: 1,
            files_changed: 0,
            lines_added: 0,
            lines_removed: 0,
            commands_executed: 0,
        };
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TurnUsageView::new(usage_info, None, ctx)
        });

        let labels_text = |view: &TurnUsageView, ctx: &warpui::AppContext| {
            let appearance = Appearance::as_ref(ctx);
            let (labels, _values) =
                view.build_label_value_columns(appearance, UsageDisplayUnit::Dollars);
            Flex::column()
                .with_children(labels)
                .finish()
                .debug_text_content()
                .unwrap_or_default()
        };

        let values_text = |view: &TurnUsageView, ctx: &warpui::AppContext| {
            let appearance = Appearance::as_ref(ctx);
            let (_labels, values) =
                view.build_label_value_columns(appearance, UsageDisplayUnit::Dollars);
            Flex::column()
                .with_children(values)
                .finish()
                .debug_text_content()
                .unwrap_or_default()
        };

        view.read(&app, |view, ctx| {
            let text = labels_text(view, ctx);
            assert!(
                !text.contains("Input")
                    && !text.contains("Output")
                    && !text.contains("Cache")
                    && !text.contains("Web search"),
                "breakdown rows should not be present while collapsed:\n{text}"
            );
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(&TurnUsageViewAction::ToggleModelExpanded(0), ctx);
        });

        view.read(&app, |view, ctx| {
            let labels = labels_text(view, ctx);
            assert!(labels.contains("Input"), "expected Input row:\n{labels}");
            assert!(labels.contains("Output"), "expected Output row:\n{labels}");
            assert!(
                labels.contains("Cache read"),
                "expected Cache read row:\n{labels}"
            );
            assert!(
                labels.contains("Cache write"),
                "expected Cache write row:\n{labels}"
            );
            assert!(
                labels.contains("Web search"),
                "expected Web search row:\n{labels}"
            );

            let values = values_text(view, ctx);
            assert!(
                values.contains("80 tokens  /  $1.00"),
                "expected Input row to show tokens and cost:\n{values}"
            );
            assert!(
                values.contains("20 tokens  /  $2.00"),
                "expected Output row to show tokens and cost:\n{values}"
            );
            assert!(
                values.contains("5 tokens  /  $0.30"),
                "expected Cache read row to show its own tokens and cost:\n{values}"
            );
            assert!(
                values.contains("3 tokens  /  $0.20"),
                "expected Cache write row to show its own tokens and cost:\n{values}"
            );
            assert!(
                values.contains("2 searches  /  $3.00"),
                "expected Web search row to show count and cost:\n{values}"
            );
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(&TurnUsageViewAction::ToggleModelExpanded(0), ctx);
        });

        view.read(&app, |view, ctx| {
            let text = labels_text(view, ctx);
            assert!(
                !text.contains("Input")
                    && !text.contains("Output")
                    && !text.contains("Cache")
                    && !text.contains("Web search"),
                "breakdown rows should be removed after collapsing:\n{text}"
            );
        });
    });
}

/// A non-zero cost that rounds down to `$0.00` at two decimal places must
/// display as `<$0.01` instead, since rounding it away would misleadingly
/// suggest no cost was incurred. A true zero cost still shows as `$0.00`.
#[test]
fn format_dollars_shows_less_than_a_cent_for_tiny_nonzero_amounts() {
    assert_eq!(format_dollars(0.0), "$0.00");
    assert_eq!(format_dollars(0.4), "<$0.01");
    assert_eq!(format_dollars(0.999), "<$0.01");
    assert_eq!(format_dollars(1.0), "$0.01");
    assert_eq!(format_dollars(150.0), "$1.50");
}

/// A `Some(0.0)` platform usage is truly zero (as opposed to `None`, meaning
/// no charge data has arrived yet) and must be omitted entirely rather than
/// rendered as a noisy `$0.00` section.
#[test]
fn platform_usage_section_omitted_when_truly_zero() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);

        let mut usage_info = placeholder_usage_info();
        usage_info.platform_usage_in_cents = Some(0.0);
        let (_window_id, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            TurnUsageView::new(usage_info, None, ctx)
        });

        view.read(&app, |view, ctx| {
            let appearance = Appearance::as_ref(ctx);
            let (labels, _values) =
                view.build_label_value_columns(appearance, UsageDisplayUnit::Dollars);
            let labels_text = Flex::column()
                .with_children(labels)
                .finish()
                .debug_text_content()
                .unwrap_or_default();
            assert!(
                !labels_text.contains("PLATFORM USAGE"),
                "a truly-zero platform usage should not render a PLATFORM USAGE section:\n{labels_text}"
            );
        });
    });
}
