use std::collections::HashSet;
use std::io;
use std::path::PathBuf;

use hex;
use warp_core::command::ExitCode;
use warpui::color::ColorU;

use super::*;
use crate::features::FeatureFlag;
use crate::terminal::model::ansi::InputBufferValue;
use crate::terminal::model::index::VisibleRow;
use crate::terminal::model::selection::ScrollDelta;
use crate::terminal::model::session::SessionId;

const HEX_ENCODED_JSON_DCS_START: &[u8] = &[0x1b, 0x50, 0x24, 0x64];
const UNENCODED_JSON_DCS_START: &[u8] = &[0x1b, 0x50, 0x24, 0x66];
const DCS_END: &[u8] = &[0x9c];
const DCS_END_7BIT: &[u8] = &[0x1b, 0x5c];

struct MockHandler {
    index: CharsetIndex,
    charset: StandardCharset,
    attr: Option<Attr>,
    identity_reported: bool,
    d_proto_hooks: Vec<DProtoHook>,
    pluggable_notifications: Vec<(Option<String>, String)>,
    hyperlink_events: Vec<Option<Hyperlink>>,
    cwd_updates: Vec<String>,
    registered_session_ids: HashSet<SessionId>,
    should_validate_dcs_hook_session_id: bool,
}

impl Handler for MockHandler {
    fn is_registered_session(&self, session_id: SessionId) -> bool {
        self.registered_session_ids.contains(&session_id)
    }

    fn should_validate_dcs_hook_session_id(&self) -> bool {
        self.should_validate_dcs_hook_session_id
    }
    fn terminal_attribute(&mut self, attr: Attr) {
        self.attr = Some(attr);
    }

    fn configure_charset(&mut self, index: CharsetIndex, charset: StandardCharset) {
        self.index = index;
        self.charset = charset;
    }

    fn set_active_charset(&mut self, index: CharsetIndex) {
        self.index = index;
    }

    fn identify_terminal<W: io::Write>(&mut self, _: &mut W, _intermediate: Option<char>) {
        self.identity_reported = true;
    }

    fn report_xtversion<W: io::Write>(&mut self, _: &mut W) {}

    fn reset_state(&mut self) {
        let registered_session_ids = self.registered_session_ids.clone();
        let should_validate_dcs_hook_session_id = self.should_validate_dcs_hook_session_id;
        *self = Self {
            registered_session_ids,
            should_validate_dcs_hook_session_id,
            ..Self::default()
        };
    }

    fn set_title(&mut self, _: Option<String>) {}

    fn set_cursor_style(&mut self, _: Option<super::CursorStyle>) {}

    fn set_cursor_shape(&mut self, _shape: super::CursorShape) {}

    fn input(&mut self, _c: char) {}

    fn goto(&mut self, _: VisibleRow, _: usize) {}

    fn goto_line(&mut self, _: VisibleRow) {}

    fn goto_col(&mut self, _: usize) {}

    fn insert_blank(&mut self, _: usize) {}

    fn move_up(&mut self, _: usize) {}

    fn move_down(&mut self, _: usize) {}

    fn device_status<W: io::Write>(&mut self, _: &mut W, _: usize) {}

    fn move_forward(&mut self, _: usize) {}

    fn move_backward(&mut self, _: usize) {}

    fn move_down_and_cr(&mut self, _: usize) {}

    fn move_up_and_cr(&mut self, _: usize) {}

    fn put_tab(&mut self, _count: u16) {}

    fn backspace(&mut self) {}

    fn carriage_return(&mut self) {}

    fn linefeed(&mut self) -> ScrollDelta {
        ScrollDelta::zero()
    }

    fn bell(&mut self) {}

    fn substitute(&mut self) {}

    fn newline(&mut self) {}

    fn set_horizontal_tabstop(&mut self) {}

    fn scroll_up(&mut self, _: usize) -> ScrollDelta {
        ScrollDelta::zero()
    }

    fn scroll_down(&mut self, _: usize) -> ScrollDelta {
        ScrollDelta::zero()
    }

    fn insert_blank_lines(&mut self, _: usize) -> ScrollDelta {
        ScrollDelta::zero()
    }

    fn delete_lines(&mut self, _: usize) -> ScrollDelta {
        ScrollDelta::zero()
    }

    fn erase_chars(&mut self, _: usize) {}

    fn delete_chars(&mut self, _: usize) {}

    fn move_backward_tabs(&mut self, _count: u16) {}

    fn move_forward_tabs(&mut self, _count: u16) {}

    fn save_cursor_position(&mut self) {}

    fn restore_cursor_position(&mut self) {}

    fn clear_line(&mut self, _mode: super::LineClearMode) {}

    fn clear_screen(&mut self, _mode: super::ClearMode) {}

    fn clear_tabs(&mut self, _mode: super::TabulationClearMode) {}

    fn reverse_index(&mut self) -> ScrollDelta {
        ScrollDelta::zero()
    }

    fn set_mode(&mut self, _mode: super::Mode) {}

    fn unset_mode(&mut self, _: super::Mode) {}

    fn set_scrolling_region(&mut self, _top: usize, _bottom: Option<usize>) {}

    fn set_keypad_application_mode(&mut self) {}

    fn unset_keypad_application_mode(&mut self) {}

    fn set_color(&mut self, _: usize, _: ColorU) {}

    fn dynamic_color_sequence<W: io::Write>(&mut self, _: &mut W, _: u8, _: usize, _: &str) {}

    fn reset_color(&mut self, _: usize) {}

    fn clipboard_store(&mut self, _: u8, _: &[u8]) {}

    fn clipboard_load(&mut self, _: u8, _: &str) {}

    fn decaln(&mut self) {}

    fn push_title(&mut self) {}

    fn pop_title(&mut self) {}

    fn text_area_size_pixels<W: io::Write>(&mut self, _: &mut W) {}

    fn text_area_size_chars<W: io::Write>(&mut self, _: &mut W) {}

