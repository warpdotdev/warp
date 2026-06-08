use ai::agent::orchestration_config::{
    OrchestrationConfig, OrchestrationConfigStatus, OrchestrationExecutionMode,
};
use num_traits::Float;
use pathfinder_geometry::vector::vec2f;
use warpui::integration::{
    AssertionCallback, AssertionOutcome, AssertionWithDataCallback, StepDataMap, TestStep,
};
use warpui::units::Pixels;
use warpui::{App, Event, SingletonEntity, TypedActionView, ViewHandle, WindowId};

use crate::ai::ai_document_view::AIDocumentView;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentModel, AIDocumentVersion};
use crate::integration_testing::view_getters::{single_terminal_view_for_tab, workspace_view};
use crate::notebooks::editor::view::RichTextEditorView;
use crate::workspace::WorkspaceAction;

const SCROLL_OFFSET_FUDGE_FACTOR_PIXELS: Pixels = Pixels::new(0.01);
#[derive(Debug, Clone, Copy)]
struct AIDocumentScrollHeaderState {
    has_header: bool,
    header_scroll_top: Pixels,
    header_height: Pixels,
    content_scroll_top: Pixels,
}

fn scroll_offset_approx_zero(value: Pixels) -> bool {
    scroll_offsets_approx_eq(value, Pixels::zero())
}

fn scroll_offsets_approx_eq(a: Pixels, b: Pixels) -> bool {
    (a - b).abs() < SCROLL_OFFSET_FUDGE_FACTOR_PIXELS
}

fn scroll_offset_approx_lt(a: Pixels, b: Pixels) -> bool {
    a < b && !scroll_offsets_approx_eq(a, b)
}

/// Creates an AI document associated with the first terminal pane.
pub fn create_ai_document(
    document_key: impl Into<String>,
    title: impl Into<String>,
    content: impl Into<String>,
) -> TestStep {
    let document_key = document_key.into();
    let title = title.into();
    let content = content.into();
    TestStep::new("Create AI document").with_action(move |app, window_id, data| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
        let terminal_view_id = terminal_view.id();
        let conversation_id = BlocklistAIHistoryModel::handle(app).update(app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let document_id = AIDocumentModel::handle(app).update(app, |model, ctx| {
            model.create_document(&title, &content, conversation_id, None, ctx)
        });
        data.insert(document_key.clone(), document_id);
    })
}

/// Adds an approved local orchestration config to an AI document.
pub fn set_orchestration_config_for_ai_document(document_key: impl Into<String>) -> TestStep {
    let document_key = document_key.into();
    TestStep::new("Set AI document orchestration config").with_action(
        move |app, _window_id, data| {
            let document_id = document_id_from_data(data, &document_key);
            let config = OrchestrationConfig {
                model_id: "auto".to_string(),
                harness_type: "oz".to_string(),
                execution_mode: OrchestrationExecutionMode::Local,
            };
            AIDocumentModel::handle(app).update(app, |model, ctx| {
                let conversation_id = model
                    .get_conversation_id_for_document_id(&document_id)
                    .expect("AI document should have an associated conversation");
                model.set_orchestration_config_for_plan(
                    conversation_id,
                    document_id.to_string(),
                    config,
                    OrchestrationConfigStatus::Approved,
                    ctx,
                );
            });
        },
    )
}

/// Opens the AI document pane for a document saved in step data.
pub fn open_ai_document(document_key: impl Into<String>) -> TestStep {
    let document_key = document_key.into();
    TestStep::new("Open AI document").with_action(move |app, window_id, data| {
        let document_id = document_id_from_data(data, &document_key);
        workspace_view(app, window_id).update(app, |workspace, ctx| {
            workspace.handle_action(
                &WorkspaceAction::OpenAIDocumentPane {
                    document_id,
                    document_version: AIDocumentVersion::default(),
                },
                ctx,
            );
        });
    })
}

