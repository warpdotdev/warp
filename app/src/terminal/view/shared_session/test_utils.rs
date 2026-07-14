use session_sharing_protocol::common::{ParticipantId, ParticipantList, SessionId};
use session_sharing_protocol::sharer::SessionSourceType;
use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity, ViewHandle};

use crate::auth::UserUid;
use crate::editor::ReplicaId;
use crate::pane_group::PaneGroup;
use crate::server::server_api::ServerApiProvider;
use crate::terminal::shared_session::manager::Manager;
use crate::terminal::TerminalView;
use crate::test_util::terminal::initialize_app_for_terminal_view;
use crate::GlobalResourceHandles;

/// Creates a terminal view that is created via the terminal manager
/// for shared session viewers. That is, it has all of the relevant models
/// set up for the viewer.
pub fn terminal_view_for_viewer(app: &mut App) -> ViewHandle<TerminalView> {
    initialize_app_for_terminal_view(app);
    app.add_singleton_model(Manager::new);

    let global_resource_handles = GlobalResourceHandles::mock(app);
    let GlobalResourceHandles {
        model_event_sender,
        tips_completed,
        user_default_shell_unsupported_banner_model_handle,
        ..
    } = global_resource_handles.clone();

    let session_id = SessionId::new();

    let (_, pane_group) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        let server_api = ServerApiProvider::as_ref(ctx).get();
        PaneGroup::new_for_shared_session_viewer(
            session_id,
            tips_completed,
            user_default_shell_unsupported_banner_model_handle,
            server_api,
            model_event_sender,
            false, // is_ambient_agent
            ctx,
        )
    });

    let terminal = pane_group.read(app, |pane_group, ctx| {
        pane_group
            .terminal_view_at_pane_index(0, ctx)
            .expect("TerminalView exists at pane index 0")
            .to_owned()
    });

    let firebase_uid = UserUid::new("mock_firebase_uid");
    terminal.update(app, |view, ctx| {
        view.on_session_share_joined(
            ParticipantId::new(),
            firebase_uid,
            ReplicaId::random(),
            Box::new(ParticipantList::default()),
            SessionId::new(),
            SessionSourceType::User,
            ctx,
        );
    });

    terminal
}
