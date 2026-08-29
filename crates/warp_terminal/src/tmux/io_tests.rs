use std::borrow::Cow;

use super::{TmuxFeedItem, TmuxIoState, TmuxPhaseKind, is_tmux_cc_start};
use crate::tmux::parser::{CONTROL_MODE_DCS, PaneId, WindowId};

fn start_command() -> Cow<'static, [u8]> {
    Cow::Borrowed(b"tmux -CC new-session -A -s warp -n warp -x 80 -y 24\n")
}

fn managed_start_command() -> Cow<'static, [u8]> {
    Cow::Borrowed(b"tmux -CC -L warp-control-v1 new-session -A -s warp -n warp -x 80 -y 24\n")
}

fn enter_control(io: &mut TmuxIoState) {
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.feed(b"%begin 1 0\n%end 1 0\n");
}

fn enter_control_after_snapshot(io: &mut TmuxIoState) {
    enter_control(io);
    io.feed(b"%begin 1 1\n%end 1 1\n");
}

#[test]
fn tmux_cc_start_is_detected() {
    assert!(is_tmux_cc_start(b"tmux -CC new-session -A -s warp\n"));
    assert!(is_tmux_cc_start(b"  tmux -CC\n"));
    assert!(!is_tmux_cc_start(b"echo tmux -CC\n"));
    assert!(super::is_managed_isolated_tmux_cc(
        b"tmux -CC -L warp-control-v1 new-session -A -s warp\n"
    ));
    assert!(super::is_managed_isolated_tmux_cc(
        b"tmux -CC -Lwarp-control-v1 new-session\n"
    ));
    assert!(!super::is_managed_isolated_tmux_cc(
        b"tmux -CC new-session -A -s warp\n"
    ));
    assert!(!super::is_managed_isolated_tmux_cc(
        b"tmux -CC -L warp-control-v1x new-session\n"
    ));
    assert!(!super::is_managed_isolated_tmux_cc(
        b"tmux -CC -Lwarp-control-v1x new-session\n"
    ));
    assert!(!super::is_managed_isolated_tmux_cc(
        b"tmux -CC -L xwarp-control-v1 new-session\n"
    ));
    assert!(!super::is_managed_isolated_tmux_cc(
        b"tmux -CC -Lxwarp-control-v1 new-session\n"
    ));
    assert!(!super::is_managed_isolated_tmux_cc(
        b"tmux -CC new-session -- 'tmux -CC -L warp-control-v1'\n"
    ));
}

#[test]
fn inputs_after_start_command_are_held_until_dcs() {
    let mut io = TmuxIoState::new();
    let written = io.enqueue_input(start_command());
    assert_eq!(written, vec![start_command()]);
    assert_eq!(io.phase(), TmuxPhaseKind::StartPending);

    assert!(io.enqueue_input(Cow::Borrowed(b"hello")).is_empty());
    let items = io.feed(CONTROL_MODE_DCS);
    assert!(
        items
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::EnteredControl { .. }))
    );
    assert_eq!(io.phase(), TmuxPhaseKind::InControl);
    assert!(io.focused_pane().is_none());
}

#[test]
fn resize_during_handshake_issues_refresh_client_on_dcs() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    assert!(io.enqueue_resize(100, 40).is_none());
    let items = io.feed(CONTROL_MODE_DCS);
    match &items[0] {
        TmuxFeedItem::EnteredControl { refresh_client } => {
            assert_eq!(
                refresh_client.as_deref(),
                Some("refresh-client -C 100x40\n")
            );
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn pending_input_is_replayed_raw_when_entry_fails() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.enqueue_input(Cow::Borrowed(b"typed-before-dcs"));
    let items = io.feed(b"tmux: command not found\n");
    let replay = items.iter().find_map(|item| match item {
        TmuxFeedItem::Exited { replay } => Some(replay.clone()),
        _ => None,
    });
    assert_eq!(replay, Some(vec![Cow::Borrowed(&b"typed-before-dcs"[..])]));
    assert_eq!(io.phase(), TmuxPhaseKind::Inactive);
}

#[test]
fn benign_startup_output_does_not_fail_start_pending() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.enqueue_input(Cow::Borrowed(b"held"));
    let items = io.feed(b"starting tmux 3.4\n");
    assert_eq!(io.phase(), TmuxPhaseKind::StartPending);
    assert!(
        items
            .iter()
            .all(|item| !matches!(item, TmuxFeedItem::Exited { .. }))
    );
    let items = io.feed(CONTROL_MODE_DCS);
    assert!(matches!(items[0], TmuxFeedItem::EnteredControl { .. }));
    assert_eq!(io.phase(), TmuxPhaseKind::InControl);
}

