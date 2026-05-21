use regex::Regex;
use serial_test::serial;
use warpui::elements::Text;
use warpui::fonts::FamilyId;

use crate::terminal::model::secrets::{self, SecretLevel};

use super::*;

#[test]
fn test_merge_no_ranges() {
    let ranges: Vec<(SecretRange, SecretLevel)> = vec![];
    let result = merge_sorted_ranges_with_levels(ranges);
    assert_eq!(result, Vec::<(SecretRange, SecretLevel)>::new());
}

#[test]
fn test_merge_single_range() {
    let ranges: Vec<(SecretRange, SecretLevel)> = vec![(
        SecretRange {
            char_range: 0..5,
            byte_range: 0..5,
        },
        SecretLevel::User,
    )];
    let result = merge_sorted_ranges_with_levels(ranges);
    assert_eq!(
        result,
        vec![(
            SecretRange {
                char_range: 0..5,
                byte_range: 0..5,
            },
            SecretLevel::User
        )]
    );
}

#[test]
fn test_merge_non_overlapping_ranges() {
    let ranges = vec![
        (
            SecretRange {
                char_range: 0..3,
                byte_range: 0..3,
            },
            SecretLevel::User,
        ),
        (
            SecretRange {
                char_range: 5..8,
                byte_range: 5..8,
            },
            SecretLevel::User,
        ),
        (
            SecretRange {
                char_range: 10..15,
                byte_range: 10..15,
            },
            SecretLevel::Enterprise,
        ),
    ];
    let result = merge_sorted_ranges_with_levels(ranges);
    assert_eq!(
        result,
        vec![
            (
                SecretRange {
                    char_range: 0..3,
                    byte_range: 0..3,
                },
                SecretLevel::User
            ),
            (
                SecretRange {
                    char_range: 5..8,
                    byte_range: 5..8,
                },
                SecretLevel::User
            ),
            (
                SecretRange {
                    char_range: 10..15,
                    byte_range: 10..15,
                },
                SecretLevel::Enterprise
            )
        ]
    );
}

#[test]
fn test_merge_overlapping_ranges() {
    let ranges = vec![
        (
            SecretRange {
                char_range: 0..5,
                byte_range: 0..5,
            },
            SecretLevel::User,
        ),
        (
            SecretRange {
                char_range: 3..10,
                byte_range: 3..10,
            },
            SecretLevel::Enterprise,
        ),
    ];
    let result = merge_sorted_ranges_with_levels(ranges);
    assert_eq!(
        result,
        vec![(
            SecretRange {
                char_range: 0..10,
                byte_range: 0..10,
            },
            SecretLevel::Enterprise
        )]
    );
}

#[test]
fn test_merge_adjacent_ranges() {
    let ranges = vec![
        (
            SecretRange {
                char_range: 0..5,
                byte_range: 0..5,
            },
            SecretLevel::User,
        ),
        (
            SecretRange {
                char_range: 5..10,
                byte_range: 5..10,
            },
            SecretLevel::User,
        ),
    ];
    let result = merge_sorted_ranges_with_levels(ranges);
    assert_eq!(
        result,
        vec![(
            SecretRange {
                char_range: 0..10,
                byte_range: 0..10,
            },
            SecretLevel::User
        )]
    );
}

#[test]
fn test_merge_complex_merge() {
    let ranges = vec![
        (
            SecretRange {
                char_range: 1..3,
                byte_range: 1..3,
            },
            SecretLevel::User,
        ),
        (
            SecretRange {
                char_range: 2..5,
                byte_range: 2..5,
            },
            SecretLevel::Enterprise,
        ),
        (
            SecretRange {
                char_range: 6..8,
                byte_range: 6..8,
            },
            SecretLevel::User,
        ),
        (
            SecretRange {
                char_range: 7..10,
                byte_range: 7..10,
            },
            SecretLevel::Enterprise,
        ),
        (
            SecretRange {
                char_range: 12..15,
                byte_range: 12..15,
            },
            SecretLevel::User,
        ),
        (
            SecretRange {
                char_range: 14..18,
                byte_range: 14..18,
            },
            SecretLevel::User,
        ),
    ];
    let result = merge_sorted_ranges_with_levels(ranges);
    assert_eq!(
        result,
        vec![
            (
                SecretRange {
                    char_range: 1..5,
                    byte_range: 1..5,
                },
                SecretLevel::Enterprise
            ),
            (
                SecretRange {
                    char_range: 6..10,
                    byte_range: 6..10,
                },
                SecretLevel::Enterprise
            ),
            (
                SecretRange {
                    char_range: 12..18,
                    byte_range: 12..18,
                },
                SecretLevel::User
            )
        ]
    );
}