    fn command_finished(&mut self, data: CommandFinishedValue) {
        self.d_proto_hooks
            .push(DProtoHook::CommandFinished { value: data });
    }

    fn precmd_with_completion_metadata(&mut self, data: PrecmdValue) {
        self.d_proto_hooks.push(DProtoHook::Precmd {
            value: PrecmdHookValue::WithCompletionMetadata(data),
        });
    }

    fn prompt_only_precmd(&mut self, data: PromptMetadata) {
        self.d_proto_hooks.push(DProtoHook::Precmd {
            value: PrecmdHookValue::PromptOnly(data),
        });
    }

    fn preexec(&mut self, data: PreexecValue) {
        self.d_proto_hooks.push(DProtoHook::Preexec { value: data });
    }

    fn bootstrapped(&mut self, data: BootstrappedValue) {
        self.d_proto_hooks.push(DProtoHook::Bootstrapped {
            value: Box::new(data),
        });
    }

    fn pre_interactive_ssh_session(&mut self, data: PreInteractiveSSHSessionValue) {
        self.d_proto_hooks
            .push(DProtoHook::PreInteractiveSSHSession { value: data })
    }

    fn ssh(&mut self, data: SSHValue) {
        self.d_proto_hooks.push(DProtoHook::SSH { value: data });
    }

    fn init_shell(&mut self, data: InitShellValue) {
        self.d_proto_hooks
            .push(DProtoHook::InitShell { value: data });
    }

    fn clear(&mut self, data: ClearValue) {
        self.d_proto_hooks.push(DProtoHook::Clear { value: data });
    }

    fn input_buffer(&mut self, data: super::InputBufferValue) {
        self.d_proto_hooks
            .push(DProtoHook::InputBuffer { value: data })
    }

    fn init_subshell(&mut self, data: InitSubshellValue) {
        self.d_proto_hooks
            .push(DProtoHook::InitSubshell { value: data })
    }

    fn sourced_rc_file(&mut self, data: SourcedRcFileForWarpValue) {
        self.d_proto_hooks
            .push(DProtoHook::SourcedRcFileForWarp { value: data })
    }

    fn pluggable_notification(&mut self, title: Option<String>, body: String) {
        self.pluggable_notifications.push((title, body));
    }

    fn set_hyperlink(&mut self, hyperlink: Option<Hyperlink>) {
        self.hyperlink_events.push(hyperlink);
    }

    fn set_current_working_directory(&mut self, path: String) {
        self.cwd_updates.push(path);
    }

    fn set_keyboard_enhancement_flags(
        &mut self,
        _mode: KeyboardModes,
        _apply: KeyboardModesApplyBehavior,
    ) {
    }

    fn push_keyboard_enhancement_flags(&mut self, _mode: KeyboardModes) {}

    fn pop_keyboard_enhancement_flags(&mut self, _count: u16) {}

    fn query_keyboard_enhancement_flags<W: io::Write>(&mut self, _: &mut W) {}
}

impl Default for MockHandler {
    fn default() -> MockHandler {
        MockHandler {
            index: CharsetIndex::G0,
            charset: StandardCharset::Ascii,
            attr: None,
            identity_reported: false,
            d_proto_hooks: Vec::new(),
            pluggable_notifications: Vec::new(),
            hyperlink_events: Vec::new(),
            cwd_updates: Vec::new(),
            registered_session_ids: HashSet::new(),
            should_validate_dcs_hook_session_id: true,
        }
    }
}

fn hex_encoded_dcs_string(dcs_payload: &str) -> Vec<u8> {
    let encoded_dcs_string = hex::encode(dcs_payload).into_bytes();
    [HEX_ENCODED_JSON_DCS_START, &encoded_dcs_string, DCS_END].concat()
}

fn parse_bytes(bytes: &[u8]) -> (Processor, MockHandler) {
    parse_bytes_with_registered_sessions(bytes, [SessionId::from(167303092612201)])
}

fn parse_bytes_with_registered_sessions(
    bytes: &[u8],
    registered_session_ids: impl IntoIterator<Item = SessionId>,
) -> (Processor, MockHandler) {
    parse_bytes_with_registered_sessions_and_validation(bytes, registered_session_ids, true)
}

fn parse_bytes_with_registered_sessions_and_validation(
    bytes: &[u8],
    registered_session_ids: impl IntoIterator<Item = SessionId>,
    should_validate_dcs_hook_session_id: bool,
) -> (Processor, MockHandler) {
    let mut parser = Processor::new();
    let mut handler = MockHandler {
        registered_session_ids: registered_session_ids.into_iter().collect(),
        should_validate_dcs_hook_session_id,
        ..Default::default()
    };

    parser.parse_bytes(&mut handler, bytes, &mut io::sink());

    (parser, handler)
}

#[test]
fn parse_control_attribute() {
    static BYTES: &[u8] = &[0x1b, b'[', b'1', b'm'];
    let (_, handler) = parse_bytes(BYTES);

    assert_eq!(handler.attr, Some(Attr::Bold));
}

#[test]
fn parse_terminal_identity_csi() {
    let bytes: &[u8] = &[0x1b, b'[', b'1', b'c'];

    let (mut parser, mut handler) = parse_bytes(bytes);

    assert!(!handler.identity_reported);
    handler.reset_state();

    let bytes: &[u8] = &[0x1b, b'[', b'c'];

    parser.parse_bytes(&mut handler, bytes, &mut io::sink());

    assert!(handler.identity_reported);
    handler.reset_state();

    let bytes: &[u8] = &[0x1b, b'[', b'0', b'c'];

    parser.parse_bytes(&mut handler, bytes, &mut io::sink());

    assert!(handler.identity_reported);
}

#[test]
fn parse_terminal_identity_esc() {
    let bytes: &[u8] = &[0x1b, b'Z'];

    let (mut parser, mut handler) = parse_bytes(bytes);

    assert!(handler.identity_reported);
    handler.reset_state();

    let bytes: &[u8] = &[0x1b, b'#', b'Z'];

    parser.parse_bytes(&mut handler, bytes, &mut io::sink());

    assert!(!handler.identity_reported);
    handler.reset_state();
}