#[test]
fn start_pending_timeout_replays_queued_input() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.enqueue_input(Cow::Borrowed(b"typed-while-waiting"));
    let later = instant::Instant::now() + std::time::Duration::from_secs(30);
    let items = io.check_start_timeout(later);
    let replay = items.iter().find_map(|item| match item {
        TmuxFeedItem::Exited { replay } => Some(replay.clone()),
        _ => None,
    });
    assert_eq!(
        replay,
        Some(vec![Cow::Borrowed(&b"typed-while-waiting"[..])])
    );
    assert_eq!(io.phase(), TmuxPhaseKind::Inactive);
}

#[test]
fn command_not_found_interleaved_with_dcs_does_not_enter_control() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.enqueue_input(Cow::Borrowed(b"held"));
    let mut bytes = b"bash: tmux: command not found\n".to_vec();
    bytes.extend_from_slice(CONTROL_MODE_DCS);
    bytes.extend_from_slice(b"%output %0 leaked\n");
    let items = io.feed(&bytes);
    assert_eq!(io.phase(), TmuxPhaseKind::Inactive);
    assert!(
        items
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::Exited { .. }))
    );
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::EnteredControl { .. }))
    );
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::PaneOutput { .. }))
    );
}

#[test]
fn overflow_does_not_replay_while_tmux_still_in_control_mode() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.enqueue_input(Cow::Borrowed(b"typed-in-control"));
    let overflow = vec![b'x'; 1_048_577];
    let items = io.feed(&overflow);
    assert_eq!(io.phase(), TmuxPhaseKind::OverflowRecovering);
    assert!(items.iter().any(|item| match item {
        TmuxFeedItem::OverflowRecovering { detach } =>
            detach.as_ref() as &[u8] == b"detach-client\n",
        _ => false,
    }));
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::Exited { .. }))
    );
    assert!(io.enqueue_input(Cow::Borrowed(b"more")).is_empty());
}

#[test]
fn valid_notification_after_overflow_does_not_reach_shell() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.feed(&vec![b'x'; 1_048_577]);
    let items = io.feed(b"%output %0 leaked\n%exit\n$ ");
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::PaneOutput { .. }))
    );
    assert!(!items.iter().any(|item| matches!(
        item,
        TmuxFeedItem::Shell(bytes) if bytes.windows(b"%output".len()).any(|w| w == b"%output")
    )));
    let replay = items.iter().find_map(|item| match item {
        TmuxFeedItem::Exited { replay } => Some(replay.clone()),
        _ => None,
    });
    assert!(replay.is_some());
    assert_eq!(io.phase(), TmuxPhaseKind::Inactive);
}

#[test]
fn window_pane_changed_selects_focus_not_first_output() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.enqueue_input(Cow::Borrowed(b"keys"));
    let mut bytes = b"%output %0 one\n%output %1 two\n".to_vec();
    let items = io.feed(&bytes);
    assert!(io.focused_pane().is_none());
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::EncodedPending(_)))
    );

    bytes = b"%window-pane-changed @0 %1\n".to_vec();
    let items = io.feed(&bytes);
    assert_eq!(io.focused_pane().map(PaneId::as_str), Some("%1"));
    assert!(items.iter().any(|item| matches!(
        item,
        TmuxFeedItem::EncodedPending(encoded) if encoded.starts_with(b"send-keys -t %1")
    )));
}

#[test]
fn reattach_active_pane_is_not_percent_zero() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.feed(b"%window-pane-changed @3 %7\n");
    assert_eq!(io.focused_pane().map(PaneId::as_str), Some("%7"));
    let encoded = io.enqueue_input(Cow::Borrowed(b"x"));
    assert_eq!(encoded.len(), 1);
    assert!(encoded[0].starts_with(b"send-keys -t %7"));
}

