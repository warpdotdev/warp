use std::sync::Arc;

use parking_lot::FairMutex;

use warpui::App;

use crate::terminal::shared_session::MAX_BYTES_SHAREABLE;
use crate::terminal::TerminalModel;
use crate::{
    terminal::shared_session::{SharedSessionActionSource, SharedSessionScrollbackType},
    test_util::{add_window_with_terminal, terminal::initialize_app_for_terminal_view},
};

use super::Body;

#[test]
fn test_open_modal_from_non_block() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);

        let window_id = app.read(|ctx| terminal_view.window_id(ctx));
        let share_session_modal = app.add_typed_action_view(window_id, Body::new);
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));

        // Share from the tab.
        let terminal_model_clone = terminal_model.clone();
        share_session_modal.update(&mut app, |share_session_modal, ctx| {
            let open_source = SharedSessionActionSource::Tab;
            share_session_modal.open(open_source, terminal_model_clone, terminal_view.id(), ctx);
        });

        // Options should be no scrollback and from start. Both enabled.
        share_session_modal.read(&app, |body, _ctx| {
            assert_eq!(body.radio_button_mouse_states.items.len(), 2);
            assert_eq!(
                body.radio_button_mouse_states.items[0].scrollback_type,
                SharedSessionScrollbackType::None
            );
            assert!(!body.radio_button_mouse_states.items[0].is_disabled);
            assert_eq!(
                body.radio_button_mouse_states.items[1].scrollback_type,
                SharedSessionScrollbackType::All
            );
            assert!(!body.radio_button_mouse_states.items[1].is_disabled);
        });
    })
}

#[test]
fn test_open_modal_from_block() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);

        let window_id = app.read(|ctx| terminal_view.window_id(ctx));
        let share_session_modal = app.add_typed_action_view(window_id, Body::new);
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));

        // Add a block that is under the limit.
        // `serde_json::to_vec` roughly triples the size of the SerializedBlock output, which is why we divide by 4.
        terminal_model
            .lock()
            .simulate_block("ls", "a".repeat(MAX_BYTES_SHAREABLE / 4).as_str());

        // Share from the very large block we just completed.
        let block_index = terminal_model.lock().block_list().active_block_index() - 1.into();
        let terminal_model_clone = terminal_model.clone();
        share_session_modal.update(&mut app, |share_session_modal, ctx| {
            let open_source = SharedSessionActionSource::BlocklistContextMenu {
                block_index: Some(block_index),
            };
            share_session_modal.open(open_source, terminal_model_clone, terminal_view.id(), ctx);
        });

        // Options should be from block, no scrollback, and from start. All enabled.
        share_session_modal.read(&app, |body, _ctx| {
            assert_eq!(body.radio_button_mouse_states.items.len(), 3);
            assert_eq!(
                body.radio_button_mouse_states.items[0].scrollback_type,
                SharedSessionScrollbackType::FromBlock { block_index },
            );
            assert!(!body.radio_button_mouse_states.items[0].is_disabled);
            assert_eq!(
                body.radio_button_mouse_states.items[1].scrollback_type,
                SharedSessionScrollbackType::None
            );
            assert!(!body.radio_button_mouse_states.items[1].is_disabled);
            assert_eq!(
                body.radio_button_mouse_states.items[2].scrollback_type,
                SharedSessionScrollbackType::All
            );
            assert!(!body.radio_button_mouse_states.items[2].is_disabled);
        });
    })
}