/// Sends a precise vertical scroll wheel event to the AI document pane.
pub fn scroll_ai_document_by(delta_y: f32) -> TestStep {
    TestStep::new("Scroll AI document").with_event_fn(move |app, window_id| {
        let document_view = match single_ai_document_view(app, window_id) {
            Ok(view) => view,
            Err(_) => panic!("expected exactly one AI document view"),
        };
        let view_position_id = format!("ai_document_view_{}", document_view.id());
        let position = document_view.read(app, |_view, ctx| {
            let rect = ctx
                .element_position_by_id_at_last_frame(window_id, &view_position_id)
                .expect("AI document view should have rendered");
            vec2f(
                (rect.origin_x() + rect.max_x()) / 2.,
                (rect.origin_y() + rect.max_y()) / 2.,
            )
        });
        Event::ScrollWheel {
            position,
            delta: vec2f(0., delta_y),
            precise: true,
            modifiers: Default::default(),
        }
    })
}

/// Records the AI document scroll-header state for a later assertion.
pub fn record_ai_document_scroll_header_state(snapshot_key: impl Into<String>) -> TestStep {
    let snapshot_key = snapshot_key.into();
    TestStep::new("Record AI document scroll-header state").with_action(
        move |app, window_id, data| {
            let state = match ai_document_scroll_header_state(app, window_id) {
                Ok(state) => state,
                Err(outcome) => panic!(
                    "failed to record AI document scroll-header state: {:?}",
                    outcome.as_failure_message()
                ),
            };
            data.insert(snapshot_key.clone(), state);
        },
    )
}

/// Asserts whether the AI document editor has a scroll header.
pub fn assert_ai_document_has_scroll_header(expected: bool) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let state = match ai_document_scroll_header_state(app, window_id) {
            Ok(state) => state,
            Err(outcome) => return outcome,
        };
        if state.has_header == expected {
            AssertionOutcome::Success
        } else {
            AssertionOutcome::failure(format!(
                "Expected AI document scroll header presence to be {expected}, got {}",
                state.has_header
            ))
        }
    })
}

/// Asserts the AI document header and content are both scrolled to the top.
pub fn assert_ai_document_header_at_top_with_content_at_top() -> AssertionCallback {
    Box::new(move |app, window_id| {
        assert_scroll_header_state(
            app,
            window_id,
            |has_header, header_scroll_top, _, content_scroll_top| {
                has_header
                    && scroll_offset_approx_zero(header_scroll_top)
                    && scroll_offset_approx_zero(content_scroll_top)
            },
            "Expected AI document header and content to be at the top",
        )
    })
}

/// Asserts the header has started scrolling away before content moves.
pub fn assert_ai_document_header_partially_hidden_before_content_scroll() -> AssertionCallback {
    Box::new(move |app, window_id| {
        assert_scroll_header_state(
            app,
            window_id,
            |has_header, header_scroll_top, header_height, content_scroll_top| {
                has_header
                    && !scroll_offset_approx_zero(header_scroll_top)
                    && scroll_offset_approx_lt(header_scroll_top, header_height)
                    && scroll_offset_approx_zero(content_scroll_top)
            },
            "Expected AI document header to be partially hidden before content scrolls",
        )
    })
}

/// Asserts the document content scrolls after the header is hidden.
pub fn assert_ai_document_content_scrolled_after_header() -> AssertionCallback {
    Box::new(move |app, window_id| {
        assert_scroll_header_state(
            app,
            window_id,
            |has_header, header_scroll_top, header_height, content_scroll_top| {
                has_header
                    && scroll_offsets_approx_eq(header_scroll_top, header_height)
                    && !scroll_offset_approx_zero(header_height)
                    && !scroll_offset_approx_zero(content_scroll_top)
            },
            "Expected AI document content to scroll after the header",
        )
    })
}

