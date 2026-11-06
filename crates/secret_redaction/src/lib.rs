//! Secret detection and redaction. This crate owns the compiled secret regexes, the default
//! secret patterns, and helpers that find and redact secrets in text.

use std::sync::Arc;

use itertools::Itertools;
use lazy_static::lazy_static;
use parking_lot::Mutex;
use regex_dfas::RegexDFAs;
use string_offset::StringRange;
use warp_core::safe_warn;
use warp_errors::report_error;

/// The character used to replace each redacted character of a detected secret.
pub const SECRET_REDACTION_REPLACEMENT_CHARACTER: &str = "*";

pub trait RegexDisplayInfo {
    fn pattern(&self) -> &str;
    fn name(&self) -> Option<&str>;
}
/// A regex pattern that can be used to detect secrets in text.
pub struct SecretsRegex {
    /// The regex pattern to match secrets in strings.  This is a meta::Regex which supports
    /// multiple patterns.
    pub regex: regex_automata::meta::Regex,

    /// The DFAs used to search for secrets in the grid.
    pub dfas: RegexDFAs,

    /// Metadata about the regex pattern, including which secret levels it corresponds to.
    pub level_metadata: RegexLevelMetadata,
}

/// Tracks counts to infer which regex patterns correspond to which secret levels
#[derive(Debug, Clone)]
pub struct RegexLevelMetadata {
    /// Number of enterprise regex patterns (they are added first)
    pub enterprise_count: usize,
    /// Number of user regex patterns (they are added after enterprise patterns)
    pub user_count: usize,
}

lazy_static! {
    /// The information needed to search for secrets in strings or terminal grids.
    ///
    /// These are initially empty, and will be populated with regexes when safe mode is enabled.
    ///
    /// This is wrapped in an Arc so that readers can clone it cheaply to keep the critical section
    /// short, allowing writers to set a new set of regexes for future readers without being blocked
    /// on any users of the old patterns.
    pub static ref SECRETS_REGEX: Mutex<Arc<SecretsRegex>> = Mutex::new(
        Arc::new(SecretsRegex {
            regex: regex_automata::meta::Regex::new_many(&[] as &[&str])
                .expect("should be able to construct empty regex"),
            dfas: RegexDFAs::new_many(&[], false, true).expect("should be able to construct empty regex DFA"),
            level_metadata: RegexLevelMetadata {
                enterprise_count: 0,
                user_count: 0,
            },
        })
    );
}

/// Represents the level/source of a secret redaction rule
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SecretLevel {
    /// User-defined custom secret patterns
    User,
    /// Enterprise/organization-defined secret patterns
    Enterprise,
}

impl SecretLevel {
    /// Returns true if this is an enterprise level secret
    pub fn is_enterprise(self) -> bool {
        matches!(self, SecretLevel::Enterprise)
    }

    /// Returns true if this is a user level secret
    pub fn is_user(self) -> bool {
        matches!(self, SecretLevel::User)
    }

    /// Returns the priority of the secret level. Enterprise has highest priority.
    pub fn priority(self) -> u8 {
        match self {
            SecretLevel::User => 0,
            SecretLevel::Enterprise => 1,
        }
    }
}

/// Updates secret scanning with a new set of user-defined and enterprise regexes.
///
/// The implementation here ensures enterprise secrets are handled differently, maintaining separation
/// from the user's configuration in their settings.
///
/// If the internal [`RegexDFAs`] or [`regex_automata::meta::Regex`] can't be constructed from the
/// new regexes for any reason, the current regexes are kept unchanged.
pub fn set_user_and_enterprise_secret_regexes<'a>(
    user_secrets: impl IntoIterator<Item = &'a regex::Regex>,
    enterprise_secrets: impl IntoIterator<Item = &'a regex::Regex>,
) {
    // Collect enterprise and user secrets into vectors to count them
    let enterprise_secrets_vec: Vec<&'a regex::Regex> = enterprise_secrets.into_iter().collect();
    let user_secrets_vec: Vec<&'a regex::Regex> = user_secrets.into_iter().collect();

    // Dedup user regex entries against enterprise regexes to improve performance
    let mut seen_patterns: std::collections::HashSet<&str> =
        enterprise_secrets_vec.iter().map(|r| r.as_str()).collect();

    let filtered_user_secrets_vec: Vec<&'a regex::Regex> = user_secrets_vec
        .into_iter()
        .filter(|r| seen_patterns.insert(r.as_str()))
        .collect();

    // Combine all secrets additively: enterprise first (highest priority), then filtered user
    let all_secrets = enterprise_secrets_vec
        .iter()
        .map(|regex| regex.as_str())
        .chain(filtered_user_secrets_vec.iter().map(|regex| regex.as_str()))
        .collect_vec();

    // Make sure we can compile both the regex and the DFA before we attempt to replace the live
    // ones.
    let dfas = match RegexDFAs::new_many(&all_secrets, false, true) {
        Ok(dfas) => dfas,
        Err(err) => {
            safe_warn!(
                safe: ("Failed to construct new RegexDFA with combined secrets"),
                full: ("Failed to construct new RegexDFA with combined secrets: {err:#}")
            );
            return;
        }
    };
    let secrets_regex = match regex_automata::meta::Regex::new_many(&all_secrets) {
        Ok(regex) => SecretsRegex {
            regex,
            dfas,
            level_metadata: RegexLevelMetadata {
                enterprise_count: enterprise_secrets_vec.len(),
                user_count: filtered_user_secrets_vec.len(),
            },
        },
        Err(err) => {
            safe_warn!(
                safe: ("Failed to construct new Regex with combined secrets"),
                full: ("Failed to construct new Regex with combined secrets: {err:#}")
            );
            return;
        }
    };

    // Store a shareable reference to the new compiled regex, DFAs, and metadata.
    *SECRETS_REGEX.lock() = Arc::new(secrets_regex);
}

