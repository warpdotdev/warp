use chrono::Local;
use warpui::integration::{AssertionCallback, TestStep};
use warpui::{App, SingletonEntity, TypedActionView, ViewHandle, WindowId, async_assert};

use crate::ai::ai_document_view::{AIDocumentAction, AIDocumentView};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentModel, AIDocumentVersion};
use crate::integration_testing::view_getters::{
    pane_group_view, single_terminal_view_for_tab, workspace_view,
};
use crate::workspace::WorkspaceAction;

pub fn create_and_open_ai_document(title: &'static str, markdown: &'static str) -> TestStep {
    TestStep::new("Create and open AI document").with_action(move |app, window_id, _| {
        let terminal_view_id = single_terminal_view_for_tab(app, window_id, 0).id();
        let document_id = app.update(|ctx| {
            let conversation_id = BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
                let conversation_id =
                    model.start_new_conversation(terminal_view_id, false, false, false, ctx);
                model.set_active_conversation_id(conversation_id, terminal_view_id, ctx);
                conversation_id
            });

            AIDocumentModel::handle(ctx).update(ctx, |model, ctx| {
                model.create_document(title, markdown, conversation_id, None, ctx)
            })
        });

        let workspace = workspace_view(app, window_id);
        workspace.update(app, |workspace, ctx| {
            workspace.handle_action(
                &WorkspaceAction::OpenAIDocumentPane {
                    document_id,
                    document_version: AIDocumentVersion::default(),
                },
                ctx,
            );
        });

        let pane_group = pane_group_view(app, window_id, 0);
        pane_group.update(app, |pane_group, ctx| {
            let pane_id = pane_group
                .ai_document_panes()
                .next()
                .expect("AI document pane should be open");
            let pane_configuration = pane_group
                .pane_by_id(pane_id)
                .expect("AI document pane should exist")
                .pane_configuration();
            pane_configuration.update(ctx, |pane_configuration, ctx| {
                pane_configuration.refresh_pane_header_overflow_menu_items(ctx);
            });
        });
    })
}

pub fn ai_document_overflow_button_position_id(app: &mut App, window_id: WindowId) -> String {
    let pane_group = pane_group_view(app, window_id, 0);
    pane_group.read(app, |pane_group, _| {
        let pane_id = pane_group
            .ai_document_panes()
            .next()
            .expect("AI document pane should be open");
        let pane_configuration_id = pane_group
            .pane_by_id(pane_id)
            .expect("AI document pane should exist")
            .pane_configuration()
            .id();
        format!("pane_header_overflow_button:{pane_configuration_id}")
    })
}

pub fn assert_ai_document_overflow_button_position_exists() -> AssertionCallback {
    Box::new(|app, window_id| {
        let position_id = ai_document_overflow_button_position_id(app, window_id);
        let presenter = app.presenter(window_id).expect("presenter should exist");
        let presenter = presenter.borrow();
        async_assert!(
            presenter
                .position_cache()
                .get_position(position_id)
                .is_some()
        )
    })
}

/// Restore two document revisions the way conversation restore does, then open the current version.
pub fn restore_and_open_ai_document(
    title: &'static str,
    initial_markdown: &'static str,
    edited_markdown: &'static str,
) -> TestStep {
    TestStep::new("Restore and open AI document").with_action(move |app, window_id, _| {
        let terminal_view_id = single_terminal_view_for_tab(app, window_id, 0).id();
        let (document_id, document_version) = app.update(|ctx| {
            let conversation_id = BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
                let conversation_id =
                    model.start_new_conversation(terminal_view_id, false, false, false, ctx);
                model.set_active_conversation_id(conversation_id, terminal_view_id, ctx);
                conversation_id
            });

            AIDocumentModel::handle(ctx).update(ctx, |model, ctx| {
                let document_id = AIDocumentId::new();
                model.restore_document(
                    document_id,
                    conversation_id,
                    title,
                    initial_markdown,
                    Local::now(),
                    ctx,
                );
                let document_version = model
                    .restore_document_edit(&document_id, edited_markdown, Local::now(), ctx)
                    .expect("restored edit should create a second version");
                (document_id, document_version)
            })
        });

        let workspace = workspace_view(app, window_id);
        workspace.update(app, |workspace, ctx| {
            workspace.handle_action(
                &WorkspaceAction::OpenAIDocumentPane {
                    document_id,
                    document_version,
                },
                ctx,
            );
        });

        let pane_group = pane_group_view(app, window_id, 0);
        pane_group.update(app, |pane_group, ctx| {
            let pane_id = pane_group
                .ai_document_panes()
                .next()
                .expect("AI document pane should be open");
            let pane_configuration = pane_group
                .pane_by_id(pane_id)
                .expect("AI document pane should exist")
                .pane_configuration();
            pane_configuration.update(ctx, |pane_configuration, ctx| {
                pane_configuration.refresh_pane_header_overflow_menu_items(ctx);
            });
        });
    })
}

fn viewed_ai_document(app: &App, window_id: WindowId) -> ViewHandle<AIDocumentView> {
    let views = app
        .views_of_type::<AIDocumentView>(window_id)
        .expect("AIDocumentView should exist");
    assert_eq!(views.len(), 1, "expected a single AI document view");
    views.into_iter().next().unwrap()
}

pub fn assert_viewed_ai_document_has_code_and_mermaid_controls() -> AssertionCallback {
    Box::new(|app, window_id| {
        let view = viewed_ai_document(app, window_id);
        let (document_id, version) = view.read(app, |view, _| {
            (*view.document_id(), view.document_version())
        });
        let editor = AIDocumentModel::handle(app).read(app, |model, _ctx| {
            model
                .get_document(&document_id, version)
                .expect("viewed document should exist")
                .get_editor()
        });
        let (shell_count, mermaid_count, mermaid_offsets) = editor.read(app, |editor, ctx| {
            (
                editor.nested_shell_command_count(ctx),
                editor.nested_rendered_mermaid_command_count(ctx),
                editor
                    .render_state()
                    .as_ref(ctx)
                    .layout_options()
                    .mermaid_render_offsets
                    .len(),
            )
        });
        async_assert!(shell_count == 1 && mermaid_count == 1 && mermaid_offsets == 1)
    })
}

pub fn select_viewed_ai_document_version(version: AIDocumentVersion) -> TestStep {
    TestStep::new("Select AI document version").with_action(move |app, window_id, _| {
        let view = viewed_ai_document(app, window_id);
        view.update(app, |view, ctx| {
            view.handle_action(&AIDocumentAction::SelectVersion(version), ctx);
        });
    })
}

pub fn select_initial_ai_document_version() -> TestStep {
    select_viewed_ai_document_version(AIDocumentVersion::default())
}