#[test]
fn test_merge_ranges_with_same_start() {
    let ranges = vec![
        (
            SecretRange {
                char_range: 1..5,
                byte_range: 1..5,
            },
            SecretLevel::User,
        ),
        (
            SecretRange {
                char_range: 1..3,
                byte_range: 1..3,
            },
            SecretLevel::User,
        ),
        (
            SecretRange {
                char_range: 1..4,
                byte_range: 1..4,
            },
            SecretLevel::Enterprise,
        ),
    ];
    let result = merge_sorted_ranges_with_levels(ranges);
    assert_eq!(
        result,
        vec![(
            SecretRange {
                char_range: 1..5,
                byte_range: 1..5,
            },
            SecretLevel::Enterprise
        )]
    );
}

#[test]
fn test_merge_ranges_with_same_end() {
    let ranges = vec![
        (
            SecretRange {
                char_range: 0..5,
                byte_range: 0..5,
            },
            SecretLevel::User,
        ),
        (
            SecretRange {
                char_range: 2..5,
                byte_range: 2..5,
            },
            SecretLevel::Enterprise,
        ),
        (
            SecretRange {
                char_range: 3..5,
                byte_range: 3..5,
            },
            SecretLevel::User,
        ),
    ];
    let result = merge_sorted_ranges_with_levels(ranges);
    assert_eq!(
        result,
        vec![(
            SecretRange {
                char_range: 0..5,
                byte_range: 0..5,
            },
            SecretLevel::Enterprise
        )]
    );
}

// Secret detection now only uses user-defined custom regexes that are populated when safe mode is enabled.
// Within this set of tests, we focus on testing the detect_secrets function that uses user-defined regexes,
// rather than system default regexes.

#[test]
fn test_detect_secrets_no_regexes_configured() {
    // With no regexes configured, no secrets should be detected
    let text = "foo warp-server-staging.firebaseapp.com bar";
    let detected_secrets = find_secrets_in_text(text);
    assert_eq!(detected_secrets, vec![]);
}

// #[serial] is used to ensure custom regexes state does not interfere with other tests,
// as the custom regexes are global state.

#[test]
#[serial]
fn test_detect_secrets_single_secret_custom() {
    // Set as user secret (enterprise secrets is empty)
    secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("ABCD").expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    let text = "foo ABCD bar";
    let detected_secrets = find_secrets_in_text(text);
    assert_eq!(
        detected_secrets,
        vec![SecretRange {
            char_range: 4..8,
            byte_range: 4..8,
        }]
    );
}

#[test]
#[serial]
fn test_detect_secrets_single_secret_custom_with_multibyte() {
    // Set a custom secret regex that matches a Chinese multibyte secret, e.g., "秘密"
    // Set as user secret (enterprise secrets is empty)
    secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("秘密").expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    let text = "foo 秘密 bar";
    let detected_secrets = find_secrets_in_text(text);

    // The Chinese secret "秘密" starts at character index 4 and ends at character index 6
    assert_eq!(
        detected_secrets,
        vec![SecretRange {
            char_range: 4..6,
            byte_range: 4..10, // multibyte chars take multiple bytes
        }]
    );
}

