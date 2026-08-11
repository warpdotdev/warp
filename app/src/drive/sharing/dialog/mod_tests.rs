use chrono::Local;
use session_sharing_protocol::common::SessionId;
use warpui::{App, SingletonEntity, ViewHandle};

use super::{SharingDialog, SharingDialogMode};
use crate::drive::sharing::ShareableObject;
use crate::terminal::TerminalView;
use crate::terminal::shared_session::manager::Manager;
use crate::terminal::shared_session::{SharedSessionSource, SharedSessionStatus};
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

fn set_shared_session_status(
    terminal: &ViewHandle<TerminalView>,
    status: SharedSessionStatus,
    app: &mut App,
) {
    terminal.update(app, |view, _| {
        view.model.lock().set_shared_session_status(status);
    });
}

fn assert_session_link_state(
    terminal: &ViewHandle<TerminalView>,
    dialog: &ViewHandle<SharingDialog>,
    expected_session_id: Option<SessionId>,
    app: &App,
) {
    terminal.read(app, |view, ctx| {
        let shared_session_status = view.model.lock().shared_session_status().clone();
        let manager = Manager::as_ref(ctx);
        assert_eq!(
            manager.session_id_for_link(&view.id(), &shared_session_status),
            expected_session_id
        );
        assert_eq!(
            manager.has_session_link(&view.id(), &shared_session_status),
            expected_session_id.is_some()
        );
    });

    dialog.read(app, |dialog, ctx| {
        assert_eq!(
            dialog.has_shared_session_link(ctx),
            expected_session_id.is_some()
        );
    });
}

#[test]
fn session_qr_code_requires_status_eligible_matching_session_id() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(Manager::new);
        app.add_singleton_model(
            |_| crate::workspaces::user_profiles::UserProfiles::new(Vec::new()),
        );
        let terminal = add_window_with_terminal(&mut app, None);
        let first_session_id = SessionId::new();

        terminal.update(&mut app, |view, ctx| {
            let window_id = ctx.window_id();
            Manager::handle(ctx).update(ctx, |manager, ctx| {
                manager.started_share(terminal.downgrade(), first_session_id, window_id, ctx);
            });
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::ActiveSharer);
        });

        let first_target = ShareableObject::Session {
            handle: terminal.downgrade(),
            session_id: first_session_id,
            started_at: Local::now(),
        };
        let window_id = app.update(|ctx| ctx.window_ids().next().unwrap());
        let dialog = app.add_typed_action_view(window_id, |ctx| {
            SharingDialog::new(Some(first_target.clone()), ctx)
        });

        assert_session_link_state(&terminal, &dialog, Some(first_session_id), &app);
        dialog.update(&mut app, |dialog, ctx| dialog.show_qr_code(ctx));
        dialog.read(&app, |dialog, _| {
            assert_eq!(dialog.mode, SharingDialogMode::QrCode);
        });

        Manager::handle(&app).update(&mut app, |manager, ctx| {
            manager.stopped_share(terminal.id(), ctx);
        });

        for status in [
            SharedSessionStatus::NotShared,
            SharedSessionStatus::FinishedViewer,
        ] {
            set_shared_session_status(&terminal, status, &mut app);
            assert_session_link_state(&terminal, &dialog, Some(first_session_id), &app);
        }

        set_shared_session_status(&terminal, SharedSessionStatus::SharePending, &mut app);
        assert_session_link_state(&terminal, &dialog, None, &app);
        dialog.update(&mut app, |dialog, ctx| {
            dialog.refresh_shared_session_link(ctx);
            assert_eq!(dialog.mode, SharingDialogMode::Access);
        });

        for status in [
            SharedSessionStatus::ViewPending,
            SharedSessionStatus::ActiveViewer {
                role: Default::default(),
            },
            SharedSessionStatus::SharePendingPreBootstrap {
                source: SharedSessionSource::default(),
            },
            SharedSessionStatus::ActiveSharer,
        ] {
            set_shared_session_status(&terminal, status, &mut app);
            assert_session_link_state(&terminal, &dialog, None, &app);
            dialog.update(&mut app, |dialog, ctx| {
                dialog.mode = SharingDialogMode::Access;
                dialog.show_qr_code(ctx);
                assert_eq!(dialog.mode, SharingDialogMode::Access);
            });
        }

        let second_session_id = SessionId::new();
        terminal.update(&mut app, |view, ctx| {
            let window_id = ctx.window_id();
            Manager::handle(ctx).update(ctx, |manager, ctx| {
                manager.started_share(terminal.downgrade(), second_session_id, window_id, ctx);
            });
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::ActiveSharer);
        });

        terminal.read(&app, |view, ctx| {
            let shared_session_status = view.model.lock().shared_session_status().clone();
            assert_eq!(
                Manager::as_ref(ctx).session_id_for_link(&view.id(), &shared_session_status),
                Some(second_session_id)
            );
        });
        dialog.read(&app, |dialog, ctx| {
            assert!(!dialog.has_shared_session_link(ctx));
        });

        dialog.update(&mut app, |dialog, ctx| {
            dialog.set_target(
                Some(ShareableObject::Session {
                    handle: terminal.downgrade(),
                    session_id: second_session_id,
                    started_at: Local::now(),
                }),
                ctx,
            );
        });
        assert_session_link_state(&terminal, &dialog, Some(second_session_id), &app);
    });
}