/// Returns the ranges of detected secrets in the given text.
pub fn find_secrets_in_text(text: &str) -> Vec<StringRange> {
    find_secrets_in_text_with_levels(text)
        .into_iter()
        .map(|(range, _level)| range)
        .collect()
}

/// Returns the ranges of detected secrets in the given text along with their SecretLevel.
pub fn find_secrets_in_text_with_levels(text: &str) -> Vec<(StringRange, SecretLevel)> {
    let secrets_regex: Arc<SecretsRegex> = { SECRETS_REGEX.lock().clone() };

    find_secrets_in_text_with_levels_using_regex(text, &secrets_regex)
}

pub fn find_secrets_in_text_with_levels_using_regex(
    text: &str,
    secrets_regex: &SecretsRegex,
) -> Vec<(StringRange, SecretLevel)> {
    let SecretsRegex {
        regex,
        level_metadata,
        ..
    } = secrets_regex;

    let mut secret_ranges = vec![];
    let mut byte_to_char_index = vec![0; text.len() + 1]; // Map byte index to char index

    // Track the current character index while iterating through the string.
    let mut char_index = 0;
    for (byte_index, _) in text.char_indices() {
        byte_to_char_index[byte_index] = char_index;
        char_index += 1;
    }
    byte_to_char_index[text.len()] = char_index; // Map the last byte to the last character index

    // Iterate over the text once, finding all matches against secret regex. Map the byte ranges
    // to character ranges and store them.
    for mat in regex.find_iter(text) {
        let start_byte = mat.start();
        let end_byte = mat.end();
        let start_char = byte_to_char_index[start_byte];
        let end_char = byte_to_char_index[end_byte];

        // Determine which pattern matched by getting the pattern ID and map via counts
        let pattern_id = mat.pattern().as_usize();
        let total_patterns = level_metadata.enterprise_count + level_metadata.user_count;
        if pattern_id >= total_patterns {
            report_error!(
                "Secret level not found for pattern ID",
                extra: { "pattern_id" => %pattern_id }
            );
            continue;
        }
        let secret_level = if pattern_id < level_metadata.enterprise_count {
            SecretLevel::Enterprise
        } else {
            SecretLevel::User
        };

        secret_ranges.push((
            StringRange {
                char_range: start_char..end_char,
                byte_range: start_byte..end_byte,
            },
            secret_level,
        ));
    }

    // Merge overlapping ranges, preserving the highest priority SecretLevel
    merge_sorted_ranges_with_levels(secret_ranges)
}

/// Merges overlapping ranges while preserving the highest priority SecretLevel
pub fn merge_sorted_ranges_with_levels(
    ranges: Vec<(StringRange, SecretLevel)>,
) -> Vec<(StringRange, SecretLevel)> {
    if ranges.is_empty() {
        return ranges;
    }

    let mut merged_ranges = vec![];
    let mut current_range = ranges[0].0.clone();
    let mut current_level = ranges[0].1;

    for (range, level) in ranges.into_iter().skip(1) {
        // We can merge based on character ranges since non-overlapping character ranges result in non-overlapping byte ranges.
        if range.char_range.start <= current_range.char_range.end {
            // Extend the current range to include the overlapping range.
            current_range.extend_range_end(&range);
            // Keep the highest priority level
            if level.priority() > current_level.priority() {
                current_level = level;
            }
        } else {
            // No overlap, push the current range and move to the next.
            merged_ranges.push((current_range, current_level));
            current_range = range;
            current_level = level;
        }
    }

    // Add the last range.
    merged_ranges.push((current_range, current_level));

    merged_ranges
}

