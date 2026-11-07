use super::should_block_save_for_secrets;

/// #8761: with redaction disabled and no enterprise enforcement, saving a
/// config that contains secrets must NOT be blocked.
#[test]
fn does_not_block_when_redaction_off_even_if_secrets_present() {
    assert!(!should_block_save_for_secrets(false, false, true));
}

/// User-level toggle on AND secrets present → block. This is the case the
/// original check was written to catch; the redaction-aware predicate
/// must preserve it.
#[test]
fn blocks_when_user_redaction_on_and_secrets_present() {
    assert!(should_block_save_for_secrets(true, false, true));
}

/// Enterprise enforcement alone is enough to gate the save, even if the
/// user toggled their personal redaction off — orgs that mandate redaction
/// must not be bypassed at the MCP-config layer.
#[test]
fn blocks_when_enterprise_enforced_and_secrets_present() {
    assert!(should_block_save_for_secrets(false, true, true));
}

/// Configs without any detected secrets are never blocked, regardless of
/// the redaction-toggle state. The check is purely a guard against
/// accidentally persisting secrets — it has nothing to add when none exist.
#[test]
fn does_not_block_when_no_secrets_regardless_of_toggle() {
    for safe_mode in [false, true] {
        for enterprise in [false, true] {
            assert!(
                !should_block_save_for_secrets(safe_mode, enterprise, false),
                "expected no block when contains_secrets=false \
                 (safe_mode={safe_mode}, enterprise={enterprise})",
            );
        }
    }
}

/// Both toggles on AND secrets present → block. Defensive: equivalent to
/// either one being on, but exhaustively pinned for the full 2x2x2 sweep
/// of (safe_mode, enterprise, contains_secrets).
#[test]
fn blocks_when_both_redactions_on_and_secrets_present() {
    assert!(should_block_save_for_secrets(true, true, true));
}

/// #13300: MCP server configs containing IP URLs or host arguments must not be flagged
/// as containing secrets, while actual credentials remain blocked.
#[test]
fn does_not_block_mcp_config_with_ip_address_endpoints() {
    use regex::Regex;

    use crate::ai::blocklist::secret_redaction::find_secrets_in_text_excluding_ips;
    use crate::terminal::model::secrets::{regexes, set_user_and_enterprise_secret_regexes};

    let ipv4_regex = Regex::new(regexes::IPV4_ADDRESS).unwrap();
    let ipv6_regex = Regex::new(regexes::IPV6_ADDRESS).unwrap();
    let openai_regex = Regex::new(regexes::OPENAI_API_KEY).unwrap();

    set_user_and_enterprise_secret_regexes(
        [&ipv4_regex, &ipv6_regex, &openai_regex],
        std::iter::empty(),
    );

    let local_mcp = r#"{
      "paper": {
        "type": "http",
        "url": "http://127.0.0.1:29979/mcp"
      }
    }"#;
    let contains_secrets = !find_secrets_in_text_excluding_ips(local_mcp).is_empty();
    assert!(!contains_secrets);
    assert!(!should_block_save_for_secrets(true, true, contains_secrets));

    let ipv6_mcp = r#"{
      "paper": {
        "type": "http",
        "url": "http://[2001:0db8:85a3:0000:0000:8a2e:0370:7334]:8080/mcp"
      }
    }"#;
    let contains_secrets = !find_secrets_in_text_excluding_ips(ipv6_mcp).is_empty();
    assert!(!contains_secrets);
    assert!(!should_block_save_for_secrets(true, true, contains_secrets));

    let credential_mcp = r#"{
      "openai": {
        "type": "http",
        "url": "https://api.openai.com",
        "headers": {
          "Authorization": "Bearer sk-123456789012345678901234567890123456789012345678"
        }
      }
    }"#;
    let contains_secrets = !find_secrets_in_text_excluding_ips(credential_mcp).is_empty();
    assert!(contains_secrets);
    assert!(should_block_save_for_secrets(true, false, contains_secrets));

    let mixed_mcp = r#"{
      "local": {
        "type": "http",
        "url": "http://127.0.0.1:8080",
        "key": "sk-123456789012345678901234567890123456789012345678"
      }
    }"#;
    let contains_secrets = !find_secrets_in_text_excluding_ips(mixed_mcp).is_empty();
    assert!(contains_secrets);
    assert!(should_block_save_for_secrets(true, true, contains_secrets));
}
