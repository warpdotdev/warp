use super::*;

#[test]
fn timed_out_error_copy_leads_with_timeout() {
    let error = Error::TimedOut.user_facing_error(SetupStage::Launch);

    assert_eq!(error.body, "Timed out while trying to start SSH extension");
    assert_eq!(
        error.detail.as_deref(),
        Some(
            "Warp stopped waiting for the SSH extension to respond. Your SSH session is \
             still running, but advanced features are unavailable for now."
        )
    );
}

#[test]
fn non_timeout_error_copy_still_leads_with_failure() {
    let error = Error::UnsupportedOs {
        os: "plan9".to_string(),
    }
    .user_facing_error(SetupStage::CheckBinary);

    assert_eq!(error.body, "Failed to verify SSH extension");
    assert_eq!(error.detail.as_deref(), Some("Unsupported OS: plan9"));
}
