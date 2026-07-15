use std::rc::Rc;
use std::sync::Arc;

use ai::agent::action::{
    CreateDocumentsRequest, DocumentDiff, DocumentToCreate, EditDocumentsRequest,
};
use ai::agent::action_result::{
    AIAgentActionResultType, CreateDocumentsResult, DocumentContext, EditDocumentsResult,
};
use ai::document::{AIDocumentId, AIDocumentVersion};
use warp::tui_export::{
    AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionType, AIConversationId,
    Appearance, TaskId,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, TypedActionView};
use warpui_core::elements::tui::{Color, Modifier, TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, TuiView, ViewHandle};

use super::{TuiPlanCodeKey, TuiPlanView, TuiPlanViewAction, TuiPlanViewEvent};
use crate::test_fixtures::{add_test_action_model, TestHostView};
use crate::tui_builder::TuiUiBuilder;

#[test]
fn streamed_create_renders_cached_markdown_and_code_children() {
    App::test((), |mut app| async move {
        let content = "# Overview\n\nBuild a **fast** timer.\n\n```rust\nfn main() {}\n```";
        let action = create_action("create-1", [("pomodoro-app-spec.md", content)]);
        let (view, _) = test_plan_view(&mut app, action.clone(), true);

        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert!(view.renders_rich_body());
            assert_eq!(view.documents.len(), 1);
            assert_eq!(view.code_views.len(), 1);
            assert_eq!(view.child_view_ids(ctx).len(), 1);
            let formatted = view.documents[0]
                .formatted
                .as_ref()
                .expect("streamed plan parses")
                .clone();

            let (lines, buffer) = render(view, 80, ctx);
            assert_eq!(lines[0], "○ Creating pomodoro-app-spec.md ▾");
            assert_eq!(lines[1], "");
            assert_eq!(lines[2], "  Overview");
            assert!(lines
                .iter()
                .any(|line| line.trim() == "Build a fast timer."));
            assert!(lines.iter().all(|line| !line.contains("**")));
            assert!(lines
                .iter()
                .skip(2)
                .filter(|line| !line.is_empty())
                .all(|line| line.starts_with("  ")));
            let plan_background = TuiUiBuilder::from_app(ctx).plan_background();
            assert_eq!(buffer[(0, 0)].bg, Color::Reset);
            assert_eq!(buffer[(0, 1)].bg, plan_background);
            assert_eq!(buffer[(79, 1)].bg, plan_background);
            assert_eq!(buffer[(0, 2)].bg, plan_background);
            assert_eq!(buffer[(79, 2)].bg, plan_background);
            assert_eq!(
                buffer[(11, 0)].fg,
                TuiUiBuilder::from_app(ctx).link_text_style().fg.unwrap()
            );
            assert!(buffer[(11, 0)].modifier.contains(Modifier::BOLD));
            assert!(!buffer[(11, 0)].modifier.contains(Modifier::UNDERLINED));

            let code_key = TuiPlanCodeKey {
                document_index: 0,
                code_index: 0,
            };
            let code_view = view.code_views[&code_key].as_ref(ctx);
            let (code_lines, _) = render_code(code_view, ctx);
            assert!(code_lines.iter().any(|line| line.contains("fn main()")));

            assert!(Arc::ptr_eq(
                &formatted,
                view.documents[0].formatted.as_ref().unwrap()
            ));
        });

        view.update(&mut app, |view, ctx| {
            let formatted = view.documents[0].formatted.as_ref().unwrap().clone();
            view.sync_action(action, true, ctx);
            assert!(Arc::ptr_eq(
                &formatted,
                view.documents[0].formatted.as_ref().unwrap()
            ));
        });

        let original_code_view = app.read(|ctx| view.as_ref(ctx).child_view_ids(ctx)[0]);
        view.update(&mut app, |view, ctx| {
            view.sync_action(
                create_action(
                    "create-1",
                    [(
                        "pomodoro-app-spec.md",
                        "# Overview\n\n```rust\nfn updated() {}\n```",
                    )],
                ),
                true,
                ctx,
            );
        });
        app.read(|ctx| {
            let code_view = view
                .as_ref(ctx)
                .code_views
                .values()
                .next()
                .expect("updated plan keeps code child");
            assert_eq!(view.as_ref(ctx).child_view_ids(ctx)[0], original_code_view);
            let (code_lines, _) = render_code(code_view.as_ref(ctx), ctx);
            assert!(code_lines.iter().any(|line| line.contains("fn updated()")));
        });
    });
}