#[test]
fn parse_truecolor_attr() {
    static BYTES: &[u8] = &[
        0x1b, b'[', b'3', b'8', b';', b'2', b';', b'1', b'2', b'8', b';', b'6', b'6', b';', b'2',
        b'5', b'5', b'm',
    ];

    let (_, handler) = parse_bytes(BYTES);

    let spec = ColorU::new(128, 66, 255, 0xff);

    assert_eq!(handler.attr, Some(Attr::Foreground(Color::Spec(spec))));
}

/// No exactly a test; useful for debugging.
#[test]
fn parse_zsh_startup() {
    static BYTES: &[u8] = &[
        0x1b, b'[', b'1', b'm', 0x1b, b'[', b'7', b'm', b'%', 0x1b, b'[', b'2', b'7', b'm', 0x1b,
        b'[', b'1', b'm', 0x1b, b'[', b'0', b'm', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ',
        b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ',
        b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ',
        b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ',
        b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ',
        b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b' ', b'\r', b' ', b'\r',
        b'\r', 0x1b, b'[', b'0', b'm', 0x1b, b'[', b'2', b'7', b'm', 0x1b, b'[', b'2', b'4', b'm',
        0x1b, b'[', b'J', b'j', b'w', b'i', b'l', b'm', b'@', b'j', b'w', b'i', b'l', b'm', b'-',
        b'd', b'e', b's', b'k', b' ', 0x1b, b'[', b'0', b'1', b';', b'3', b'2', b'm', 0xe2, 0x9e,
        0x9c, b' ', 0x1b, b'[', b'0', b'1', b';', b'3', b'2', b'm', b' ', 0x1b, b'[', b'3', b'6',
        b'm', b'~', b'/', b'c', b'o', b'd', b'e',
    ];

    parse_bytes(BYTES);
}

#[test]
fn parse_designate_g0_as_line_drawing() {
    static BYTES: &[u8] = &[0x1b, b'(', b'0'];
    let (_, handler) = parse_bytes(BYTES);

    assert_eq!(handler.index, CharsetIndex::G0);
    assert_eq!(
        handler.charset,
        StandardCharset::SpecialCharacterAndusizeDrawing
    );
}

#[test]
fn parse_designate_g1_as_line_drawing_and_invoke() {
    static BYTES: &[u8] = &[0x1b, b')', b'0', 0x0e];
    let (mut parser, handler) = parse_bytes(BYTES);

    assert_eq!(handler.index, CharsetIndex::G1);
    assert_eq!(
        handler.charset,
        StandardCharset::SpecialCharacterAndusizeDrawing
    );

    let mut handler = MockHandler::default();
    parser.parse_bytes(&mut handler, &[BYTES[3]], &mut io::sink());

    assert_eq!(handler.index, CharsetIndex::G1);
}

#[test]
fn parse_valid_rgb_colors() {
    assert_eq!(
        xparse_color(b"rgb:f/e/d"),
        Some(ColorU::new(0xff, 0xee, 0xdd, 0xff))
    );
    assert_eq!(
        xparse_color(b"rgb:11/aa/ff"),
        Some(ColorU::new(0x11, 0xaa, 0xff, 0xff))
    );
    assert_eq!(
        xparse_color(b"rgb:f/ed1/cb23"),
        Some(ColorU::new(0xff, 0xec, 0xca, 0xff))
    );
    assert_eq!(
        xparse_color(b"rgb:ffff/0/0"),
        Some(ColorU::new(0xff, 0x0, 0x0, 0xff))
    );
}

#[test]
fn parse_valid_legacy_rgb_colors() {
    assert_eq!(
        xparse_color(b"#1af"),
        Some(ColorU::new(0x10, 0xa0, 0xf0, 0xff))
    );
    assert_eq!(
        xparse_color(b"#11aaff"),
        Some(ColorU::new(0x11, 0xaa, 0xff, 0xff))
    );
    assert_eq!(
        xparse_color(b"#110aa0ff0"),
        Some(ColorU::new(0x11, 0xaa, 0xff, 0xff))
    );
    assert_eq!(
        xparse_color(b"#1100aa00ff00"),
        Some(ColorU::new(0x11, 0xaa, 0xff, 0xff))
    );
}

#[test]
fn parse_invalid_rgb_colors() {
    assert_eq!(xparse_color(b"rgb:0//"), None);
    assert_eq!(xparse_color(b"rgb://///"), None);
}

#[test]
fn parse_invalid_legacy_rgb_colors() {
    assert_eq!(xparse_color(b"#"), None);
    assert_eq!(xparse_color(b"#f"), None);
}

#[test]
fn parse_invalid_number() {
    assert_eq!(parse_number(b"1abc"), None);
}

#[test]
fn parse_valid_number() {
    assert_eq!(parse_number(b"123"), Some(123));
}

#[test]
fn parse_number_too_large() {
    assert_eq!(parse_number(b"321"), None);
}

#[test]
fn named_color_to_ansi_escape_valid() {
    assert!(matches!(NamedColor::Red.to_ansi_fg_escape_code(), Ok(31)));
    assert!(matches!(NamedColor::Red.to_ansi_bg_escape_code(), Ok(41)));
    assert!(matches!(
        NamedColor::BrightGreen.to_ansi_fg_escape_code(),
        Ok(92)
    ));
    assert!(matches!(
        NamedColor::BrightBlue.to_ansi_bg_escape_code(),
        Ok(104)
    ));
}

#[test]
fn named_color_to_ansi_escape_invalid() {
    assert!(NamedColor::Background.to_ansi_fg_escape_code().is_err());
    assert!(NamedColor::Foreground.to_ansi_bg_escape_code().is_err());
    assert!(NamedColor::Cursor.to_ansi_bg_escape_code().is_err());
}

