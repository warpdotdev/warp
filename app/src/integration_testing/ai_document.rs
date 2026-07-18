use warpui::integration::{AssertionCallback, TestStep};
use warpui::{async_assert, App, SingletonEntity, TypedActionView, WindowId};

use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::ai::document::ai_document_model::{AIDocumentModel, AIDocumentVersion};
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
        async_assert!(presenter
            .position_cache()
            .get_position(position_id)
            .is_some())
    })
}