#[test]
fn layout_with_one_pane_focuses_that_pane() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.feed(b"%layout-change @0 80x24,0,0,4\n");
    assert_eq!(io.focused_pane().map(PaneId::as_str), Some("%4"));
}

#[test]
fn client_commands_stay_raw_in_control_mode() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.feed(b"%window-pane-changed @0 %1\n");
    let written = io.enqueue_control_command(Cow::Borrowed(b"split-window -h -t %1\n"));
    assert_eq!(
        written,
        vec![Cow::Borrowed(&b"split-window -h -t %1\n"[..])]
    );
}

#[test]
fn interleaved_output_and_focus_does_not_steal_pending_keys() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.enqueue_input(Cow::Borrowed(b"typed"));
    let items = io.feed(b"%output %9 noise\n%window-pane-changed @2 %3\n");
    assert_eq!(io.focused_pane().map(PaneId::as_str), Some("%3"));
    assert!(items.iter().any(|item| matches!(
        item,
        TmuxFeedItem::EncodedPending(encoded) if encoded.starts_with(b"send-keys -t %3")
    )));
}

#[test]
fn latest_resize_wins_when_interleaved_with_start() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.enqueue_resize(80, 24);
    io.enqueue_resize(120, 40);
    let items = io.feed(CONTROL_MODE_DCS);
    match &items[0] {
        TmuxFeedItem::EnteredControl { refresh_client } => {
            assert_eq!(
                refresh_client.as_deref(),
                Some("refresh-client -C 120x40\n")
            );
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn interleaved_split_does_not_consume_capture_reply() {
    let mut io = TmuxIoState::new();
    enter_control_after_snapshot(&mut io);
    io.feed(b"%window-pane-changed @0 %4\n");
    io.enqueue_control_command(Cow::Borrowed(b"capture-pane -p -t %4\n"));
    io.enqueue_control_command(Cow::Borrowed(b"split-window -h -t %4 -P -F '#{pane_id}'\n"));
    io.enqueue_control_command(Cow::Borrowed(b"select-pane -t %4\n"));
    let capture_end = io.feed(b"%begin 1 1\nsnapshot-line\n%end 1 1\n");
    assert!(capture_end.iter().any(|item| matches!(
        item,
        TmuxFeedItem::CommandEnd {
            capture_pane: Some(pane),
            payload,
            ..
        } if pane.as_str() == "%4" && payload == &["snapshot-line".to_string()]
    )));
    let split_end = io.feed(b"%begin 1 2\n%5\n%end 1 2\n");
    assert!(split_end.iter().any(|item| matches!(
        item,
        TmuxFeedItem::CommandEnd {
            capture_pane: None,
            payload,
            ..
        } if payload == &["%5".to_string()]
    )));
}

#[test]
fn window_events_are_forwarded() {
    let mut io = TmuxIoState::new();
    io.feed(CONTROL_MODE_DCS);
    let items = io.feed(b"%window-add @2\n");
    assert_eq!(
        items,
        vec![TmuxFeedItem::WindowAdd {
            window_id: WindowId::from("@2")
        }]
    );
}

#[test]
fn enter_issues_list_windows_snapshot() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    let items = io.feed(CONTROL_MODE_DCS);
    assert!(items.iter().any(|item| matches!(
        item,
        TmuxFeedItem::EncodedPending(bytes)
            if bytes.as_slice() == crate::tmux::encode::LIST_WINDOWS_LAYOUT_COMMAND.as_bytes()
    )));
}

#[test]
fn enter_disables_exit_empty_before_snapshot() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(managed_start_command());
    let items = io.feed(CONTROL_MODE_DCS);
    let encoded: Vec<&[u8]> = items
        .iter()
        .filter_map(|item| match item {
            TmuxFeedItem::EncodedPending(bytes) => Some(bytes.as_slice()),
            _ => None,
        })
        .collect();
    let exit_empty = encoded
        .iter()
        .position(|bytes| *bytes == crate::tmux::encode::EXIT_EMPTY_OFF_COMMAND.as_bytes())
        .expect("exit-empty off");
    let snapshot = encoded
        .iter()
        .position(|bytes| *bytes == crate::tmux::encode::LIST_WINDOWS_LAYOUT_COMMAND.as_bytes())
        .expect("list-windows");
    assert!(exit_empty < snapshot);
    assert!(
        !encoded
            .iter()
            .any(|bytes| bytes.starts_with(b"set -s exit-unattached"))
    );
}