#[test]
fn test_open_modal_from_non_block_disabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);

        let window_id = app.read(|ctx| terminal_view.window_id(ctx));
        let share_session_modal = app.add_typed_action_view(window_id, Body::new);
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));

        // Add a block that is under the limit.
        // `serde_json::to_vec` roughly triples the size of the SerializedBlock output, which is why we divide by 4.
        terminal_model
            .lock()
            .simulate_block("ls", "a".repeat(MAX_BYTES_SHAREABLE / 4).as_str());

        // Add another block that puts us over the sharing limit for the whole session.
        // `serde_json::to_vec` roughly triples the size of the SerializedBlock output, which is why we divide by 4. This plus the earlier block puts us over the limit.
        terminal_model
            .lock()
            .simulate_block("ls", "a".repeat(MAX_BYTES_SHAREABLE / 4).as_str());

        // Share from the tab.
        let terminal_model_clone = terminal_model.clone();
        share_session_modal.update(&mut app, |share_session_modal, ctx| {
            let open_source = SharedSessionActionSource::Tab;
            share_session_modal.open(open_source, terminal_model_clone, terminal_view.id(), ctx);
        });

        // Options should be no scrollback and from start. From start is disabled.
        share_session_modal.read(&app, |body, _ctx| {
            assert_eq!(body.radio_button_mouse_states.items.len(), 2);
            assert_eq!(
                body.radio_button_mouse_states.items[0].scrollback_type,
                SharedSessionScrollbackType::None
            );
            assert!(!body.radio_button_mouse_states.items[0].is_disabled);
            assert_eq!(
                body.radio_button_mouse_states.items[1].scrollback_type,
                SharedSessionScrollbackType::All
            );
            assert!(body.radio_button_mouse_states.items[1].is_disabled);
        });
    })
}

#[test]
fn test_open_modal_from_block_disabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);

        let window_id = app.read(|ctx| terminal_view.window_id(ctx));
        let share_session_modal = app.add_typed_action_view(window_id, Body::new);
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));

        // Add a block that is under the limit.
        // `serde_json::to_vec` roughly triples the size of the SerializedBlock output, which is why we divide by 4.
        terminal_model
            .lock()
            .simulate_block("ls", "a".repeat(MAX_BYTES_SHAREABLE / 4).as_str());
        let mut block_index = terminal_model.lock().block_list().active_block_index() - 1.into();

        // Add another block that puts us over the sharing limit for the whole session.
        // `serde_json::to_vec` roughly triples the size of the SerializedBlock output, which is why we divide by 4. This plus the earlier block puts us over the limit.
        terminal_model
            .lock()
            .simulate_block("ls", "a".repeat(MAX_BYTES_SHAREABLE / 4).as_str());

        // Open modal from the first very large block.
        let terminal_model_clone = terminal_model.clone();
        share_session_modal.update(&mut app, |share_session_modal, ctx| {
            let open_source = SharedSessionActionSource::BlocklistContextMenu {
                block_index: Some(block_index),
            };
            share_session_modal.open(open_source, terminal_model_clone, terminal_view.id(), ctx);
        });

        // From block and from start of the session are disabled because they are over the limit. No scrollback is enabled.
        share_session_modal.read(&app, |body, _ctx| {
            assert_eq!(body.radio_button_mouse_states.items.len(), 3);
            assert_eq!(
                body.radio_button_mouse_states.items[0].scrollback_type,
                SharedSessionScrollbackType::FromBlock { block_index }
            );
            assert!(body.radio_button_mouse_states.items[0].is_disabled);
            assert_eq!(
                body.radio_button_mouse_states.items[1].scrollback_type,
                SharedSessionScrollbackType::None
            );
            assert!(!body.radio_button_mouse_states.items[1].is_disabled);
            assert_eq!(
                body.radio_button_mouse_states.items[2].scrollback_type,
                SharedSessionScrollbackType::All
            );
            assert!(body.radio_button_mouse_states.items[2].is_disabled);
        });

        // Open modal from the newly finished block, which excludes the very large first block we created.
        block_index = terminal_model.lock().block_list().active_block_index() - 1.into();
        let terminal_model_clone = terminal_model.clone();
        share_session_modal.update(&mut app, |share_session_modal, ctx| {
            let open_source = SharedSessionActionSource::BlocklistContextMenu {
                block_index: Some(block_index),
            };
            share_session_modal.open(open_source, terminal_model_clone, terminal_view.id(), ctx);
        });

        // From block and no scrollback are enabled, but from start of session is disabled.
        share_session_modal.read(&app, |body, _ctx| {
            assert_eq!(body.radio_button_mouse_states.items.len(), 3);
            assert_eq!(
                body.radio_button_mouse_states.items[0].scrollback_type,
                SharedSessionScrollbackType::FromBlock { block_index }
            );
            assert!(!body.radio_button_mouse_states.items[0].is_disabled);
            assert_eq!(
                body.radio_button_mouse_states.items[1].scrollback_type,
                SharedSessionScrollbackType::None
            );
            assert!(!body.radio_button_mouse_states.items[1].is_disabled);
            assert_eq!(
                body.radio_button_mouse_states.items[2].scrollback_type,
                SharedSessionScrollbackType::All
            );
            assert!(body.radio_button_mouse_states.items[2].is_disabled);
        });
    });
}

