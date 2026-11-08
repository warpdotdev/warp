use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use instant::Instant;
use warpui::r#async::Timer;
use warpui::{App, SingletonEntity, TypedActionView};

use super::{InviteTarget, TemporaryInviteAction, TemporaryInviteModal, invite_expired};
use crate::auth::AuthStateProvider;
use crate::server::ids::ServerId;
use crate::server::server_api::team::{MockTeamClient, TemporaryInviteCode};
use crate::workspace::view::tests::{
    initialize_app, initialize_app_with_team_client, mock_workspace,
};

#[test]
fn native_generation_dispatches_once_and_recopy_reuses_the_response() {
    App::test((), |mut app| async move {
        let mut client = MockTeamClient::new();
        client
            .expect_generate_temporary_invite_code()
            .withf(|workspace_uid, team_uid| {
                *workspace_uid == ServerId::from(123).into()
                    && *team_uid == Some(ServerId::from(456))
            })
            .times(1)
            .return_once(|_, _| {
                Ok(TemporaryInviteCode {
                    invite_uid: "generated-invite".into(),
                    invite_link: "https://example.com/team/generated".into(),
                    expires_at: Utc.with_ymd_and_hms(2100, 1, 1, 0, 0, 0).unwrap(),
                })
            });
        initialize_app_with_team_client(&mut app, Arc::new(client));
        let workspace = mock_workspace(&mut app);
        let modal = workspace.update(&mut app, |_, ctx| {
            ctx.add_typed_action_view(TemporaryInviteModal::new)
        });
        modal.update(&mut app, |modal, ctx| {
            let target = InviteTarget {
                workspace_uid: ServerId::from(123).into(),
                team_uid: ServerId::from(456),
                actor_uid: AuthStateProvider::as_ref(ctx).get().user_id().unwrap(),
            };
            modal.open(
                target,
                "Engineering".into(),
                "No domain restrictions".into(),
                ctx,
            );
            assert!(modal.result.is_none());
            modal.handle_action(&TemporaryInviteAction::Copy, ctx);
            modal.handle_action(&TemporaryInviteAction::Copy, ctx);
            assert!(modal.pending);
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while modal.read(&app, |modal, _| modal.pending) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for invite generation"
            );
            Timer::after(Duration::from_millis(10)).await;
        }
        modal.update(&mut app, |modal, ctx| {
            assert_eq!(
                modal.result.as_ref().unwrap().invite_uid,
                "generated-invite"
            );
            assert!(!modal.pending);
            modal.handle_action(&TemporaryInviteAction::Copy, ctx);
            assert_eq!(
                ctx.clipboard().read().plain_text,
                "https://example.com/team/generated"
            );
        });
    });
}

#[test]
fn a_generation_response_after_logout_is_discarded() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        let modal = workspace.update(&mut app, |_, ctx| {
            ctx.add_typed_action_view(TemporaryInviteModal::new)
        });
        modal.update(&mut app, |modal, ctx| {
            let target = InviteTarget {
                workspace_uid: ServerId::from(123).into(),
                team_uid: ServerId::from(456),
                actor_uid: AuthStateProvider::as_ref(ctx).get().user_id().unwrap(),
            };
            modal.open(
                target,
                "Engineering".into(),
                "No domain restrictions".into(),
                ctx,
            );
            modal.pending = true;
            AuthStateProvider::as_ref(ctx).get().set_user(None);
            modal.finish_generation(
                modal.epoch,
                target,
                Ok(TemporaryInviteCode {
                    invite_uid: "late-invite".into(),
                    invite_link: "https://example.com/team/late".into(),
                    expires_at: Utc.with_ymd_and_hms(2100, 1, 1, 0, 0, 0).unwrap(),
                }),
                ctx,
            );

            assert!(modal.target.is_none());
            assert!(modal.result.is_none());
            assert!(!modal.pending);
            assert_eq!(modal.link_editor.as_ref(ctx).buffer_text(ctx), "");
        });
    });
}

#[test]
fn refuses_to_recopy_at_expiry() {
    let expiry = Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap();

    assert!(!invite_expired(
        expiry,
        Utc.with_ymd_and_hms(2026, 11, 8, 11, 59, 59).unwrap()
    ));
    assert!(invite_expired(expiry, expiry));
    assert!(invite_expired(
        expiry,
        Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 1).unwrap()
    ));
}

