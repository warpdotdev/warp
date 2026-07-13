use warp::tui_export::ServerConversationToken;

use super::TuiExitSummaryHandle;

#[test]
fn exit_summary_tracks_and_clears_selected_token() {
    let summary = TuiExitSummaryHandle::default();
    let token = ServerConversationToken::new(uuid::Uuid::new_v4().to_string());

    assert_eq!(summary.token(), None);
    summary.set_token(Some(token.clone()));
    assert_eq!(summary.token(), Some(token));
    summary.set_token(None);
    assert_eq!(summary.token(), None);
}
