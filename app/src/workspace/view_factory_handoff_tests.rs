use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use warp_core::features::FeatureFlag;
use warpui::{App, SingletonEntity};

use super::tests::{initialize_app, mock_workspace};
use super::*;
use crate::ai::blocklist::{PendingAttachment, PendingFile};
use crate::ai::cloud_environments::CloudSelectorChoice;
use crate::ai::orchestration::{CloudAgentStartupFailure, CloudAgentStartupIssue};
use crate::server::ids::ServerId;
use crate::server::server_api::ai::{SpawnAgentRequest, UserQueryMode};

fn assert_failed_factory_handoff_restores_source(server_rejection: bool) {
    App::test((), |mut app| async move {
        let _factory_selector = FeatureFlag::CloudModeFactorySelector.override_enabled(true);
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        let source = workspace.read(&app, |workspace, ctx| {
            workspace
                .active_tab_pane_group()
                .as_ref(ctx)
                .focused_session_view(ctx)
                .expect("source terminal")
        });
        let (target_view, target_model) = source.update(&mut app, |view, ctx| {
            view.start_cloud_mode(None, ctx)
                .expect("provisional target")
        });
        let target = Arc::new(Mutex::new(Some((source.clone(), target_model))));
        let message = if server_rejection {
            "Factory configuration rejected by server."
        } else {
            "Factory is no longer available."
        };
        let choice = CloudSelectorChoice::Factory {
            uid: "factory-12".to_owned(),
            environment_uid: SyncId::ServerId(ServerId::from(12)),
            foreman_agent_uid: "foreman-12".to_owned(),
        };
        let request = server_rejection.then(|| SpawnAgentRequest {
            prompt: Some("continue".to_owned()),
            mode: UserQueryMode::Normal,
            config: None,
            title: None,
            team: Some(true),
            agent_identity_uid: Some("foreman-12".to_owned()),
            skill: None,
            attachments: vec![],
            interactive: Some(true),
            parent_run_id: None,
            runtime_skills: vec![],
            referenced_attachments: vec![],
            conversation_id: None,
            initial_snapshot_token: None,
            snapshot_disabled: None,
            orchestration_handoff: None,
        });
        let toasts = Rc::new(RefCell::new(Vec::new()));
        workspace.update(&mut app, |_, ctx| {
            let observed_toasts = toasts.clone();
            let toast_stack = WorkspaceToastStack::handle(ctx);
            ctx.subscribe_to_model(&toast_stack, move |_, _, event, _| {
                if let WorkspaceToastStackEvent::AddEphemeralToast { toast, .. } = event {
                    observed_toasts
                        .borrow_mut()
                        .push(toast.main_text().to_owned());
                }
            });
        });
        workspace.update(&mut app, |workspace, ctx| {
            assert_eq!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .active_session_view(ctx)
                    .expect("active target")
                    .id(),
                target_view.id()
            );
            workspace.handle_failed_handoff_commit(
                &source,
                target,
                HandoffCommitFailure {
                    issue: CloudAgentStartupIssue::Failed(CloudAgentStartupFailure::Other {
                        message: message.to_owned(),
                    }),
                    request,
                    restoration: Some(HandoffRestoration {
                        prompt: "continue".to_owned(),
                        attachments: vec![PendingAttachment::File(PendingFile {
                            file_name: "context.txt".to_owned(),
                            file_path: std::path::PathBuf::from("/tmp/context.txt"),
                            mime_type: "text/plain".to_owned(),
                        })],
                        environment_id: Some(choice.environment_id()),
                        selected_choice: Some(choice),
                    }),
                    derived_workspace_had_content: None,
                    snapshot_failed: false,
                },
                LocalToCloudHandoffIntent::UserInitiated(HandoffEntryPoint::Ampersand),
                ctx,
            );
        });
        workspace.read(&app, |workspace, ctx| {
            assert_eq!(
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .active_session_view(ctx)
                    .expect("restored source")
                    .id(),
                source.id()
            );
        });
        source.read(&app, |view, ctx| {
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
        });
        assert_eq!(
            toasts.borrow().as_slice(),
            &[format!("{message} Choose a replacement.")]
        );
    });
}

#[test]
fn late_factory_revalidation_failure_restores_source() {
    assert_failed_factory_handoff_restores_source(false);
}

#[test]
fn server_factory_rejection_restores_source() {
    assert_failed_factory_handoff_restores_source(true);
}