#[test]
fn arbitrary_tmux_cc_does_not_disable_exit_empty() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    let items = io.feed(CONTROL_MODE_DCS);
    assert!(!items.iter().any(|item| matches!(
        item,
        TmuxFeedItem::EncodedPending(bytes)
            if bytes.as_slice() == crate::tmux::encode::EXIT_EMPTY_OFF_COMMAND.as_bytes()
    )));
    assert!(items.iter().any(|item| matches!(
        item,
        TmuxFeedItem::EncodedPending(bytes)
            if bytes.as_slice() == crate::tmux::encode::LIST_WINDOWS_LAYOUT_COMMAND.as_bytes()
    )));
}

#[test]
fn explicit_managed_flag_disables_exit_empty_without_warp_socket() {
    let mut io = TmuxIoState::new().with_managed_isolated();
    io.enqueue_input(start_command());
    let items = io.feed(CONTROL_MODE_DCS);
    assert!(items.iter().any(|item| matches!(
        item,
        TmuxFeedItem::EncodedPending(bytes)
            if bytes.as_slice() == crate::tmux::encode::EXIT_EMPTY_OFF_COMMAND.as_bytes()
    )));
}

#[test]
fn tmux_3_6a_new_session_without_layout_change_bootstraps_at0_percent0() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    let mut bytes = CONTROL_MODE_DCS.to_vec();
    bytes.extend_from_slice(
        b"%begin 271 0\n%end 271 0\n%window-add @0\n%sessions-changed\n%session-changed $0 warp\n%output %0 hi\n",
    );
    let items = io.feed(&bytes);
    assert!(
        items
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::EnteredControl { .. }))
    );
    assert!(items.iter().any(|item| matches!(
        item,
        TmuxFeedItem::WindowAdd {
            window_id
        } if window_id.as_str() == "@0"
    )));
    assert!(items.iter().any(|item| matches!(
        item,
        TmuxFeedItem::LayoutChange {
            window_id,
            layout,
            ..
        } if window_id.as_str() == "@0" && layout.contains(",0")
    )));
    assert_eq!(io.focused_pane().map(PaneId::as_str), Some("%0"));
    assert!(items.iter().any(|item| matches!(
        item,
        TmuxFeedItem::PaneOutput { pane_id, bytes }
            if pane_id.as_str() == "%0" && bytes == b"hi"
    )));
}

#[test]
fn empty_new_session_command_end_does_not_steal_snapshot() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.feed(b"%begin 271 0\n%end 271 0\n");
    let items = io.feed(b"%begin 272 1\n@0 80x24,0,0,0\n%end 272 1\n");
    assert!(items.iter().any(|item| matches!(
        item,
        TmuxFeedItem::LayoutChange {
            window_id,
            layout,
            ..
        } if window_id.as_str() == "@0" && layout == "80x24,0,0,0"
    )));
    assert_eq!(io.focused_pane().map(PaneId::as_str), Some("%0"));
}

#[test]
fn presentation_ready_timeout_detaches_when_layout_never_arrives() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    let later = instant::Instant::now() + std::time::Duration::from_secs(30);
    let items = io.check_timeouts(later);
    assert!(items.iter().any(|item| match item {
        TmuxFeedItem::PresentationUnready { detach } => {
            detach.as_ref() as &[u8] == b"detach-client\n"
        }
        _ => false,
    }));
    assert_eq!(io.phase(), TmuxPhaseKind::PresentationRecovering);
}

#[test]
fn output_before_window_add_bootstraps_first_pane_only() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    let mut bytes = CONTROL_MODE_DCS.to_vec();
    bytes.extend_from_slice(
        b"%begin 271 0\n%end 271 0\n%output %0 hi\n%output %1 later\n%window-add @0\n",
    );
    let items = io.feed(&bytes);
    assert!(items.iter().any(|item| matches!(
        item,
        TmuxFeedItem::LayoutChange {
            window_id,
            layout,
            ..
        } if window_id.as_str() == "@0" && layout == "80x24,0,0,0"
    )));
    assert_eq!(io.focused_pane().map(PaneId::as_str), Some("%0"));
}