#[test]
fn parse_dcs_ssh() {
    let bytes = hex_encoded_dcs_string(
        r#"{
                "hook": "SSH",
                "value": {
                    "socket_path": "~/.ssh/9001",
                    "remote_shell": "zsh",
                    "session_id": 167303092612201,
                    "remote_session_id": 167303092612202
                }
            }"#,
    );
    let (_, handler) = parse_bytes(&bytes);

    assert_eq!(handler.d_proto_hooks.len(), 1);
    match handler.d_proto_hooks.first().unwrap() {
        DProtoHook::SSH { value } => assert_eq!(
            *value,
            SSHValue {
                socket_path: PathBuf::from("~/.ssh/9001"),
                remote_shell: "zsh".to_string(),
                session_id: Some(167303092612201),
                remote_session_id: Some(167303092612202),
                external_control_master: false,
            }
        ),
        _ => panic!("incorrect dcs value"),
    };
}

#[test]
fn parse_dcs_ssh_with_external_control_master() {
    let bytes = hex_encoded_dcs_string(
        r#"{
                "hook": "SSH",
                "value": {
                    "socket_path": "/home/user/.ssh/cm-user@host:22",
                    "remote_shell": "zsh",
                    "session_id": 167303092612201,
                    "remote_session_id": 167303092612202,
                    "external_control_master": true
                }
            }"#,
    );
    let (_, handler) = parse_bytes(&bytes);

    assert_eq!(handler.d_proto_hooks.len(), 1);
    match handler.d_proto_hooks.first().unwrap() {
        DProtoHook::SSH { value } => assert_eq!(
            *value,
            SSHValue {
                socket_path: PathBuf::from("/home/user/.ssh/cm-user@host:22"),
                remote_shell: "zsh".to_string(),
                session_id: Some(167303092612201),
                remote_session_id: Some(167303092612202),
                external_control_master: true,
            }
        ),
        _ => panic!("incorrect dcs value"),
    };
}

#[test]
fn parse_dcs_precmd_classifies_payload_with_completion_metadata() {
    let bytes = hex_encoded_dcs_string(
        r#"{
                "hook": "Precmd",
                "value": {
                    "pwd": "/Users",
                    "ps1": "$>",
                    "honor_ps1": true,
                    "git_head": "",
                    "git_branch": "",
                    "virtual_env": "",
                    "conda_env": "numpy",
                    "exit_code": 0,
                    "next_block_id": "block_id",
                    "session_id": 167303092612201
                }
            }"#,
    );
    let (_, handler) = parse_bytes(&bytes);

    assert_eq!(handler.d_proto_hooks.len(), 1);
    match handler.d_proto_hooks.first().unwrap() {
        DProtoHook::Precmd { value } => assert_eq!(
            *value,
            PrecmdHookValue::WithCompletionMetadata(PrecmdValue {
                completion_metadata: CompletionMetadata {
                    exit_code: ExitCode::from(0),
                    next_block_id: "block_id".to_owned().into(),
                },
                prompt_metadata: PromptMetadata {
                    pwd: Some("/Users".to_string()),
                    ps1: Some("$>".to_string()),
                    honor_ps1: Some(true),
                    rprompt: None,
                    git_head: None,
                    git_branch: None,
                    virtual_env: None,
                    conda_env: Some("numpy".to_string()),
                    node_version: None,
                    kube_config: None,
                    session_id: Some(167303092612201),
                    ps1_is_encoded: None,
                    is_after_in_band_command: false,
                },
            })
        ),
        _ => panic!("incorrect dcs value"),
    };
}

#[test]
fn pending_precmd_classifies_payload_with_completion_metadata() {
    let mut hook = PendingHook::create("Precmd").unwrap();
    hook.update("exit_code".to_owned(), "127".to_owned());
    hook.update("next_block_id".to_owned(), "block_id".to_owned());
    hook.update("session_id".to_owned(), "167303092612201".to_owned());

    match hook.finish().unwrap() {
        DProtoHook::Precmd {
            value: PrecmdHookValue::WithCompletionMetadata(value),
        } => {
            assert_eq!(
                value.completion_metadata,
                CompletionMetadata {
                    exit_code: ExitCode::from(127),
                    next_block_id: "block_id".to_owned().into(),
                }
            );
            assert_eq!(value.prompt_metadata.session_id, Some(167303092612201));
        }
        _ => panic!("incorrect dcs value"),
    }
}

#[test]
fn parse_dcs_precmd_classifies_prompt_only_payload() {
    let bytes = hex_encoded_dcs_string(
        r#"{
                "hook": "Precmd",
                "value": {
                    "pwd": "/Users",
                    "session_id": 167303092612201
                }
            }"#,
    );
    let (_, handler) = parse_bytes(&bytes);

    assert_eq!(handler.d_proto_hooks.len(), 1);
    match handler.d_proto_hooks.first().unwrap() {
        DProtoHook::Precmd {
            value: PrecmdHookValue::PromptOnly(value),
        } => {
            assert_eq!(value.pwd.as_deref(), Some("/Users"));
            assert_eq!(value.session_id, Some(167303092612201));
        }
        _ => panic!("incorrect dcs value"),
    }
}

#[test]
fn parse_dcs_precmd_rejects_partial_completion_metadata() {
    for partial_completion_metadata in [
        r#""exit_code": 0"#,
        r#""next_block_id": "block_id""#,
        r#""exit_code": null"#,
        r#""exit_code": null, "next_block_id": "block_id""#,
    ] {
        let bytes = hex_encoded_dcs_string(&format!(
            r#"{{
                "hook": "Precmd",
                "value": {{
                    "pwd": "/Users",
                    {partial_completion_metadata},
                    "session_id": 167303092612201
                }}
            }}"#
        ));
        let (_, handler) = parse_bytes(&bytes);

        assert!(handler.d_proto_hooks.is_empty());
    }
}

