use warpui::async_assert_eq;
use warpui::integration::{AssertionCallback, AssertionOutcome};

use crate::ai::agent::redaction::redact_secrets;
use crate::ai::blocklist::block::secret_redaction::find_secrets_in_text;
use crate::integration_testing::view_getters::single_terminal_view;
use crate::terminal::safe_mode_settings::get_secret_obfuscation_mode;

pub fn assert_secret_tooltip_open(open: bool) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view(app, window_id);
        let error_message = if open {
            "The secret tooltip should be open"
        } else {
            "The secret tooltip should not be open"
        };
        terminal_view.read(app, |view, _ctx| {
            async_assert_eq!(view.is_secret_tooltip_open(), open, "{}", error_message)
        })
    })
}

/// Assert that secrets are properly redacted for AI conversations in both modes
pub fn assert_secrets_redacted_for_ai(
    test_text: String,
    expected_phone_redaction: String,
    expected_api_key_redaction: String,
    original_phone: String,
    original_api_key: String,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view(app, window_id);
        terminal_view.read(app, |_view, ctx| {
            let secret_redaction_mode = get_secret_obfuscation_mode(ctx);

            // Test that we properly detect secrets in the input
            let detected_secrets = find_secrets_in_text(&test_text);
            if detected_secrets.is_empty() {
                return AssertionOutcome::failure(format!(
                    "Should detect secrets in test text: {test_text}"
                ));
            }

            // Test that redaction works for both modes when sending to AI
            if secret_redaction_mode.should_redact_secret() {
                let mut redacted_text = test_text.clone();
                redact_secrets(&mut redacted_text);

                if !redacted_text.contains(&expected_phone_redaction) {
                    return AssertionOutcome::failure(format!(
                        "Phone number should be redacted in text sent to AI: {redacted_text}"
                    ));
                }
                if !redacted_text.contains(&expected_api_key_redaction) {
                    return AssertionOutcome::failure(format!(
                        "API key should be redacted in text sent to AI: {redacted_text}"
                    ));
                }
                if redacted_text.contains(&original_phone) {
                    return AssertionOutcome::failure(format!(
                        "Original phone number should not appear in redacted text: {redacted_text}"
                    ));
                }
                if redacted_text.contains(&original_api_key) {
                    return AssertionOutcome::failure(format!(
                        "Original API key should not appear in redacted text: {redacted_text}"
                    ));
                }
            }

            AssertionOutcome::Success
        })
    })
}