#[test]
fn presentation_timeout_holds_input_and_ignores_late_snapshot_until_exit() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    let later = instant::Instant::now() + std::time::Duration::from_secs(30);
    io.check_timeouts(later);
    assert_eq!(io.phase(), TmuxPhaseKind::PresentationRecovering);
    assert!(
        io.enqueue_input(Cow::Borrowed(b"typed-after-timeout"))
            .is_empty()
    );
    let late = io.feed(b"%begin 1 1\n@0 80x24,0,0,0\n%end 1 1\n%output %0 leaked\n");
    assert!(
        !late
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::LayoutChange { .. }))
    );
    assert!(
        !late
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::PaneOutput { .. }))
    );
    let items = io.feed(b"%exit\n");
    let replay = items.iter().find_map(|item| match item {
        TmuxFeedItem::Exited { replay } => Some(replay.clone()),
        _ => None,
    });
    assert_eq!(
        replay,
        Some(vec![Cow::Borrowed(&b"typed-after-timeout"[..])])
    );
    assert_eq!(io.phase(), TmuxPhaseKind::Inactive);
}

#[test]
fn superseded_snapshot_reply_is_ignored() {
    let mut io = TmuxIoState::new();
    enter_control(&mut io);
    io.enqueue_control_command(Cow::Borrowed(
        crate::tmux::encode::LIST_WINDOWS_LAYOUT_COMMAND.as_bytes(),
    ));
    let stale = io.feed(b"%begin 1 1\n@0 80x24,0,0,0\n%end 1 1\n");
    assert!(
        !stale
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::LayoutChange { .. }))
    );
    assert!(io.focused_pane().is_none());
    let live = io.feed(b"%begin 1 2\n@0 80x24,0,0,7\n%end 1 2\n");
    assert!(live.iter().any(|item| matches!(
        item,
        TmuxFeedItem::LayoutChange {
            window_id,
            layout,
            ..
        } if window_id.as_str() == "@0" && layout == "80x24,0,0,7"
    )));
    assert_eq!(io.focused_pane().map(PaneId::as_str), Some("%7"));
}

#[test]
fn snapshot_error_clears_intent_and_ignores_late_success() {
    let mut io = TmuxIoState::new();
    enter_control(&mut io);
    let err = io.feed(b"%begin 1 1\nno windows\n%error 1 1\n");
    assert!(
        err.iter()
            .any(|item| matches!(item, TmuxFeedItem::CommandEnd { error: true, .. }))
    );
    let late = io.feed(b"%begin 1 2\n@0 80x24,0,0,0\n%end 1 2\n");
    assert!(
        !late
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::LayoutChange { .. }))
    );
    assert!(io.focused_pane().is_none());
}

#[test]
fn snapshot_error_then_capture_does_not_steal_capture_reply() {
    let mut io = TmuxIoState::new();
    enter_control(&mut io);
    let err = io.feed(b"%begin 1 1\nno windows\n%error 1 1\n");
    assert!(
        err.iter()
            .any(|item| matches!(item, TmuxFeedItem::CommandEnd { error: true, .. }))
    );
    io.enqueue_control_command(Cow::Borrowed(b"capture-pane -p -t %0\n"));
    let capture = io.feed(b"%begin 1 2\npane-bytes\n%end 1 2\n");
    assert!(capture.iter().any(|item| matches!(
        item,
        TmuxFeedItem::CommandEnd {
            error: false,
            capture_pane: Some(pane),
            payload,
            ..
        } if pane.as_str() == "%0" && payload == &["pane-bytes".to_string()]
    )));
    assert!(
        !capture
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::LayoutChange { .. }))
    );
}