#[test]
fn pending_precmd_rejects_partial_or_invalid_completion_metadata() {
    for fields in [
        vec![("exit_code", "0")],
        vec![("next_block_id", "block_id")],
        vec![("exit_code", "invalid"), ("next_block_id", "block_id")],
    ] {
        let mut hook = PendingHook::create("Precmd").unwrap();
        for (key, value) in fields {
            hook.update(key.to_owned(), value.to_owned());
        }

        assert!(hook.finish().is_err());
    }
}

#[test]
fn parse_dcs_unregistered_session_id_rejected() {
    let bytes = hex_encoded_dcs_string(
        r#"{
                "hook": "Precmd",
                "value": {
                    "pwd": "/Users",
                    "session_id": 167303092612201
                }
            }"#,
    );
    let (_, handler) = parse_bytes_with_registered_sessions(&bytes, []);

    assert_eq!(handler.d_proto_hooks.len(), 0);
}

#[test]
fn parse_dcs_unregistered_session_id_allowed_when_validation_disabled() {
    let bytes = hex_encoded_dcs_string(
        r#"{
                "hook": "Precmd",
                "value": {
                    "pwd": "/Users",
                    "session_id": 167303092612201
                }
            }"#,
    );
    let (_, handler) = parse_bytes_with_registered_sessions_and_validation(&bytes, [], false);

    assert_eq!(handler.d_proto_hooks.len(), 1);
    match handler.d_proto_hooks.first().unwrap() {
        DProtoHook::Precmd {
            value: PrecmdHookValue::PromptOnly(value),
        } => assert_eq!(value.session_id, Some(167303092612201)),
        _ => panic!("incorrect dcs value"),
    };
}

#[test]
fn parse_dcs_command_finished() {
    let bytes = hex_encoded_dcs_string(
        r#"{
                "hook": "CommandFinished",
                "value": {
                    "exit_code": 127,
                    "next_block_id": "block_id",
                    "session_id": 167303092612201
                }
            }"#,
    );
    let (_, handler) = parse_bytes(&bytes);

    assert_eq!(handler.d_proto_hooks.len(), 1);
    match handler.d_proto_hooks.first().unwrap() {
        DProtoHook::CommandFinished { value } => {
            assert_eq!(
                *value,
                CommandFinishedValue {
                    completion_metadata: CompletionMetadata {
                        exit_code: ExitCode::from(127),
                        next_block_id: "block_id".to_owned().into(),
                    },
                    session_id: Some(167303092612201)
                }
            )
        }
        _ => panic!("incorrect dcs value"),
    };
}

#[test]
fn parse_dcs_bootstrapped() {
    let bytes = hex_encoded_dcs_string(
        r#"{
                "hook": "Bootstrapped",
                "value": {
                    "histfile": "/Users/andy/.zsh_history",
                    "session_id": 167303092612201,
                    "shell": "bash",
                    "home_dir": "/Users/andy",
                    "user": "andy",
                    "host": "ubuntu-test",
                    "path": "/usr/sbin:/usr/bin",
                    "editor": "vim",
                    "aliases": "vi=nvim\nvim=nvim",
                    "abbreviations": "abbr -a -- vi nvim\nabbr -a -- gc 'git checkout'",
                    "env_var_names": "LOGNAME CARGO_HOME",
                    "function_names": "cd\nextract",
                    "builtins": "alias\nhistory",
                    "keywords": "for\nif",
                    "shell_version": "5.8.0",
                    "shell_options": "alwaystoend\nautocd",
                    "rcfiles_start_time": "1675789245.4744160175",
                    "rcfiles_end_time": "1675789246.9067308903",
                    "shell_plugins": "powerlevel10k pure",
                    "shell_path": "/usr/local/bin/bash"
                }
            }"#,
    );
    let (_, handler) = parse_bytes(&bytes);

    assert_eq!(handler.d_proto_hooks.len(), 1);
    match handler.d_proto_hooks.first().unwrap() {
        DProtoHook::Bootstrapped { value } => assert_eq!(
            **value,
            BootstrappedValue {
                session_id: Some(167303092612201),
                histfile: Some("/Users/andy/.zsh_history".to_string()),
                shell: "bash".to_string(),
                home_dir: Some("/Users/andy".to_string()),
                path: Some("/usr/sbin:/usr/bin".to_string()),
                cdpath: None,
                editor: Some("vim".to_string()),
                aliases: Some("vi=nvim\nvim=nvim".to_string()),
                abbreviations: Some("abbr -a -- vi nvim\nabbr -a -- gc 'git checkout'".to_string()),
                env_var_names: Some("LOGNAME CARGO_HOME".to_string()),
                function_names: Some("cd\nextract".to_string()),
                builtins: Some("alias\nhistory".to_string()),
                keywords: Some("for\nif".to_string()),
                shell_version: Some("5.8.0".to_string()),
                shell_options: Some(HashSet::from([
                    "alwaystoend".to_string(),
                    "autocd".to_string()
                ])),
                shell_plugins: Some(HashSet::from([
                    "powerlevel10k".to_string(),
                    "pure".to_string()
                ])),
                rcfiles_start_time: Some(1675789245.474416.into()),
                rcfiles_end_time: Some(1675789246.906731.into()),
                vi_mode_enabled: None,
                os_category: None,
                linux_distribution: None,
                wsl_name: None,
                shell_path: Some("/usr/local/bin/bash".to_string())
            }
        ),
        _ => panic!("incorrect dcs value"),
    };
}

#[test]
fn parse_dcs_init_shell() {
    let bytes = hex_encoded_dcs_string(
        r#"{
                "hook": "InitShell",
                "value": {
                    "session_id": 167303092612201,
                    "user": "andy",
                    "hostname": "ubuntu-test",
                    "shell": "zsh"
                }
            }"#,
    );
    let (_, handler) = parse_bytes(&bytes);

    assert_eq!(handler.d_proto_hooks.len(), 1);
    match handler.d_proto_hooks.first().unwrap() {
        DProtoHook::InitShell { value } => assert_eq!(
            *value,
            InitShellValue {
                session_id: SessionId::from(167303092612201),
                user: "andy".to_owned(),
                hostname: "ubuntu-test".to_owned(),
                shell: "zsh".to_string(),
                ..Default::default()
            }
        ),
        _ => panic!("incorrect dcs value"),
    };
}