/// Redact all detected secrets in-place within the given string.
pub fn redact_secrets(input: &mut String) {
    let mut secrets: Vec<_> = find_secrets_in_text(input)
        .into_iter()
        .map(|r| r.byte_range)
        .collect();
    // Replace from the end to preserve indices
    secrets.sort_by_key(|range| range.start);
    for range in secrets.into_iter().rev() {
        let replacement =
            SECRET_REDACTION_REPLACEMENT_CHARACTER.repeat(range.end.saturating_sub(range.start));
        input.replace_range(range.start..range.end, &replacement);
    }
}

pub mod regexes {
    use super::RegexDisplayInfo;

    /// A default regex pattern with its descriptive name
    pub struct DefaultRegex {
        pub pattern: &'static str,
        pub name: &'static str,
    }

    impl RegexDisplayInfo for DefaultRegex {
        fn pattern(&self) -> &str {
            self.pattern
        }

        fn name(&self) -> Option<&str> {
            Some(self.name)
        }
    }

    impl RegexDisplayInfo for &DefaultRegex {
        fn pattern(&self) -> &str {
            self.pattern
        }

        fn name(&self) -> Option<&str> {
            Some(self.name)
        }
    }
    /// Identifies an IPv4 address. Source: <https://stackoverflow.com/questions/5284147/validating-ipv4-addresses-with-regexp>.
    pub const IPV4_ADDRESS: &str = r"\b((25[0-5]|(2[0-4]|1\d|[1-9]|)\d)\.?\b){4}\b";

    /// Identifies an IPv6 address. Source: <https://regex101.com/library/aL7tV3?orderBy=RELEVANCE&search=ip>
    pub const IPV6_ADDRESS: &str =
        r"\b((([0-9A-Fa-f]{1,4}:){1,6}:)|(([0-9A-Fa-f]{1,4}:){7}))([0-9A-Fa-f]{1,4})\b";

    /// Identifies a phone number. Source: <https://stackoverflow.com/questions/16699007/regular-expression-to-match-standard-10-digit-phone-number>.
    /// NOTE: This does not match 10 digit unformatted numbers (e.g. 1234567890) because it would trigger many false positive matches.
    pub const PHONE_NUMBER: &str = r"\b(\+\d{1,2}\s)?\(?\d{3}\)?[\s.-]\d{3}[\s.-]\d{4}\b";

    /// Identifies a MAC Address. Source: <https://stackoverflow.com/questions/4260467/what-is-a-regular-expression-for-a-mac-address>.
    pub const MAC_ADDRESS: &str =
        r"\b((([a-zA-z0-9]{2}[-:]){5}([a-zA-z0-9]{2}))|(([a-zA-z0-9]{2}:){5}([a-zA-z0-9]{2})))\b";

    /// Identifies a Google API Key. Source: <https://github.com/odomojuli/RegExAPI>.
    pub const GOOGLE_API_KEY: &str = r"\bAIza[0-9A-Za-z-_]{35}\b";

    /// Identifies an OpenAI API Key.
    /// Source: <https://platform.openai.com/account/api-keys>
    pub const OPENAI_API_KEY: &str = r"\bsk-[a-zA-Z0-9]{48}\b";

    /// Identifies an Anthropic API Key. Supports current and possible future formats,
    /// such as sk-ant-api03-... with variable-length body including alphanumerics and hyphens.
    /// Based on current observed format lengths (~96 chars), but allows 80–120 as buffer.
    pub const ANTHROPIC_API_KEY: &str = r"\bsk-ant-api\d{0,2}-[a-zA-Z0-9\-]{80,120}\b";

    /// Identifies a general `sk-` style API key (e.g., OpenAI, Anthropic).
    /// Accepts a wide range of formats with alphanumeric and hyphen characters,
    /// with a length buffer between 10–100 characters.
    ///
    /// Used in case providers update their API key format.
    pub const GENERIC_SK_API_KEY: &str = r"\bsk-[a-zA-Z0-9\-]{10,100}\b";

    /// Identifies a Fireworks API Key. Format: fw_ followed by 24 alphanumeric characters.
    pub const FIREWORKS_API_KEY: &str = r"\bfw_[a-zA-Z0-9]{24}\b";

    /// Identifies an AWS Access ID.
    pub const AWS_ACCESS_ID: &str =
        r"\b(AKIA|A3T|AGPA|AIDA|AROA|AIPA|ANPA|ANVA|ASIA)[A-Z0-9]{12,}\b";

    /// Identifies a Slack app token.
    pub const SLACK_APP_TOKEN: &str = r"\bxapp-[0-9]+-[A-Za-z0-9_]+-[0-9]+-[a-f0-9]+\b";

