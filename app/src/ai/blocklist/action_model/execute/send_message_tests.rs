use super::*;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use warpui::{App, EntityId};

#[test]
fn sender_run_id_and_task_id_for_send_falls_back_to_ambient_task_id() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let ambient_task_id = "11111111-1111-1111-1111-111111111111"
            .parse()
            .expect("valid ambient task id");

        let (sender_run_id, task_id, task_resolution) = app.read(|ctx| {
            sender_run_id_and_task_id_for_send(conversation_id, Some(ambient_task_id), ctx)
        });

        assert_eq!(sender_run_id, ambient_task_id.to_string());
        assert_eq!(task_id, Some(ambient_task_id));
        assert_eq!(
            task_resolution,
            SendMessageTaskResolution::AmbientTaskFallback
        );
    });
}

#[test]
fn sender_run_id_and_task_id_for_send_prefers_conversation_task_id() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let conversation_task_id = "22222222-2222-2222-2222-222222222222"
            .parse()
            .expect("valid conversation task id");
        let ambient_task_id = "33333333-3333-3333-3333-333333333333"
            .parse()
            .expect("valid ambient task id");

        history_model.update(&mut app, |history_model, _| {
            history_model
                .conversation_mut(&conversation_id)
                .expect("conversation exists")
                .set_task_id(conversation_task_id);
        });

        let (sender_run_id, task_id, task_resolution) = app.read(|ctx| {
            sender_run_id_and_task_id_for_send(conversation_id, Some(ambient_task_id), ctx)
        });

        assert_eq!(sender_run_id, conversation_task_id.to_string());
        assert_eq!(task_id, Some(conversation_task_id));
        assert_eq!(task_resolution, SendMessageTaskResolution::ConversationTask);
    });
}
