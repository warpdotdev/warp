use super::{CONTROL_MODE_DCS, ControlEvent, ControlModeParser, PaneId, octal_unescape};

fn enter_and_push(parser: &mut ControlModeParser, rest: &[u8]) -> Vec<ControlEvent> {
    let mut bytes = CONTROL_MODE_DCS.to_vec();
    bytes.extend_from_slice(rest);
    parser.push(&bytes)
}

fn octal_escape(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        if b < b' ' || b == b'\\' || b >= 0x7f {
            out.push_str(&format!("\\{b:03o}"));
        } else {
            out.push(b as char);
        }
    }
    out
}

#[test]
fn dcs_enters_control_mode() {
    let mut parser = ControlModeParser::new();
    let events = parser.push(CONTROL_MODE_DCS);
    assert_eq!(events, vec![ControlEvent::EnteredControlMode]);
    assert!(parser.is_in_control_mode());
}

#[test]
fn dcs_can_be_split_across_chunks() {
    let mut parser = ControlModeParser::new();
    assert!(parser.push(&CONTROL_MODE_DCS[..3]).is_empty());
    assert!(!parser.is_in_control_mode());
    assert_eq!(
        parser.push(&CONTROL_MODE_DCS[3..]),
        vec![ControlEvent::EnteredControlMode]
    );
}

#[test]
fn bytes_before_dcs_are_not_pane_output() {
    let mut parser = ControlModeParser::new();
    let mut bytes = b"this is not a pane\n".to_vec();
    bytes.extend_from_slice(CONTROL_MODE_DCS);
    bytes.extend_from_slice(b"%output %0 hi\n");
    let events = parser.push(&bytes);
    assert_eq!(
        events,
        vec![
            ControlEvent::EnteredControlMode,
            ControlEvent::PaneOutput {
                pane_id: PaneId::from("%0"),
                bytes: b"hi".to_vec(),
            }
        ]
    );
}

#[test]
fn begin_end_command_reply_is_not_pane_output() {
    let mut parser = ControlModeParser::new();
    let events = enter_and_push(
        &mut parser,
        b"%begin 1 2\n%output %0 should-not-leak\nlive-reply-line\n%end 1 2\n%output %0 real\n",
    );
    assert_eq!(
        events,
        vec![
            ControlEvent::EnteredControlMode,
            ControlEvent::CommandBegin { time: 1, number: 2 },
            ControlEvent::CommandEnd {
                time: 1,
                number: 2,
                error: false,
                payload: vec![
                    "%output %0 should-not-leak".into(),
                    "live-reply-line".into()
                ],
            },
            ControlEvent::PaneOutput {
                pane_id: PaneId::from("%0"),
                bytes: b"real".to_vec(),
            },
        ]
    );
}

#[test]
fn error_ends_command_reply() {
    let mut parser = ControlModeParser::new();
    let events = enter_and_push(&mut parser, b"%begin 9 8 extra\nnope\n%error 9 8\n");
    assert_eq!(
        events,
        vec![
            ControlEvent::EnteredControlMode,
            ControlEvent::CommandBegin { time: 9, number: 8 },
            ControlEvent::CommandEnd {
                time: 9,
                number: 8,
                error: true,
                payload: vec!["nope".into()],
            },
        ]
    );
}

#[test]
fn output_octal_decodes_escape_and_high_bytes() {
    let payload = b"\x1b]9278;{\"hook\":\"InitShell\"}\x07";
    let line = format!("%output %12 {}\n", octal_escape(payload));
    let mut parser = ControlModeParser::new();
    let events = enter_and_push(&mut parser, line.as_bytes());
    assert_eq!(
        events,
        vec![
            ControlEvent::EnteredControlMode,
            ControlEvent::PaneOutput {
                pane_id: PaneId::from("%12"),
                bytes: payload.to_vec(),
            },
        ]
    );
}