    /// The following identify github tokens. Source: <https://github.com/odomojuli/RegExAPI>
    /// and source of `[A-Za-z0-9_]` character set is <https://github.blog/changelog/2021-03-31-authentication-token-format-updates-are-generally-available/>
    pub const GITHUB_CLASSIC_PERSONAL_ACCESS_TOKEN: &str = r"\bghp_[A-Za-z0-9_]{36}\b";
    pub const GITHUB_FINE_GRAINED_PERSONAL_ACCESS_TOKEN: &str = r"\bgithub_pat_[A-Za-z0-9_]{82}\b";
    pub const GITHUB_OAUTH_ACCESS_TOKEN: &str = r"\bgho_[A-Za-z0-9_]{36}\b";
    pub const GITHUB_USER_TO_SERVER_TOKEN: &str = r"\bghu_[A-Za-z0-9_]{36}\b";
    pub const GITHUB_SERVER_TO_SERVER_TOKEN: &str = r"\bghs_[A-Za-z0-9_]{36}\b";

    /// Identifies Stripe API Keys. Source: <https://github.com/l4yton/RegHex#stripe-api-key>
    pub const STRIPE_KEY: &str = r"\b(?:r|s)k_(test|live)_[0-9a-zA-Z]{24}\b";

    /// Identifies a Firebase Auth Domain.
    pub const FIREBASE_AUTH_DOMAIN: &str = r"\b([a-z0-9-]){1,30}(\.firebaseapp\.com)\b";

    /// Identifies a JSON web token (JWT). Source: <https://en.wikipedia.org/wiki/JSON_Web_Token>
    /// "ey" is the beginning of the patterns for the header and claims b/c that is:
    /// echo -n '{"' | base64
    /// We know those sections are JSON and should begin with '{"'.
    pub const JWT: &str = r"\b(ey[a-zA-z0-9_\-=]{10,}\.){2}[a-zA-z0-9_\-=]{10,}\b";

    /// Identifies a Warp API Key. Format: wk- followed by a version number and any combination of hex digits, hyphens, or periods.
    pub const WARP_API_KEY: &str = r"\bwk-[0-9]+\.[A-Fa-f0-9.\-]+\b";

    /// Returns a slice of regex strings that can be used to identify secrets.
    // NOTE: All regexes added here must also be added server-side in logic/ai/util.go.
    pub const DEFAULT_REGEXES_WITH_NAMES: &[DefaultRegex] = &[
        DefaultRegex {
            pattern: IPV4_ADDRESS,
            name: "IPv4 Address",
        },
        DefaultRegex {
            pattern: IPV6_ADDRESS,
            name: "IPv6 Address",
        },
        DefaultRegex {
            pattern: PHONE_NUMBER,
            name: "Phone Number",
        },
        DefaultRegex {
            pattern: MAC_ADDRESS,
            name: "MAC Address",
        },
        DefaultRegex {
            pattern: GOOGLE_API_KEY,
            name: "Google API Key",
        },
        DefaultRegex {
            pattern: AWS_ACCESS_ID,
            name: "AWS Access ID",
        },
        DefaultRegex {
            pattern: SLACK_APP_TOKEN,
            name: "Slack App Token",
        },
        DefaultRegex {
            pattern: GITHUB_CLASSIC_PERSONAL_ACCESS_TOKEN,
            name: "GitHub Classic Personal Access Token",
        },
        DefaultRegex {
            pattern: GITHUB_FINE_GRAINED_PERSONAL_ACCESS_TOKEN,
            name: "GitHub Fine-Grained Personal Access Token",
        },
        DefaultRegex {
            pattern: GITHUB_OAUTH_ACCESS_TOKEN,
            name: "GitHub OAuth Access Token",
        },
        DefaultRegex {
            pattern: GITHUB_USER_TO_SERVER_TOKEN,
            name: "GitHub User-to-Server Token",
        },
        DefaultRegex {
            pattern: GITHUB_SERVER_TO_SERVER_TOKEN,
            name: "GitHub Server-to-Server Token",
        },
        DefaultRegex {
            pattern: STRIPE_KEY,
            name: "Stripe Key",
        },
        DefaultRegex {
            pattern: FIREBASE_AUTH_DOMAIN,
            name: "Firebase Auth Domain",
        },
        DefaultRegex {
            pattern: JWT,
            name: "JWT",
        },
        DefaultRegex {
            pattern: OPENAI_API_KEY,
            name: "OpenAI API Key",
        },
        DefaultRegex {
            pattern: ANTHROPIC_API_KEY,
            name: "Anthropic API Key",
        },
        DefaultRegex {
            pattern: GENERIC_SK_API_KEY,
            name: "Generic SK API Key",
        },
        DefaultRegex {
            pattern: FIREWORKS_API_KEY,
            name: "Fireworks API Key",
        },
        DefaultRegex {
            pattern: WARP_API_KEY,
            name: "Warp API Key",
        },
    ];
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