#[test]
fn parse_dcs_init_shell_7bit_st() {
    let payload = r#"{
                "hook": "InitShell",
                "value": {
                    "session_id": 167303092612201,
                    "user": "andy",
                    "hostname": "ubuntu-test",
                    "shell": "bash"
                }
            }"#;
    let encoded = hex::encode(payload).into_bytes();
    let bytes = [HEX_ENCODED_JSON_DCS_START, &encoded, DCS_END_7BIT].concat();
    let (_, handler) = parse_bytes(&bytes);

    assert_eq!(handler.d_proto_hooks.len(), 1);
    match handler.d_proto_hooks.first().unwrap() {
        DProtoHook::InitShell { value } => assert_eq!(
            *value,
            InitShellValue {
                session_id: SessionId::from(167303092612201),
                user: "andy".to_owned(),
                hostname: "ubuntu-test".to_owned(),
                shell: "bash".to_string(),
                ..Default::default()
            }
        ),
        _ => panic!("incorrect dcs value"),
    };
}

#[test]
fn parse_dcs_input_buffer() {
    let bytes = hex_encoded_dcs_string(
        r#"{
                "hook": "InputBuffer",
                "value": {
                    "buffer": "ls -al dir",
                    "session_id": 167303092612201
                }
            }"#,
    );

    let (_, handler) = parse_bytes(&bytes);

    assert_eq!(handler.d_proto_hooks.len(), 1);
    match handler.d_proto_hooks.first().unwrap() {
        DProtoHook::InputBuffer { value } => assert_eq!(
            *value,
            InputBufferValue {
                buffer: "ls -al dir".to_string(),
                session_id: Some(167303092612201)
            }
        ),
        _ => panic!("incorrect dcs value"),
    }
}

#[test]
fn parse_sourced_rc_file_hook() {
    let rc_file_hook = r#"{"hook": "SourcedRcFileForWarp", "value": { "shell": "zsh" }}"#;
    let bytes = [
        UNENCODED_JSON_DCS_START,
        &Vec::from(rc_file_hook.as_bytes()),
        DCS_END,
    ]
    .concat();

    let (_, handler) = parse_bytes(&bytes);

    assert_eq!(handler.d_proto_hooks.len(), 1);
    match handler.d_proto_hooks.first().unwrap() {
        DProtoHook::SourcedRcFileForWarp { value } => assert_eq!(
            *value,
            SourcedRcFileForWarpValue {
                shell: "zsh".to_owned(),
                uname: None,
            }
        ),
        _ => panic!("incorrect dcs value"),
    }
}

#[test]
fn parse_sourced_rc_file_hook_with_uname() {
    let rc_file_hook =
        r#"{"hook": "SourcedRcFileForWarp", "value": { "shell": "zsh", "uname": "Darwin" }}"#;
    let bytes = [
        UNENCODED_JSON_DCS_START,
        &Vec::from(rc_file_hook.as_bytes()),
        DCS_END,
    ]
    .concat();

    let (_, handler) = parse_bytes(&bytes);

    assert_eq!(handler.d_proto_hooks.len(), 1);
    match handler.d_proto_hooks.first().unwrap() {
        DProtoHook::SourcedRcFileForWarp { value } => assert_eq!(
            *value,
            SourcedRcFileForWarpValue {
                shell: "zsh".to_owned(),
                uname: Some("Darwin".to_owned()),
            }
        ),
        _ => panic!("incorrect dcs value"),
    }
}

#[test]
fn parse_osc8_hyperlink_open() {
    let _guard = FeatureFlag::OscHyperlinks.override_enabled(true);
    // ESC ] 8 ; ; https://example.com ESC \
    let bytes: &[u8] = b"\x1b]8;;https://example.com\x1b\\";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.hyperlink_events.len(), 1);
    let hyperlink = handler.hyperlink_events[0].as_ref().expect("opening link");
    assert_eq!(hyperlink.id, None);
    assert_eq!(hyperlink.uri, "https://example.com");
}

#[test]
fn parse_osc8_hyperlink_open_with_id() {
    let _guard = FeatureFlag::OscHyperlinks.override_enabled(true);
    let bytes: &[u8] = b"\x1b]8;id=link-1;https://example.com\x07";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.hyperlink_events.len(), 1);
    let hyperlink = handler.hyperlink_events[0].as_ref().expect("opening link");
    assert_eq!(hyperlink.id.as_deref(), Some("link-1"));
    assert_eq!(hyperlink.uri, "https://example.com");
}

#[test]
fn parse_osc8_hyperlink_close_canonical() {
    let _guard = FeatureFlag::OscHyperlinks.override_enabled(true);
    // Canonical close: ESC ] 8 ; ; ESC \
    let bytes: &[u8] = b"\x1b]8;;\x1b\\";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.hyperlink_events.len(), 1);
    assert!(handler.hyperlink_events[0].is_none());
}

#[test]
fn parse_osc8_open_then_close_bell_terminator() {
    let _guard = FeatureFlag::OscHyperlinks.override_enabled(true);
    // Open with bell terminator, write some bytes (irrelevant to the dispatch
    // mock), then close. Both terminator forms must dispatch.
    let bytes: &[u8] = b"\x1b]8;;https://example.com/report\x07Click me\x1b]8;;\x07";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.hyperlink_events.len(), 2);
    let opened = handler.hyperlink_events[0].as_ref().expect("opening link");
    assert_eq!(opened.uri, "https://example.com/report");
    assert!(handler.hyperlink_events[1].is_none());
}