#[test]
fn a_late_generation_response_cannot_repopulate_a_closed_dialog() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        let modal = workspace.update(&mut app, |_, ctx| {
            ctx.add_typed_action_view(TemporaryInviteModal::new)
        });
        modal.update(&mut app, |modal, ctx| {
            let target = InviteTarget {
                workspace_uid: ServerId::from(123).into(),
                team_uid: ServerId::from(123),
                actor_uid: AuthStateProvider::as_ref(ctx).get().user_id().unwrap(),
            };
            modal.open(
                target,
                "Engineering".into(),
                "No domain restrictions".into(),
                ctx,
            );
            let epoch = modal.epoch;
            modal.pending = true;
            modal.handle_action(&TemporaryInviteAction::Copy, ctx);
            assert!(modal.pending);
            modal.clear(ctx);

            modal.finish_generation(
                epoch,
                target,
                Ok(TemporaryInviteCode {
                    invite_uid: "late-invite".into(),
                    invite_link: "https://example.com/team/late".into(),
                    expires_at: Utc.with_ymd_and_hms(2100, 1, 1, 0, 0, 0).unwrap(),
                }),
                ctx,
            );

            assert!(modal.target.is_none());
            assert!(modal.result.is_none());
            assert!(!modal.pending);
            assert_eq!(modal.link_editor.as_ref(ctx).buffer_text(ctx), "");
        });
    });
}

#[test]
fn issuance_failure_does_not_retain_the_server_error() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        let modal = workspace.update(&mut app, |_, ctx| {
            ctx.add_typed_action_view(TemporaryInviteModal::new)
        });
        modal.update(&mut app, |modal, ctx| {
            let target = InviteTarget {
                workspace_uid: ServerId::from(123).into(),
                team_uid: ServerId::from(123),
                actor_uid: AuthStateProvider::as_ref(ctx).get().user_id().unwrap(),
            };
            modal.open(
                target,
                "Engineering".into(),
                "No domain restrictions".into(),
                ctx,
            );
            modal.pending = true;
            modal.finish_generation(
                modal.epoch,
                target,
                Err(anyhow::anyhow!("sensitive-error-body")),
                ctx,
            );

            assert!(!modal.pending);
            assert!(modal.result.is_none());
            assert!(
                !modal
                    .message
                    .as_ref()
                    .unwrap()
                    .contains("sensitive-error-body")
            );
        });
    });
}

#[test]
fn recopy_reuses_the_dialog_credential_and_close_clears_it() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        let modal = workspace.update(&mut app, |_, ctx| {
            ctx.add_typed_action_view(TemporaryInviteModal::new)
        });
        modal.update(&mut app, |modal, ctx| {
            let target = InviteTarget {
                workspace_uid: ServerId::from(123).into(),
                team_uid: ServerId::from(123),
                actor_uid: AuthStateProvider::as_ref(ctx).get().user_id().unwrap(),
            };
            modal.open(
                target,
                "Engineering".into(),
                "No domain restrictions".into(),
                ctx,
            );
            modal.result = Some(TemporaryInviteCode {
                invite_uid: "test-invite".into(),
                invite_link: "https://example.com/team/test-only-credential".into(),
                expires_at: Utc.with_ymd_and_hms(2100, 1, 1, 0, 0, 0).unwrap(),
            });
            modal.handle_action(&TemporaryInviteAction::Copy, ctx);
            modal.handle_action(&TemporaryInviteAction::Copy, ctx);

            assert_eq!(modal.result.as_ref().unwrap().invite_uid, "test-invite");
            assert!(!modal.pending);
            assert_eq!(
                ctx.clipboard().read().plain_text,
                "https://example.com/team/test-only-credential"
            );

            modal.clear(ctx);
            assert!(modal.result.is_none());
            assert!(modal.target.is_none());
            assert_eq!(modal.link_editor.as_ref(ctx).buffer_text(ctx), "");
        });
    });
}

#[test]
fn expired_result_cannot_be_recopied_or_silently_reissued() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        let modal = workspace.update(&mut app, |_, ctx| {
            ctx.add_typed_action_view(TemporaryInviteModal::new)
        });
        modal.update(&mut app, |modal, ctx| {
            modal.result = Some(TemporaryInviteCode {
                invite_uid: "expired-invite".into(),
                invite_link: "https://example.com/team/expired".into(),
                expires_at: Utc.with_ymd_and_hms(2000, 1, 1, 0, 0, 0).unwrap(),
            });
            modal.copy(ctx);

            assert_eq!(modal.result.as_ref().unwrap().invite_uid, "expired-invite");
            assert!(!modal.pending);
            assert!(modal.message.as_ref().unwrap().contains("expired"));
            assert_ne!(
                ctx.clipboard().read().plain_text,
                "https://example.com/team/expired"
            );
        });
    });
}
