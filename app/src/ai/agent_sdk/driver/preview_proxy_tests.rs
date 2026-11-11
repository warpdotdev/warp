use super::*;

#[test]
fn parses_listener_port_configuration() {
    assert_eq!(listener_port("").unwrap(), 49152);
    assert_eq!(listener_port("1").unwrap(), 1);
    assert_eq!(listener_port("65535").unwrap(), 65535);
    for value in ["0", "65536", "+30000", " 30000"] {
        assert!(listener_port(value).is_err(), "{value:?}");
    }
}