#[test]
fn parse_osc8_uri_with_semicolons_dispatches_full_uri() {
    let _guard = FeatureFlag::OscHyperlinks.override_enabled(true);
    // Anti-regression for the rejoin contract — the dispatcher must hand the
    // full URI (including embedded `;`) to set_hyperlink.
    let bytes: &[u8] = b"\x1b]8;;https://example.com/a?x=1;y=2\x1b\\";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.hyperlink_events.len(), 1);
    let hyperlink = handler.hyperlink_events[0].as_ref().expect("opening link");
    assert_eq!(hyperlink.uri, "https://example.com/a?x=1;y=2");
}

#[test]
fn parse_osc8_malformed_param_is_ignored_link_still_opens() {
    let _guard = FeatureFlag::OscHyperlinks.override_enabled(true);
    // A param without `=` is ignored (per the OSC 8 spec); the link still opens.
    let bytes: &[u8] = b"\x1b]8;notavalidparam;https://example.com\x07";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.hyperlink_events.len(), 1);
    let hyperlink = handler.hyperlink_events[0].as_ref().expect("opening link");
    assert_eq!(hyperlink.id, None);
    assert_eq!(hyperlink.uri, "https://example.com");
}

#[test]
fn parse_osc8_dropped_when_feature_flag_disabled() {
    let _guard = FeatureFlag::OscHyperlinks.override_enabled(false);
    let bytes: &[u8] = b"\x1b]8;;https://example.com\x1b\\";
    let (_, handler) = parse_bytes(bytes);

    // Flag off -> dispatch falls through to `unhandled`, no event fires.
    assert_eq!(handler.hyperlink_events.len(), 0);
}

#[test]
fn parse_osc8_malformed_sequence_clears_active_hyperlink() {
    let _guard = FeatureFlag::OscHyperlinks.override_enabled(true);
    // Open a valid link, then send a malformed (non-UTF-8 URI) sequence. The
    // parse error must clear the active hyperlink so subsequent output can't
    // inherit the stale URI.
    let bytes: &[u8] = b"\x1b]8;;https://example.com\x1b\\text\x1b]8;;\xff\xfe\x1b\\";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.hyperlink_events.len(), 2);
    assert_eq!(
        handler.hyperlink_events[0].as_ref().map(|h| h.uri.as_str()),
        Some("https://example.com")
    );
    assert!(handler.hyperlink_events[1].is_none());
}

#[test]
fn parse_osc9_notification() {
    let bytes: &[u8] = b"\x1b]9;Hello from OSC 9\x07";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.pluggable_notifications.len(), 1);
    let (title, body) = &handler.pluggable_notifications[0];
    assert_eq!(*title, None);
    assert_eq!(body, "Hello from OSC 9");
}

#[test]
fn parse_osc9_notification_with_st_terminator() {
    let bytes: &[u8] = b"\x1b]9;Message with ST terminator\x1b\\";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.pluggable_notifications.len(), 1);
    let (title, body) = &handler.pluggable_notifications[0];
    assert_eq!(*title, None);
    assert_eq!(body, "Message with ST terminator");
}

#[test]
fn parse_osc9_empty_body() {
    let bytes: &[u8] = b"\x1b]9;\x07";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.pluggable_notifications.len(), 0);
}

#[test]
fn parse_osc9_windows_terminal_cwd_ignored() {
    // OSC 9;9 is Windows Terminal's CWD notification (ESC ] 9 ; 9 ; "<cwd>" ST).
    // It should be silently ignored and not trigger a pluggable notification.
    // Reference: https://github.com/microsoft/terminal/issues/8166
    let bytes: &[u8] = b"\x1b]9;9;\"C:\\Users\\scottha\"\x07";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.pluggable_notifications.len(), 0);
}

#[test]
fn parse_osc9_numeric_subcommand_ignored() {
    // Any OSC 9 sequence with a purely numeric params[1] is a ConEmu-style subcommand
    // and should be silently ignored, not treated as a notification.
    // This covers known ones (9;4 progress, 9;9 CWD) and any unknown future ones.
    for subcommand in [b"1" as &[u8], b"2", b"3", b"4", b"5", b"6", b"7", b"8"] {
        let bytes = [b"\x1b]9;", subcommand, b";data\x07"].concat();
        let (_, handler) = parse_bytes(&bytes);
        assert_eq!(
            handler.pluggable_notifications.len(),
            0,
            "OSC 9;{} should be ignored",
            String::from_utf8_lossy(subcommand)
        );
    }
}

#[test]
fn parse_osc777_notification() {
    let bytes: &[u8] = b"\x1b]777;notify;Build Complete;Your build has finished\x07";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.pluggable_notifications.len(), 1);
    let (title, body) = &handler.pluggable_notifications[0];
    assert_eq!(title.as_deref(), Some("Build Complete"));
    assert_eq!(body, "Your build has finished");
}

#[test]
fn parse_osc777_notification_empty_title() {
    let bytes: &[u8] = b"\x1b]777;notify;;Just the body\x07";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.pluggable_notifications.len(), 1);
    let (title, body) = &handler.pluggable_notifications[0];
    assert_eq!(*title, None);
    assert_eq!(body, "Just the body");
}

#[test]
fn parse_osc777_notification_with_semicolons_in_body() {
    let bytes: &[u8] = b"\x1b]777;notify;Title;Body with; semicolons; here\x07";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.pluggable_notifications.len(), 1);
    let (title, body) = &handler.pluggable_notifications[0];
    assert_eq!(title.as_deref(), Some("Title"));
    assert_eq!(body, "Body with; semicolons; here");
}

#[test]
fn parse_osc777_non_notify_subcommand_ignored() {
    let bytes: &[u8] = b"\x1b]777;other;title;body\x07";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.pluggable_notifications.len(), 0);
}

#[test]
fn parse_osc777_missing_parts_ignored() {
    let bytes: &[u8] = b"\x1b]777;notify;only_title\x07";
    let (_, handler) = parse_bytes(bytes);

    assert_eq!(handler.pluggable_notifications.len(), 0);
}

