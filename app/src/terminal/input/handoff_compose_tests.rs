use warpui::App;

use super::HandoffComposeState;
use crate::ai::ambient_agents::telemetry::HandoffEntryPoint;
use crate::ai::blocklist::handoff::{HandoffLaunchAttachments, PendingCloudLaunch};
use crate::ai::blocklist::{PendingAttachment, PendingFile};
use crate::ai::cloud_environments::CloudSelectorChoice;
use crate::server::ids::{ClientId, ServerId, SyncId};
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

#[test]
fn restoring_failed_factory_handoff_reinstates_draft_and_requires_replacement() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let choice = CloudSelectorChoice::Factory {
            uid: "factory-12".to_owned(),
            environment_uid: SyncId::ServerId(ServerId::from(12)),
            foreman_agent_uid: "foreman-12".to_owned(),
        };
        let attachment = PendingAttachment::File(PendingFile {
            file_name: "context.txt".to_owned(),
            file_path: std::path::PathBuf::from("/tmp/context.txt"),
            mime_type: "text/plain".to_owned(),
        });
        terminal.update(&mut app, |view, ctx| {
            view.input().update(ctx, |input, ctx| {
                input.restore_cloud_handoff_draft(
                    PendingCloudLaunch {
                        prompt: "continue".to_owned(),
                        attachments: HandoffLaunchAttachments {
                            request_attachments: vec![],
                            display_attachments: vec![attachment],
                        },
                    },
                    Some(choice.environment_id()),
                    Some(choice.clone()),
                    true,
                    ctx,
                );
            });
        });
        terminal.read(&app, |view, ctx| {
            let input = view.input().as_ref(ctx);
            assert_eq!(input.editor().as_ref(ctx).buffer_text(ctx), "continue");
            assert_eq!(
                input
                    .ai_context_model()
                    .as_ref(ctx)
                    .pending_attachments()
                    .len(),
                1
            );
            let state = input.handoff_compose_state.as_ref(ctx);
            assert!(state.is_active());
            assert!(state.selection_invalidated());
            assert!(state.selected_choice().is_none());
            assert!(state.selected_environment_id().is_none());
        });
    });
}

#[test]
fn invalidated_factory_handoff_requires_explicit_replacement() {
    App::test((), |mut app| async move {
        let state = app.add_model(|_| HandoffComposeState::default());
        let backing = SyncId::ServerId(ServerId::from(12));
        let ordinary = SyncId::ServerId(ServerId::from(13));
        state.update(&mut app, |state, ctx| {
            state.activate(HandoffEntryPoint::Ampersand, ctx);
            state.set_choice(
                CloudSelectorChoice::Factory {
                    uid: "factory-12".to_owned(),
                    environment_uid: backing,
                    foreman_agent_uid: "foreman-12".to_owned(),
                },
                ctx,
            );
            state.invalidate_choice(ctx);
            state.set_environment_id(Some(ordinary), false, ctx);
            state.ensure_default_environment_id(ordinary, ctx);
        });
        state.read(&app, |state, _| {
            assert!(state.selection_invalidated());
            assert!(state.selected_choice().is_none());
            assert!(state.selected_environment_id().is_none());
        });
        state.update(&mut app, |state, ctx| {
            state.set_environment_id(Some(ordinary), true, ctx);
        });
        state.read(&app, |state, _| {
            assert!(!state.selection_invalidated());
            assert_eq!(
                state.selected_choice(),
                Some(&CloudSelectorChoice::Environment(ordinary))
            );
        });
    });
}

#[test]
fn preserves_explicit_environment_selection() {
    App::test((), |mut app| async move {
        let state = app.add_model(|_| HandoffComposeState::default());
        let default_environment_id = SyncId::ClientId(ClientId::new());
        let explicit_environment_id = SyncId::ClientId(ClientId::new());

        state.update(&mut app, |state, ctx| {
            state.activate(HandoffEntryPoint::Ampersand, ctx);
            state.ensure_default_environment_id(default_environment_id, ctx);
        });
        state.read(&app, |state, _| {
            assert_eq!(
                state.selected_environment_id(),
                Some(&default_environment_id)
            );
        });

        // Explicit selection should stick even when ensure_default tries to overwrite.
        state.update(&mut app, |state, ctx| {
            state.set_environment_id(Some(explicit_environment_id), true, ctx);
            state.ensure_default_environment_id(default_environment_id, ctx);
        });
        state.read(&app, |state, _| {
            assert_eq!(
                state.selected_environment_id(),
                Some(&explicit_environment_id)
            );
        });
    });
}