#[test]
#[serial]
fn test_detect_secrets_multiple_secrets() {
    // Set custom regexes to include patterns that would previously have been system defaults
    secrets::set_user_and_enterprise_secret_regexes(
        [
            &Regex::new("ABCD").expect("Should be able to construct regex"),
            &Regex::new(r"\bghp_[A-Za-z0-9_]{36}\b").expect("Should be able to construct regex"),
            &Regex::new(r"\b([a-z0-9-]){1,30}(\.firebaseapp\.com)\b")
                .expect("Should be able to construct regex"),
            &Regex::new(r"\b(?:r|s)k_(test|live)_[0-9a-zA-Z]{24}\b")
                .expect("Should be able to construct regex"),
        ],
        std::iter::empty(), // No enterprise secrets
    );

    // Using custom secret, github token, firebase domain, and stripe key as secrets.
    let text = "ABCD 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 foo baz warp-server-staging.firebaseapp.com bar \n foo sk_live_4eC39HqLyjWDarjtT1zdp7dc qux foo";
    let detected_secrets = find_secrets_in_text(text);
    assert_eq!(
        detected_secrets,
        vec![
            SecretRange {
                char_range: 0..4,
                byte_range: 0..4,
            },
            SecretRange {
                char_range: 5..45,
                byte_range: 5..45,
            },
            SecretRange {
                char_range: 54..89,
                byte_range: 54..89,
            },
            SecretRange {
                char_range: 100..132,
                byte_range: 100..132,
            }
        ]
    );
}

#[test]
fn test_add_secret_redaction_to_text_no_secrets() {
    let text = Text::new_inline("This is a test.", FamilyId(0), 12.0);
    let detected_secrets_in_location = DetectedSecretsInTextLocation::default();
    let location = TextLocation::Output {
        section_index: 0,
        line_index: 0,
    };

    let original_text = text.text().to_owned();

    let result = redact_secrets_in_element(text, &detected_secrets_in_location, location, true);

    // No changes should be made to the text.
    assert_eq!(result.text().to_owned(), original_text);
}

#[test]
fn test_add_secret_redaction_to_text_with_redaction() {
    let text = Text::new_inline("This is a secret: secret123.", FamilyId(0), 12.0);
    let location = TextLocation::Output {
        section_index: 0,
        line_index: 0,
    };

    let secret_range = SecretRange {
        char_range: 18..27, // "secret123"
        byte_range: 18..27,
    };
    let hoverable_secret = Secret {
        secret: "secret123".to_owned(),
        is_obfuscated: true,
        mouse_state: Default::default(),
        secret_level: SecretLevel::User,
    };

    let mut detected_secrets_in_location = DetectedSecretsInTextLocation::default();
    detected_secrets_in_location
        .detected_secrets
        .insert(secret_range.clone(), hoverable_secret);

    let result = redact_secrets_in_element(text, &detected_secrets_in_location, location, true);

    // The secret should be replaced with asterisks.
    assert_eq!(result.text(), "This is a secret: *********.");
}

#[test]
fn test_add_secret_redaction_to_text_with_multibyte_characters() {
    // Text with multibyte characters (e.g., Chinese characters).
    let text = Text::new_inline("这是一个秘密: 密码1234.", FamilyId(0), 12.0);
    let location = TextLocation::Output {
        section_index: 0,
        line_index: 0,
    };

    // Range for the secret "码1234" in the multibyte text.
    let secret_range = SecretRange {
        char_range: 9..14,  // "码1234"
        byte_range: 23..30, // Byte range will be larger due to multibyte characters
    };
    let hoverable_secret = Secret {
        secret: "码1234".to_owned(),
        is_obfuscated: true,
        mouse_state: Default::default(),
        secret_level: SecretLevel::User,
    };

    let mut detected_secrets_in_location = DetectedSecretsInTextLocation::default();
    detected_secrets_in_location
        .detected_secrets
        .insert(secret_range.clone(), hoverable_secret);

    let result = redact_secrets_in_element(text, &detected_secrets_in_location, location, true);

    // The secret should be replaced with asterisks.
    assert_eq!(result.text(), "这是一个秘密: 密*****.");
}

// Test case-sensitive matching by default
#[test]
#[serial]
fn test_detect_secrets_case_sensitive() {
    // Set as user secret (enterprise secrets is empty)
    secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("ABCD").expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    // Should match exact case
    let text = "foo ABCD bar";
    let detected_secrets = find_secrets_in_text(text);
    assert_eq!(
        detected_secrets,
        vec![SecretRange {
            char_range: 4..8,
            byte_range: 4..8,
        }]
    );

    // Should not match different case
    let text = "foo abcd bar";
    let detected_secrets = find_secrets_in_text(text);
    assert_eq!(detected_secrets, vec![]);
}

