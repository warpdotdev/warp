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

#[test]
fn same_buffer_shell_then_dcs_then_output() {
    use super::DecodeItem;
    let mut parser = ControlModeParser::new();
    let mut bytes = b"prompt$ ".to_vec();
    bytes.extend_from_slice(CONTROL_MODE_DCS);
    bytes.extend_from_slice(b"%output %0 hi\n");
    assert_eq!(
        parser.decode(&bytes),
        vec![
            DecodeItem::Shell(b"prompt$ ".to_vec()),
            DecodeItem::Control(ControlEvent::EnteredControlMode),
            DecodeItem::Control(ControlEvent::PaneOutput {
                pane_id: PaneId::from("%0"),
                bytes: b"hi".to_vec(),
            }),
        ]
    );
    assert!(parser.is_in_control_mode());
}

#[test]
fn dcs_prefix_is_held_across_chunks_without_delaying_plain_bytes() {
    use super::DecodeItem;
    let mut parser = ControlModeParser::new();
    assert_eq!(
        parser.decode(b"abc"),
        vec![DecodeItem::Shell(b"abc".to_vec())]
    );
    assert!(parser.decode(&CONTROL_MODE_DCS[..3]).is_empty());
    assert_eq!(
        parser.decode(&CONTROL_MODE_DCS[3..]),
        vec![DecodeItem::Control(ControlEvent::EnteredControlMode)]
    );
}

#[test]
fn same_buffer_exit_then_shell_bytes() {
    use super::DecodeItem;
    let mut parser = ControlModeParser::new();
    let mut bytes = CONTROL_MODE_DCS.to_vec();
    bytes.extend_from_slice(b"%output %0 x\n%exit\n$ ");
    assert_eq!(
        parser.decode(&bytes),
        vec![
            DecodeItem::Control(ControlEvent::EnteredControlMode),
            DecodeItem::Control(ControlEvent::PaneOutput {
                pane_id: PaneId::from("%0"),
                bytes: b"x".to_vec(),
            }),
            DecodeItem::Control(ControlEvent::Exit { reason: None }),
            DecodeItem::Shell(b"$ ".to_vec()),
        ]
    );
    assert!(!parser.is_in_control_mode());
}

#[test]
fn oversized_control_line_discards_until_exit() {
    use super::DecodeItem;
    let mut parser = ControlModeParser::new();
    parser.decode(CONTROL_MODE_DCS);
    assert!(parser.is_in_control_mode());
    let overflow = vec![b'x'; 1_048_577];
    let items = parser.decode(&overflow);
    assert!(
        items
            .iter()
            .any(|item| matches!(item, DecodeItem::Control(ControlEvent::ProtocolOverflow)))
    );
    assert!(!parser.is_in_control_mode());
    assert!(
        parser
            .decode(b"prompt$ ")
            .iter()
            .all(|item| !matches!(item, DecodeItem::Shell(_)))
    );
}

#[test]
fn valid_notification_after_overflow_does_not_reach_shell() {
    use super::DecodeItem;
    let mut parser = ControlModeParser::new();
    parser.decode(CONTROL_MODE_DCS);
    let overflow = vec![b'x'; 1_048_577];
    parser.decode(&overflow);
    let items = parser.decode(b"%output %0 leaked\n%layout-change @0 80x24,0,0,0\n%exit\n$ ");
    assert!(!items.iter().any(|item| matches!(
        item,
        DecodeItem::Shell(bytes) if bytes.windows(b"%output".len()).any(|w| w == b"%output")
            || bytes.windows(b"%layout".len()).any(|w| w == b"%layout")
    )));
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, DecodeItem::Control(ControlEvent::PaneOutput { .. })))
    );
    assert!(
        items
            .iter()
            .any(|item| matches!(item, DecodeItem::Control(ControlEvent::Exit { .. })))
    );
    assert_eq!(
        items
            .iter()
            .filter_map(|item| match item {
                DecodeItem::Shell(bytes) => Some(bytes.as_slice()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![&b"$ "[..]]
    );
}

#[test]
fn oversized_reply_payload_aborts_to_shell_parsing() {
    use super::DecodeItem;
    let mut parser = ControlModeParser::new();
    parser.decode(CONTROL_MODE_DCS);
    let mut bytes = b"%begin 1 1\n".to_vec();
    for _ in 0..10_001 {
        bytes.extend_from_slice(b"line\n");
    }
    let items = parser.decode(&bytes);
    assert!(
        items
            .iter()
            .any(|item| matches!(item, DecodeItem::Control(ControlEvent::ProtocolOverflow)))
    );
    assert!(!parser.is_in_control_mode());
    assert!(
        parser
            .decode(b"%output %0 leaked\n")
            .iter()
            .all(|item| !matches!(
                item,
                DecodeItem::Shell(_) | DecodeItem::Control(ControlEvent::PaneOutput { .. })
            ))
    );
}

#[test]
fn protocol_lines_are_not_shell_bytes() {
    use super::DecodeItem;
    let mut parser = ControlModeParser::new();
    let mut bytes = CONTROL_MODE_DCS.to_vec();
    bytes.extend_from_slice(b"%begin 1 1\nsecret\n%end 1 1\n");
    let items = parser.decode(&bytes);
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, DecodeItem::Shell(_)))
    );
    assert!(items.iter().any(|item| matches!(
        item,
        DecodeItem::Control(ControlEvent::CommandEnd { payload, .. }) if payload == &["secret".to_string()]
    )));
}
