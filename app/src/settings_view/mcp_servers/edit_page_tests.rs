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