#[test]
fn plan_body_has_left_padding_without_right_padding() {
    App::test((), |mut app| async move {
        let content = format!("{}xyz", "word ".repeat(15));
        assert_eq!(content.len(), 78);
        let action = create_action("create-1", [("Plan", content.as_str())]);
        let (view, _) = test_plan_view(&mut app, action, true);

        app.read(|ctx| {
            let (lines, buffer) = render(view.as_ref(ctx), 80, ctx);
            assert_eq!(lines[2], format!("  {content}"));
            assert_eq!(buffer[(79, 2)].symbol(), "z");
        });
    });
}

#[test]
fn finalized_single_create_keeps_plan_name_in_header() {
    App::test((), |mut app| async move {
        let action = create_action("create-1", [("pomodoro-app-spec.md", "streamed content")]);
        let (view, action_model) = test_plan_view(&mut app, action.clone(), true);
        action_model.update(&mut app, |model, ctx| {
            model.apply_finished_action_result(
                AIConversationId::new(),
                action_result(
                    &action,
                    AIAgentActionResultType::CreateDocuments(CreateDocumentsResult::Success {
                        created_documents: vec![document_context(
                            AIDocumentId::new(),
                            "final content",
                        )],
                    }),
                ),
                ctx,
            );
        });

        app.read(|ctx| {
            let (lines, _) = render(view.as_ref(ctx), 60, ctx);
            assert_eq!(lines[0], "✓ Created pomodoro-app-spec.md ▾");
        });
    });
}

#[test]
fn finalized_create_replaces_streamed_payload_and_keeps_action_order() {
    App::test((), |mut app| async move {
        let action = create_action(
            "create-1",
            [("First", "streamed first"), ("Second", "streamed second")],
        );
        let (view, action_model) = test_plan_view(&mut app, action.clone(), true);
        let conversation_id = AIConversationId::new();
        let first_id = AIDocumentId::new();
        let second_id = AIDocumentId::new();
        action_model.update(&mut app, |model, ctx| {
            model.apply_finished_action_result(
                conversation_id,
                action_result(
                    &action,
                    AIAgentActionResultType::CreateDocuments(CreateDocumentsResult::Success {
                        created_documents: vec![
                            document_context(first_id, "final first"),
                            document_context(second_id, "final second"),
                        ],
                    }),
                ),
                ctx,
            );
        });

        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert_eq!(
                view.presentation
                    .documents
                    .iter()
                    .map(|document| (document.title.as_str(), document.content.as_str()))
                    .collect::<Vec<_>>(),
                vec![("First", "final first"), ("Second", "final second")]
            );
            let (lines, _) = render(view, 60, ctx);
            assert_eq!(lines[0], "✓ Created 2 documents ▾");
            let positions = ["First", "final first", "Second", "final second"].map(|needle| {
                lines
                    .iter()
                    .position(|line| line.trim() == needle)
                    .unwrap_or_else(|| panic!("missing {needle:?} in {lines:?}"))
            });
            assert!(positions.is_sorted());
            let joined = lines.join("\n");
            assert!(!joined.contains("streamed"));
        });
    });
}

#[test]
fn finalized_edit_uses_full_result_content() {
    App::test((), |mut app| async move {
        let document_id = AIDocumentId::new();
        let action = edit_action("edit-1", document_id);
        let (view, action_model) = test_plan_view(&mut app, action.clone(), false);
        app.read(|ctx| assert!(!view.as_ref(ctx).renders_rich_body()));

        action_model.update(&mut app, |model, ctx| {
            model.apply_finished_action_result(
                AIConversationId::new(),
                action_result(
                    &action,
                    AIAgentActionResultType::EditDocuments(EditDocumentsResult::Success {
                        updated_documents: vec![document_context(
                            document_id,
                            "# Updated\n\nFinal body\n\n| Key | Value |\n| --- | --- |\n| Mode | Focus |",
                        )],
                    }),
                ),
                ctx,
            );
        });

        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert!(view.presentation.documents[0]
                .content
                .contains("| Mode | Focus |"));
            let (lines, _) = render(view, 50, ctx);
            assert_eq!(lines[0], "✓ Updated plan ▾");
            assert!(lines.iter().any(|line| line.trim() == "Updated"));
            assert!(lines.iter().any(|line| line.trim() == "Final body"));
            let joined = lines.join("\n");
            assert!(joined.contains("Key"));
            assert!(joined.contains("Value"));
            assert!(joined.contains("Mode"));
            assert!(joined.contains("Focus"));
            assert!(!joined.contains("| --- |"));
        });
    });
}