#[test]
fn multi_window_add_then_percent0_output_waits_for_snapshot() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    let mut bytes = CONTROL_MODE_DCS.to_vec();
    bytes.extend_from_slice(
        b"%begin 1 0\n%end 1 0\n%window-add @0\n%window-add @1\n%output %0 hi\n",
    );
    let items = io.feed(&bytes);
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::LayoutChange { .. }))
    );
    assert!(io.focused_pane().is_none());
    let snapshot = io.feed(b"%begin 1 1\n@0 80x24,0,0,0\n@1 80x24,0,0,3\n%end 1 1\n");
    assert!(snapshot.iter().any(|item| matches!(
        item,
        TmuxFeedItem::LayoutChange {
            window_id,
            layout,
            ..
        } if window_id.as_str() == "@0" && layout == "80x24,0,0,0"
    )));
    assert!(snapshot.iter().any(|item| matches!(
        item,
        TmuxFeedItem::LayoutChange {
            window_id,
            layout,
            ..
        } if window_id.as_str() == "@1" && layout == "80x24,0,0,3"
    )));
}

#[test]
fn capture_then_snapshot_does_not_treat_layout_shaped_capture_as_snapshot() {
    let mut io = TmuxIoState::new();
    enter_control_after_snapshot(&mut io);
    io.enqueue_control_command(Cow::Borrowed(b"capture-pane -p -t %0\n"));
    io.enqueue_control_command(Cow::Borrowed(
        crate::tmux::encode::LIST_WINDOWS_LAYOUT_COMMAND.as_bytes(),
    ));
    let capture = io.feed(b"%begin 1 2\n@0 80x24,0,0,0\n%end 1 2\n");
    assert!(capture.iter().any(|item| matches!(
        item,
        TmuxFeedItem::CommandEnd {
            capture_pane: Some(pane),
            payload,
            ..
        } if pane.as_str() == "%0" && payload == &["@0 80x24,0,0,0".to_string()]
    )));
    assert!(
        !capture
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::LayoutChange { .. }))
    );
    let snapshot = io.feed(b"%begin 1 3\n@0 80x24,0,0,4\n%end 1 3\n");
    assert!(snapshot.iter().any(|item| matches!(
        item,
        TmuxFeedItem::LayoutChange {
            window_id,
            layout,
            ..
        } if window_id.as_str() == "@0" && layout == "80x24,0,0,4"
    )));
}

#[test]
fn empty_snapshot_then_capture_keeps_capture_reply() {
    let mut io = TmuxIoState::new();
    enter_control(&mut io);
    let empty = io.feed(b"%begin 1 1\n%end 1 1\n");
    assert!(empty.iter().any(|item| matches!(
        item,
        TmuxFeedItem::CommandEnd {
            capture_pane: None,
            error: false,
            ..
        }
    )));
    io.enqueue_control_command(Cow::Borrowed(b"capture-pane -p -t %3\n"));
    let capture = io.feed(b"%begin 1 2\npane-bytes\n%end 1 2\n");
    assert!(capture.iter().any(|item| matches!(
        item,
        TmuxFeedItem::CommandEnd {
            capture_pane: Some(pane),
            payload,
            ..
        } if pane.as_str() == "%3" && payload == &["pane-bytes".to_string()]
    )));
}

#[test]
fn other_then_snapshot_does_not_apply_layout_shaped_other_payload() {
    let mut io = TmuxIoState::new();
    enter_control_after_snapshot(&mut io);
    io.enqueue_control_command(Cow::Borrowed(b"select-pane -t %0\n"));
    io.enqueue_control_command(Cow::Borrowed(
        crate::tmux::encode::LIST_WINDOWS_LAYOUT_COMMAND.as_bytes(),
    ));
    let other = io.feed(b"%begin 1 2\n@0 80x24,0,0,0\n%end 1 2\n");
    assert!(other.iter().any(|item| matches!(
        item,
        TmuxFeedItem::CommandEnd {
            capture_pane: None,
            payload,
            ..
        } if payload == &["@0 80x24,0,0,0".to_string()]
    )));
    assert!(
        !other
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::LayoutChange { .. }))
    );
    let snapshot = io.feed(b"%begin 1 3\n@0 80x24,0,0,9\n%end 1 3\n");
    assert!(snapshot.iter().any(|item| matches!(
        item,
        TmuxFeedItem::LayoutChange {
            layout,
            ..
        } if layout == "80x24,0,0,9"
    )));
}

