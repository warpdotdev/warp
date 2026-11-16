use super::*;

#[test]
fn test_ios() {
    let ua = "Mozilla/5.0 (iPhone; CPU iPhone OS 16_0 like Mac OS X) AppleWebKit/605.1.15";
    assert!(is_mobile_user_agent(ua));
}

#[test]
fn test_android() {
    let ua = "Mozilla/5.0 (Linux; Android 13; Pixel 7) AppleWebKit/537.36 Chrome/108.0.0.0 Mobile Safari/537.36";
    assert!(is_mobile_user_agent(ua));
}

#[test]
fn test_desktop() {
    assert!(!is_mobile_user_agent(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)"
    ));
    assert!(!is_mobile_user_agent(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64)"
    ));
}

#[test]
fn test_empty_user_agent() {
    assert!(!is_mobile_user_agent(""));
}

#[test]
fn test_case_insensitivity() {
    let ua = "MOZILLA/5.0 (IPHONE; CPU IPHONE OS 16_0 LIKE MAC OS X)";
    assert!(is_mobile_user_agent(ua));
}