#[test]
fn collapse_persists_across_payload_updates_and_invalidates_layout() {
    App::test((), |mut app| async move {
        let initial = create_action("create-1", [("Plan", "first body")]);
        let (view, _) = test_plan_view(&mut app, initial, true);
        let invalidations = Rc::new(std::cell::Cell::new(0));
        let invalidations_for_subscription = invalidations.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&view, move |_, event, _| match event {
                TuiPlanViewEvent::LayoutChanged => {
                    invalidations_for_subscription.set(invalidations_for_subscription.get() + 1);
                }
            });
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(&TuiPlanViewAction::SetCollapsed(true), ctx);
            view.sync_action(
                create_action("create-1", [("Plan", "second body")]),
                true,
                ctx,
            );
        });

        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert!(view.collapsed);
            assert_eq!(view.presentation.documents[0].content, "second body");
            let (lines, _) = render(view, 40, ctx);
            assert_eq!(lines, vec!["○ Creating Plan ▸"]);
        });
        assert!(invalidations.get() >= 2);
    });
}

#[test]
fn cancelled_create_discards_streamed_body_and_code_children() {
    App::test((), |mut app| async move {
        let action = create_action("create-1", [("Plan", "```rust\nfn streamed() {}\n```")]);
        let (view, action_model) = test_plan_view(&mut app, action.clone(), true);
        app.read(|ctx| assert_eq!(view.as_ref(ctx).code_views.len(), 1));

        action_model.update(&mut app, |model, ctx| {
            model.apply_finished_action_result(
                AIConversationId::new(),
                action_result(
                    &action,
                    AIAgentActionResultType::CreateDocuments(CreateDocumentsResult::Cancelled),
                ),
                ctx,
            );
        });

        app.read(|ctx| {
            let view = view.as_ref(ctx);
            assert!(!view.renders_rich_body());
            assert!(view.code_views.is_empty());
        });
    });
}

fn test_plan_view(
    app: &mut App,
    action: AIAgentAction,
    output_streaming: bool,
) -> (
    ViewHandle<TuiPlanView>,
    warpui_core::ModelHandle<warp::tui_export::BlocklistAIActionModel>,
) {
    app.add_singleton_model(|_| Appearance::mock());
    let action_model = add_test_action_model(app);
    let action_model_for_view = action_model.clone();
    let view = app.update(|ctx| {
        let (window_id, _) = ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| TestHostView,
        );
        ctx.add_typed_action_tui_view(window_id, |ctx| {
            TuiPlanView::new(action, output_streaming, &action_model_for_view, ctx)
        })
    });
    (view, action_model)
}

fn create_action<const N: usize>(id: &str, documents: [(&str, &str); N]) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task-1".to_owned()),
        action: AIAgentActionType::CreateDocuments(CreateDocumentsRequest {
            documents: documents
                .into_iter()
                .map(|(title, content)| DocumentToCreate {
                    content: content.to_owned(),
                    title: title.to_owned(),
                })
                .collect(),
        }),
        requires_result: true,
    }
}

fn edit_action(id: &str, document_id: AIDocumentId) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task-1".to_owned()),
        action: AIAgentActionType::EditDocuments(EditDocumentsRequest {
            diffs: vec![DocumentDiff {
                document_id,
                search: "old".to_owned(),
                replace: "new".to_owned(),
            }],
        }),
        requires_result: true,
    }
}

fn action_result(action: &AIAgentAction, result: AIAgentActionResultType) -> AIAgentActionResult {
    AIAgentActionResult {
        id: action.id.clone(),
        task_id: action.task_id.clone(),
        result,
    }
}

fn document_context(document_id: AIDocumentId, content: &str) -> DocumentContext {
    DocumentContext {
        document_id,
        document_version: AIDocumentVersion::default(),
        content: content.to_owned(),
        line_ranges: Vec::new(),
    }
}

fn render(
    view: &TuiPlanView,
    width: u16,
    app: &warpui_core::AppContext,
) -> (Vec<String>, warpui_core::elements::tui::TuiBuffer) {
    let mut presenter = TuiPresenter::new();
    let frame = presenter.present_element(view.render(app), TuiRect::new(0, 0, width, 40), app);
    let mut lines = frame
        .buffer
        .to_lines()
        .into_iter()
        .map(|line| line.trim_end().to_owned())
        .collect::<Vec<_>>();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    (lines, frame.buffer)
}

fn render_code(
    view: &crate::tui_code_block_view::TuiCodeBlockView,
    app: &warpui_core::AppContext,
) -> (Vec<String>, warpui_core::elements::tui::TuiBuffer) {
    let mut presenter = TuiPresenter::new();
    let frame = presenter.present_element(view.render(app), TuiRect::new(0, 0, 40, 5), app);
    (
        frame
            .buffer
            .to_lines()
            .into_iter()
            .map(|line| line.trim_end().to_owned())
            .collect(),
        frame.buffer,
    )
}