/// Asserts content scroll moved upward while the header remained fully consumed.
pub fn assert_ai_document_content_scrolled_up_before_header_reappears(
    snapshot_key: impl Into<String>,
) -> AssertionWithDataCallback {
    let snapshot_key = snapshot_key.into();
    Box::new(move |app, window_id, data| {
        let Some(previous) = data.get::<_, AIDocumentScrollHeaderState>(&snapshot_key) else {
            return AssertionOutcome::failure(format!(
                "Missing AI document scroll-header state snapshot for key {snapshot_key}"
            ));
        };
        let current = match ai_document_scroll_header_state(app, window_id) {
            Ok(state) => state,
            Err(outcome) => return outcome,
        };
        if current.has_header
            && !scroll_offset_approx_zero(current.header_height)
            && scroll_offsets_approx_eq(current.header_scroll_top, current.header_height)
            && !scroll_offset_approx_zero(current.content_scroll_top)
            && scroll_offset_approx_lt(current.content_scroll_top, previous.content_scroll_top)
        {
            AssertionOutcome::Success
        } else {
            AssertionOutcome::failure(format!(
                "Expected content to scroll upward before header reappears: previous={previous:?}, current={current:?}"
            ))
        }
    })
}

/// Returns the AI document ID stored by an earlier integration step.
fn document_id_from_data(data: &StepDataMap, key: &str) -> AIDocumentId {
    *data
        .get::<_, AIDocumentId>(key)
        .expect("AI document ID should be present in step data")
}

/// Returns the single AI document view in a window.
fn single_ai_document_view(
    app: &App,
    window_id: WindowId,
) -> Result<ViewHandle<AIDocumentView>, AssertionOutcome> {
    let views = app
        .views_of_type::<AIDocumentView>(window_id)
        .unwrap_or_default();
    if views.len() == 1 {
        Ok(views[0].clone())
    } else {
        Err(AssertionOutcome::failure(format!(
            "Expected exactly one AI document view, found {}",
            views.len()
        )))
    }
}

fn ai_document_scroll_header_state(
    app: &mut App,
    window_id: WindowId,
) -> Result<AIDocumentScrollHeaderState, AssertionOutcome> {
    let editor = single_ai_document_editor_view(app, window_id)?;
    Ok(editor.read(app, |editor, ctx| {
        let render_state = editor.model().as_ref(ctx).render_state().clone();
        let render_state = render_state.as_ref(ctx);
        let header_height = render_state.scroll_prefix_height();
        AIDocumentScrollHeaderState {
            has_header: !scroll_offset_approx_zero(header_height),
            header_scroll_top: render_state.viewport().scroll_top().min(header_height),
            header_height,
            content_scroll_top: render_state.content_scroll_top(),
        }
    }))
}

/// Returns the single AI document editor view in a window.
fn single_ai_document_editor_view(
    app: &App,
    window_id: WindowId,
) -> Result<ViewHandle<RichTextEditorView>, AssertionOutcome> {
    let views = app
        .views_of_type::<RichTextEditorView>(window_id)
        .unwrap_or_default();
    if views.len() == 1 {
        Ok(views[0].clone())
    } else {
        Err(AssertionOutcome::failure(format!(
            "Expected exactly one AI document editor view, found {}",
            views.len()
        )))
    }
}

/// Applies a predicate to the AI document scroll-header state.
fn assert_scroll_header_state(
    app: &mut App,
    window_id: WindowId,
    predicate: impl FnOnce(bool, Pixels, Pixels, Pixels) -> bool,
    failure_message: &str,
) -> AssertionOutcome {
    let state = match ai_document_scroll_header_state(app, window_id) {
        Ok(state) => state,
        Err(outcome) => return outcome,
    };
    if predicate(
        state.has_header,
        state.header_scroll_top,
        state.header_height,
        state.content_scroll_top,
    ) {
        AssertionOutcome::Success
    } else {
        AssertionOutcome::failure(format!("{failure_message}: state={state:?}"))
    }
}