#[test]
fn test_open_modal_from_long_running_block() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);
        let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let window_id = app.read(|ctx| terminal_view.window_id(ctx));
        let share_session_modal = app.add_typed_action_view(window_id, Body::new);

        // Add a long-running block that is under the limit.
        // `serde_json::to_vec` roughly triples the size of the SerializedBlock output, which is why we divide by 4.
        terminal_model
            .lock()
            .simulate_long_running_block("ls", "a".repeat(MAX_BYTES_SHAREABLE / 4).as_str());
        assert_eq!(terminal_model.lock().block_list().blocks().len(), 2);
        assert!(terminal_model
            .lock()
            .block_list()
            .active_block()
            .is_executing());

        // Open the share modal.
        let terminal_model_clone = terminal_model.clone();
        share_session_modal.update(&mut app, |share_session_modal, ctx| {
            let open_source = SharedSessionActionSource::Tab;
            share_session_modal.open(open_source, terminal_model_clone, terminal_view.id(), ctx);
        });

        // Options should be no scrollback and from start. Both are enabled.
        share_session_modal.read(&app, |body, _ctx| {
            assert_eq!(body.radio_button_mouse_states.items.len(), 2);
            assert_eq!(
                body.radio_button_mouse_states.items[0].scrollback_type,
                SharedSessionScrollbackType::None
            );
            assert!(!body.radio_button_mouse_states.items[0].is_disabled);
            assert_eq!(
                body.radio_button_mouse_states.items[1].scrollback_type,
                SharedSessionScrollbackType::All
            );
            assert!(!body.radio_button_mouse_states.items[1].is_disabled);
        });

        // Add more output to block so that it's over the limit.
        terminal_model
            .lock()
            .process_bytes("a".repeat(MAX_BYTES_SHAREABLE / 4).as_str());
        assert_eq!(terminal_model.lock().block_list().blocks().len(), 2);
        assert!(terminal_model
            .lock()
            .block_list()
            .active_block()
            .is_executing());

        // Re-open the modal to refresh the options.
        let terminal_model_clone = terminal_model.clone();
        share_session_modal.update(&mut app, |share_session_modal, ctx| {
            let open_source = SharedSessionActionSource::Tab;
            share_session_modal.open(open_source, terminal_model_clone, terminal_view.id(), ctx);
        });

        // Options should be no scrollback and from start. Both are disabled.
        share_session_modal.read(&app, |body, _ctx| {
            assert_eq!(body.radio_button_mouse_states.items.len(), 2);
            assert_eq!(
                body.radio_button_mouse_states.items[0].scrollback_type,
                SharedSessionScrollbackType::None
            );
            assert!(body.radio_button_mouse_states.items[0].is_disabled);
            assert_eq!(
                body.radio_button_mouse_states.items[1].scrollback_type,
                SharedSessionScrollbackType::All
            );
            assert!(body.radio_button_mouse_states.items[1].is_disabled);
        });
    })
}