#[test]
fn startup_end_then_refresh_error_does_not_drop_snapshot() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.enqueue_resize(100, 40);
    io.feed(CONTROL_MODE_DCS);
    let startup = io.feed(b"%begin 1 0\n%end 1 0\n");
    assert!(startup.iter().any(|item| matches!(
        item,
        TmuxFeedItem::CommandEnd {
            error: false,
            capture_pane: None,
            ..
        }
    )));
    let refresh_err = io.feed(b"%begin 1 1\nbad size\n%error 1 1\n");
    assert!(refresh_err.iter().any(|item| matches!(
        item,
        TmuxFeedItem::CommandEnd {
            error: true,
            capture_pane: None,
            ..
        }
    )));
    let snapshot = io.feed(b"%begin 1 2\n@0 80x24,0,0,0\n%end 1 2\n");
    assert!(snapshot.iter().any(|item| matches!(
        item,
        TmuxFeedItem::LayoutChange {
            window_id,
            layout,
            ..
        } if window_id.as_str() == "@0" && layout == "80x24,0,0,0"
    )));
}

#[test]
fn detach_client_holds_raw_input_until_exit_then_replays() {
    let mut io = TmuxIoState::new();
    enter_control(&mut io);
    let written = io.enqueue_control_command(Cow::Borrowed(b"detach-client\n"));
    assert_eq!(written, vec![Cow::Borrowed(&b"detach-client\n"[..])]);
    assert_eq!(io.phase(), TmuxPhaseKind::PresentationRecovering);
    assert!(
        io.enqueue_input(Cow::Borrowed(b"typed-after-detach"))
            .is_empty()
    );
    let leaked = io.feed(b"%output %0 leaked\n");
    assert!(
        !leaked
            .iter()
            .any(|item| matches!(item, TmuxFeedItem::PaneOutput { .. }))
    );
    let items = io.feed(b"%exit\n");
    let replay = items.iter().find_map(|item| match item {
        TmuxFeedItem::Exited { replay } => Some(replay.clone()),
        _ => None,
    });
    assert_eq!(
        replay,
        Some(vec![Cow::Borrowed(&b"typed-after-detach"[..])])
    );
    assert_eq!(io.phase(), TmuxPhaseKind::Inactive);
}

#[test]
fn internal_send_keys_command_is_emitted_exactly_once() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.feed(b"%window-pane-changed @0 %0\n");
    let command = Cow::Borrowed(&b"send-keys -t %0 -H 41\n"[..]);
    let written = io.enqueue_control_command(command.clone());
    assert_eq!(written, vec![command]);
}

#[test]
fn user_key_a_becomes_one_send_keys_command() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.feed(b"%window-pane-changed @0 %0\n");
    let written = io.enqueue_pane_input(&PaneId::from("%0"), Cow::Borrowed(b"A"));
    assert_eq!(written.len(), 1);
    assert_eq!(&written[0][..], b"send-keys -t %0 -H 41\n");
}

#[test]
fn bootstrap_and_split_control_commands_are_not_double_encoded() {
    let mut io = TmuxIoState::new();
    io.enqueue_input(start_command());
    io.feed(CONTROL_MODE_DCS);
    io.feed(b"%window-pane-changed @0 %0\n");
    let bootstrap = crate::tmux::encode::send_keys_command(&PaneId::from("%0"), b":\n");
    let written = io.enqueue_control_command(Cow::Owned(bootstrap.clone()));
    assert_eq!(written.len(), 1);
    assert_eq!(&written[0][..], bootstrap.as_slice());
    let split = io.enqueue_control_command(Cow::Borrowed(b"split-window -h -t %0\n"));
    assert_eq!(split, vec![Cow::Borrowed(&b"split-window -h -t %0\n"[..])]);
}

#[test]
fn raw_control_is_rejected_outside_control_mode() {
    let mut io = TmuxIoState::new();
    assert!(
        io.enqueue_control_command(Cow::Borrowed(b"split-window -h\n"))
            .is_empty()
    );
    assert!(
        io.enqueue_pane_input(&PaneId::from("%0"), Cow::Borrowed(b"A"))
            .is_empty()
    );
}
