use std::sync::Arc;

use parking_lot::FairMutex;
use warpui::r#async::executor::Background;
use warpui::{App, EntityId};

use super::AgentViewConversationSelection;
use crate::ai::blocklist::agent_view::{
    AgentViewController, AgentViewEntryOrigin, EphemeralMessageModel,
};
use crate::ai::blocklist::{BlocklistAIHistoryModel, ConversationSelection};
use crate::terminal::color::{self, Colors};
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::test_utils::block_size;
use crate::terminal::TerminalModel;
use crate::test_util::settings::initialize_settings_for_tests;

#[test]
fn gui_selection_delegates_to_agent_view() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let terminal_surface_id = EntityId::new();
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::new_for_test(
            block_size(),
            color::List::from(&Colors::default()),
            ChannelEventListener::new_for_test(),
            Arc::new(Background::default()),
            false,
            None,
            false,
            false,
            None,
        )));
        let ephemeral_message_model = app.add_model(|_| EphemeralMessageModel::new());
        let agent_view_controller = app.add_model(|_| {
            AgentViewController::new(terminal_model, terminal_surface_id, ephemeral_message_model)
        });
        let selection = app.add_model(|ctx| {
            Box::new(AgentViewConversationSelection::new(
                terminal_surface_id,
                agent_view_controller.clone(),
                ctx,
            )) as Box<dyn ConversationSelection>
        });
        let conversation_id = history.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_surface_id, false, false, false, ctx)
        });

        selection.update(&mut app, |selection, ctx| {
            selection.select_existing_conversation(
                conversation_id,
                AgentViewEntryOrigin::ConversationSelector,
                ctx,
            );
        });

        selection.read(&app, |selection, ctx| {
            assert_eq!(
                selection.selected_conversation_id(ctx),
                Some(conversation_id)
            );
        });
        agent_view_controller.read(&app, |controller, _| {
            assert!(controller.is_active());
        });
    });
}
