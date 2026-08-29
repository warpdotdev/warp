use super::*;

#[test]
fn send_keys_encodes_hex_for_the_target_pane() {
    let encoded = send_keys_command(&PaneId::from("%3"), b"A\n");
    assert_eq!(encoded, b"send-keys -t %3 -H 41 0a\n");
}