// Test opt-in case-insensitive matching with (?i) flag
#[test]
#[serial]
fn test_detect_secrets_case_insensitive_opt_in() {
    // Set as user secret with case-insensitive flag
    secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("(?i)ABCD").expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    // Should match both cases when case-insensitive flag is used
    let text = "foo ABCD bar abcd baz";
    let detected_secrets = find_secrets_in_text(text);
    assert_eq!(
        detected_secrets,
        vec![
            SecretRange {
                char_range: 4..8,
                byte_range: 4..8,
            },
            SecretRange {
                char_range: 13..17,
                byte_range: 13..17,
            }
        ]
    );
}

// Test case sensitivity for default regex patterns
#[test]
#[serial]
fn test_detect_secrets_default_regex_case_sensitivity() {
    // Set user secret with a stripe-key like pattern, but enforce case sensitivity
    secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new(r"\bsk_test_[0-9a-z]{24}\b").expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    // Only matches keys that use lowercase
    let text = "API keys: sk_test_abcdef123456789012345678 SK_TEST_ABCDEF123456789012345678";
    let detected_secrets = find_secrets_in_text(text);
    assert_eq!(
        detected_secrets,
        vec![SecretRange {
            char_range: 10..42,
            byte_range: 10..42,
        }]
    );

    // When we want case-insensitive matching, we explicitly use [A-Za-z]
    secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new(r"\bsk_test_[0-9A-Za-z]{24}\b").expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    // Now matches both cases because of the explicit character class [A-Za-z]
    let text = "API keys: sk_test_abcdef123456789012345678 sk_test_ABCDEF123456789012345678";
    let detected_secrets = find_secrets_in_text(text);
    assert_eq!(
        detected_secrets,
        vec![
            SecretRange {
                char_range: 10..42,
                byte_range: 10..42,
            },
            SecretRange {
                char_range: 43..75,
                byte_range: 43..75,
            }
        ]
    );
}

// Regression test for panic `assertion failed: self.is_char_boundary(n)`.
// End-to-end detection and redaction of custom multibyte secrets.
#[test]
#[serial]
fn test_detect_and_redact_custom_multibyte_secrets() {
    // Set the custom secret regex to detect both "テストファイル" and "ABCD"
    // Set as user secrets (enterprise secrets is empty)
    secrets::set_user_and_enterprise_secret_regexes(
        [
            &Regex::new("テストファイル").expect("Should be able to construct regex"),
            &Regex::new("ABCD").expect("Should be able to construct regex"),
        ],
        std::iter::empty(), // No enterprise secrets
    );
    let text = "これはテストファイルです。 ABCD";

    // Step 1: Detect secrets in the text
    let detected_secrets = find_secrets_in_text(text);
    assert_eq!(
        detected_secrets,
        vec![
            SecretRange {
                char_range: 3..10, // "テストファイル"
                byte_range: 9..30, // Multibyte character byte range
            },
            SecretRange {
                char_range: 14..18, // "ABCD"
                byte_range: 40..44,
            }
        ]
    );

    // Step 2: Prepare for redaction by inserting the detected secrets
    let location = TextLocation::Output {
        section_index: 0,
        line_index: 0,
    };

    let mut detected_secrets_in_location = DetectedSecretsInTextLocation::default();

    for secret_range in detected_secrets.iter() {
        let hoverable_secret = Secret {
            secret: text[secret_range.byte_range.clone()].to_owned(),
            is_obfuscated: true,
            mouse_state: Default::default(),
            secret_level: SecretLevel::User,
        };
        detected_secrets_in_location
            .detected_secrets
            .insert(secret_range.clone(), hoverable_secret);
    }

    // Step 3: Redact the secrets in the text
    let text_obj = Text::new_inline(text, FamilyId(0), 12.0);
    let redacted_text =
        redact_secrets_in_element(text_obj, &detected_secrets_in_location, location, true);

    // The expected result after redaction
    let expected_redacted_text = "これは*******です。 ****";

    assert_eq!(redacted_text.text(), expected_redacted_text);
}