#[test]
fn parse_osc1337_without_second_param_does_not_panic() {
    // Regression for #12817: a bare OSC 1337 with no second parameter
    // (`ESC ] 1337 BEL`) used to index `params[1]` unconditionally and panic
    // with "index out of bounds". Untrusted PTY output must never crash the
    // parser; a malformed sequence should be ignored instead.
    let bytes: &[u8] = b"\x1b]1337\x07";
    let (_, _handler) = parse_bytes(bytes);

    // Also exercise the ST-terminated form.
    let bytes: &[u8] = b"\x1b]1337\x1b\\";
    let (_, _handler) = parse_bytes(bytes);
}

#[test]
fn parse_osc7_local_hostname() {
    // Happy path: payload host matches the running machine's hostname.
    let local = crate::terminal::model::session::get_local_hostname()
        .expect("test requires a real local hostname");
    let payload = format!("\x1b]7;file://{local}/Users/foo/bar\x07");
    let (_, handler) = parse_bytes(payload.as_bytes());

    assert_eq!(handler.cwd_updates, vec!["/Users/foo/bar".to_string()]);
}

#[test]
fn parse_osc7_with_st_terminator() {
    let local = crate::terminal::model::session::get_local_hostname()
        .expect("test requires a real local hostname");
    let payload = format!("\x1b]7;file://{local}/Users/foo/bar\x1b\\");
    let (_, handler) = parse_bytes(payload.as_bytes());

    assert_eq!(handler.cwd_updates, vec!["/Users/foo/bar".to_string()]);
}

#[test]
fn parse_osc7_percent_encoded() {
    let local = crate::terminal::model::session::get_local_hostname()
        .expect("test requires a real local hostname");
    let payload = format!("\x1b]7;file://{local}/Users/foo%20bar/baz%2Fqux\x07");
    let (_, handler) = parse_bytes(payload.as_bytes());

    assert_eq!(
        handler.cwd_updates,
        vec!["/Users/foo bar/baz/qux".to_string()]
    );
}

#[test]
fn parse_osc7_path_with_unescaped_semicolons_preserved() {
    // OSC parameters split on `;`, so a URI path with a literal semicolon
    // arrives as multiple params. Rejoining preserves the full path instead
    // of truncating at the first semicolon.
    let local = crate::terminal::model::session::get_local_hostname()
        .expect("test requires a real local hostname");
    let payload = format!("\x1b]7;file://{local}/Users/foo;bar/baz\x07");
    let (_, handler) = parse_bytes(payload.as_bytes());

    assert_eq!(handler.cwd_updates, vec!["/Users/foo;bar/baz".to_string()]);
}

#[test]
fn parse_osc7_empty_host_ignored() {
    // Hostless payload (`file:///path`) is terminal-controlled and a remote
    // shell over a wrapper SSH session can emit it just as easily as a local one; reject.
    let bytes: &[u8] = b"\x1b]7;file:///Users/foo/bar\x07";
    let (_, handler) = parse_bytes(bytes);

    assert!(handler.cwd_updates.is_empty());
}

#[test]
fn parse_osc7_localhost_host_ignored() {
    // `localhost` is also untrustworthy from a remote shell — reject.
    let bytes: &[u8] = b"\x1b]7;file://localhost/Users/foo\x07";
    let (_, handler) = parse_bytes(bytes);

    assert!(handler.cwd_updates.is_empty());
}

#[test]
fn parse_osc7_uppercase_localhost_host_ignored() {
    let bytes: &[u8] = b"\x1b]7;file://LOCALHOST/Users/foo\x07";
    let (_, handler) = parse_bytes(bytes);

    assert!(handler.cwd_updates.is_empty());
}

#[test]
fn parse_osc7_non_local_host_ignored() {
    // `.invalid` is reserved (RFC 2606) and is guaranteed never to match the
    // local hostname, so this exercises the SSH-spoofing guard.
    let bytes: &[u8] = b"\x1b]7;file://not-this-machine.invalid/Users/foo\x07";
    let (_, handler) = parse_bytes(bytes);

    assert!(handler.cwd_updates.is_empty());
}

#[test]
fn parse_osc7_non_file_scheme_ignored() {
    let bytes: &[u8] = b"\x1b]7;http://example.com/foo\x07";
    let (_, handler) = parse_bytes(bytes);

    assert!(handler.cwd_updates.is_empty());
}

#[test]
fn parse_osc7_missing_path_ignored() {
    // Host is present but no path segment — should be rejected, not panic.
    let bytes: &[u8] = b"\x1b]7;file://localhost\x07";
    let (_, handler) = parse_bytes(bytes);

    assert!(handler.cwd_updates.is_empty());
}

#[test]
fn parse_osc7_malformed_percent_escape_ignored() {
    let bytes: &[u8] = b"\x1b]7;file:///Users/foo%2/bar\x07";
    let (_, handler) = parse_bytes(bytes);

    assert!(handler.cwd_updates.is_empty());
}

#[test]
fn parse_osc7_truncated_percent_at_end_ignored() {
    // A trailing `%` with no following digits must be rejected, not accepted
    // as a literal byte.
    let bytes: &[u8] = b"\x1b]7;file:///Users/foo%\x07";
    let (_, handler) = parse_bytes(bytes);

    assert!(handler.cwd_updates.is_empty());
}

#[test]
fn parse_osc7_truncated_percent_with_one_hex_digit_ignored() {
    // A `%` with only one following hex digit must be rejected.
    let bytes: &[u8] = b"\x1b]7;file:///Users/foo%2\x07";
    let (_, handler) = parse_bytes(bytes);

    assert!(handler.cwd_updates.is_empty());
}

#[test]
fn parse_osc7_empty_payload_ignored() {
    let bytes: &[u8] = b"\x1b]7;\x07";
    let (_, handler) = parse_bytes(bytes);

    assert!(handler.cwd_updates.is_empty());
}