#[test]
fn output_octal_decodes_dcs_json_bytes() {
    let payload = b"\x1bP$warp;{\"hook\":\"precmd\"}\x1b\\";
    let line = format!("%output %0 {}\n", octal_escape(payload));
    let mut parser = ControlModeParser::new();
    let events = enter_and_push(&mut parser, line.as_bytes());
    match &events[1] {
        ControlEvent::PaneOutput { pane_id, bytes } => {
            assert_eq!(pane_id.as_str(), "%0");
            assert_eq!(bytes, payload);
        }
        other => panic!("expected pane output, got {other:?}"),
    }
}

#[test]
fn incomplete_line_is_held() {
    let mut parser = ControlModeParser::new();
    let events = enter_and_push(&mut parser, b"%output %0 hel");
    assert_eq!(events, vec![ControlEvent::EnteredControlMode]);
    assert_eq!(
        parser.push(b"lo\n"),
        vec![ControlEvent::PaneOutput {
            pane_id: PaneId::from("%0"),
            bytes: b"hello".to_vec(),
        }]
    );
}

#[test]
fn exit_with_and_without_reason() {
    let mut parser = ControlModeParser::new();
    let events = enter_and_push(&mut parser, b"%exit\n");
    assert_eq!(
        events,
        vec![
            ControlEvent::EnteredControlMode,
            ControlEvent::Exit { reason: None },
        ]
    );
    assert!(!parser.is_in_control_mode());
    let mut parser = ControlModeParser::new();
    let events = enter_and_push(&mut parser, b"%exit server exited\n");
    assert_eq!(
        events,
        vec![
            ControlEvent::EnteredControlMode,
            ControlEvent::Exit {
                reason: Some("server exited".to_owned()),
            },
        ]
    );
}

#[test]
fn command_reply_payload_is_retained_on_end() {
    let mut parser = ControlModeParser::new();
    let events = enter_and_push(&mut parser, b"%begin 1 2\n%1\n%end 1 2\n");
    assert_eq!(
        events,
        vec![
            ControlEvent::EnteredControlMode,
            ControlEvent::CommandBegin { time: 1, number: 2 },
            ControlEvent::CommandEnd {
                time: 1,
                number: 2,
                error: false,
                payload: vec!["%1".into()],
            },
        ]
    );
}

#[test]
fn layout_and_window_notifications_are_parsed() {
    use super::WindowId;
    let mut parser = ControlModeParser::new();
    let events = enter_and_push(
        &mut parser,
        b"%window-add @1\n%layout-change @1 1x1,0,0,0 1x1,0,0,0 *\n%session-window-changed $1 @1\n%window-pane-changed @1 %2\n%window-close @1\n",
    );
    assert_eq!(
        events,
        vec![
            ControlEvent::EnteredControlMode,
            ControlEvent::WindowAdd {
                window_id: WindowId::from("@1"),
            },
            ControlEvent::LayoutChange {
                window_id: WindowId::from("@1"),
                layout: "1x1,0,0,0".into(),
                visible_layout: Some("1x1,0,0,0".into()),
                flags: Some("*".into()),
            },
            ControlEvent::SessionWindowChanged {
                window_id: WindowId::from("@1"),
            },
            ControlEvent::WindowPaneChanged {
                window_id: WindowId::from("@1"),
                pane_id: PaneId::from("%2"),
            },
            ControlEvent::WindowClose {
                window_id: WindowId::from("@1"),
            },
        ]
    );
}

#[test]
fn unknown_notifications_are_dropped() {
    let mut parser = ControlModeParser::new();
    let events = enter_and_push(
        &mut parser,
        b"%sessions-changed\n%client-session-changed /dev/pts/0 $1\n",
    );
    assert_eq!(events, vec![ControlEvent::EnteredControlMode]);
}

#[test]
fn octal_unescape_leaves_printable_bytes() {
    assert_eq!(octal_unescape(b"abc"), b"abc");
    assert_eq!(octal_unescape(b"\\033"), &[0x1b]);
    assert_eq!(octal_unescape(b"\\134"), b"\\");
}
