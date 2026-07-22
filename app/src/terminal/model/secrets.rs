#![allow(dead_code)]

use std::collections::HashMap;
use std::ops::{Not, RangeInclusive};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::anyhow;
use itertools::Itertools;
use lazy_static::lazy_static;
use parking_lot::Mutex;
use rangemap::{RangeInclusiveMap, StepLite};
use warpui::EntityId;
use warpui::elements::SecretRange;

use super::grid::grid_handler::GridHandler;
use super::grid::{Dimensions as _, RespectDisplayedOutput};
use super::terminal_model::RangeInModel;
use crate::ai::blocklist::TextLocation;
use crate::safe_warn;
use crate::terminal::model::find::RegexDFAs;
use crate::terminal::model::index::Point;

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

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd)]
/// A handle to a [`Secret`].
pub struct SecretHandle(usize);

impl SecretHandle {
    pub(super) fn next() -> Self {
        static SECRET_HANDLE: AtomicUsize = AtomicUsize::new(0);
        let next = SECRET_HANDLE.fetch_add(1, Ordering::Relaxed);
        SecretHandle(next)
    }

    pub fn id(&self) -> String {
        format!("{}", self.0)
    }
}

#[derive(Clone, Debug)]
pub struct RichContentSecretTooltipInfo {
    pub secret: String,
    pub secret_range: SecretRange,
    pub location: TextLocation,
    pub is_obfuscated: bool,
    pub position_id: String,
    pub view_id: EntityId,
    pub secret_level: SecretLevel,
}

#[derive(Copy, Clone, Debug)]
pub enum IsObfuscated {
    Yes,
    No,
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

/// Whether or not to respect obfuscated secrets when retrieving grid contents.
#[derive(Copy, Clone, PartialEq)]
pub enum RespectObfuscatedSecrets {
    No,
    Yes,
}

/// Whether or not to obfuscate secrets during grid and tooltip rendering, respecting the Safe Mode setting.
#[derive(Clone, Copy, Debug, Default)]
pub enum ObfuscateSecrets {
    // Identify and visually obfuscate secrets
    Yes,
    /// Do not visually obfuscate secrets, but highlight them with a strikethrough
    Strikethrough,
    /// Show secrets with normal styling but still detect them for interaction (no visual treatment)
    AlwaysShow,
    #[default]
    No,
}

impl Not for ObfuscateSecrets {
    type Output = Self;

    fn not(self) -> Self::Output {
        match self {
            ObfuscateSecrets::Yes => ObfuscateSecrets::No,
            ObfuscateSecrets::No => ObfuscateSecrets::Yes,
            ObfuscateSecrets::Strikethrough => ObfuscateSecrets::Yes,
            ObfuscateSecrets::AlwaysShow => ObfuscateSecrets::Yes,
        }
    }
}

impl ObfuscateSecrets {
    /// Returns the "stronger" obfuscation mode. Priority: Yes > Strikethrough > AlwaysShow > No
    pub fn and(&self, other: &ObfuscateSecrets) -> ObfuscateSecrets {
        match (self, other) {
            (ObfuscateSecrets::Yes, _) | (_, ObfuscateSecrets::Yes) => ObfuscateSecrets::Yes,
            (ObfuscateSecrets::Strikethrough, _) | (_, ObfuscateSecrets::Strikethrough) => {
                ObfuscateSecrets::Strikethrough
            }
            (ObfuscateSecrets::AlwaysShow, _) | (_, ObfuscateSecrets::AlwaysShow) => {
                ObfuscateSecrets::AlwaysShow
            }
            (ObfuscateSecrets::No, ObfuscateSecrets::No) => ObfuscateSecrets::No,
        }
    }

    /// Returns whether the secret should be redacted given the current safe mode settings.
    /// This includes obfuscation, strikethrough, and always show (for interaction purposes).
    pub fn should_redact_secret(&self) -> bool {
        matches!(
            self,
            ObfuscateSecrets::Yes | ObfuscateSecrets::Strikethrough | ObfuscateSecrets::AlwaysShow
        )
    }

    /// Returns whether the current obfuscation mode is `ObfuscateSecrets::Yes`
    pub fn is_visually_obfuscated(&self) -> bool {
        matches!(self, ObfuscateSecrets::Yes)
    }
}

/// A secret (API key, password, etc) contained within the grid.
#[derive(Clone, Debug)]
pub struct Secret {
    /// Whether the secret is currently obfuscated.
    is_obfuscated: IsObfuscated,
    range: RangeInclusive<Point>,
    /// The level/source of this secret's redaction rule
    secret_level: SecretLevel,
}

impl RangeInModel for &Secret {
    fn range(&self) -> RangeInclusive<Point> {
        self.range.clone()
    }
}

impl RangeInModel for &mut Secret {
    fn range(&self) -> RangeInclusive<Point> {
        self.range.clone()
    }
}

pub type SecretAndHandle<'a> = (SecretHandle, &'a Secret);

impl Secret {
    pub(super) fn set_is_obfuscated(&mut self, is_obfuscated: IsObfuscated) {
        self.is_obfuscated = is_obfuscated
    }

    pub fn is_obfuscated(&self) -> bool {
        matches!(self.is_obfuscated, IsObfuscated::Yes)
    }

    pub fn new(
        is_obfuscated: IsObfuscated,
        range: RangeInclusive<Point>,
        secret_level: SecretLevel,
    ) -> Self {
        Self {
            is_obfuscated,
            range,
            secret_level,
        }
    }

    pub fn secret_level(&self) -> SecretLevel {
        self.secret_level
    }
}

/// Map that is responsible for storing secrets indexed by both [`SecretHandle`] and `Range`.
#[derive(Clone, Default, Debug)]
pub(in crate::terminal::model) struct SecretMap {
    /// Mapping of secrets stored within the grid, keyed on the secret's [`SecretHandle`].
    secrets: HashMap<SecretHandle, Secret>,
    /// Mapping of secrets keyed on the range of the secret.
    secret_ranges: RangeInclusiveMap<RangeMapPoint, SecretHandle>,
}

impl SecretMap {
    /// Insert a [`Secret`] identified by `handle` into the map.
    pub fn insert(&mut self, handle: SecretHandle, secret: Secret, num_columns: usize) {
        let secret_range = secret.range.clone();
        let range_point_range = RangeMapPoint::new(*secret_range.start(), num_columns)
            ..=RangeMapPoint::new(*secret_range.end(), num_columns);
        self.secret_ranges.insert(range_point_range, handle);
        self.secrets.insert(handle, secret);
    }

    /// Removes a [`Secret`] identified by `handle` from the map.
    pub fn remove(&mut self, handle: SecretHandle, num_columns: usize) {
        let removed = self.secrets.remove(&handle);
        if let Some(secret) = removed {
            let range = RangeMapPoint::new(*secret.range.start(), num_columns)
                ..=RangeMapPoint::new(*secret.range.end(), num_columns);
            self.secret_ranges.remove(range);
        }
    }

    /// Returns the [`Secret`] identified by [`SecretHandle`] or `None` if no such secret exists.
    pub fn get_by_handle(&self, handle: &SecretHandle) -> Option<&Secret> {
        self.secrets.get(handle)
    }

    /// Returns the [`Secret`] and its corresponding [`SecretHandle`] contained at the current
    /// [`Point`]. Returns `None` if there is no secret at the given point.
    pub fn get_by_point(
        &self,
        point: Point,
        grid: &GridHandler,
        respect_displayed_output: RespectDisplayedOutput,
    ) -> Option<SecretAndHandle<'_>> {
        let original_point = if grid.has_displayed_output()
            && matches!(respect_displayed_output, RespectDisplayedOutput::Yes)
        {
            grid.maybe_translate_point_from_displayed_to_original(point)
        } else {
            point
        };
        let point_with_metadata = RangeMapPoint::new(original_point, grid.columns());
        let handle = self.secret_ranges.get(&point_with_metadata).copied();

        handle.zip(handle.and_then(|h| self.get_by_handle(&h)))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&SecretHandle, &Secret)> {
        self.secrets.iter()
    }

    #[cfg(test)]
    pub fn ranges(&self) -> impl Iterator<Item = (RangeInclusive<Point>, &SecretHandle)> {
        self.secret_ranges
            .iter()
            .map(|(range, handle)| (range.start().as_point()..=range.end().as_point(), handle))
    }

    /// Clears all secrets within the map.
    pub fn clear(&mut self) {
        self.secrets.clear();
        self.secret_ranges.clear();
    }

    /// Marks the secret identified by `handle` as obfuscated. Returns an `Err` if no secret is
    /// identified by the `handle`.
    pub fn set_is_obfuscated(
        &mut self,
        handle: &SecretHandle,
        is_obfuscated: IsObfuscated,
    ) -> anyhow::Result<()> {
        let secret = self
            .secrets
            .get_mut(handle)
            .ok_or_else(|| anyhow!("No secret identified by provided SecretHandle"))?;
        secret.is_obfuscated = is_obfuscated;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.secrets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }

    /// Clears all of the secret ranges. Should be called after the resize of a grid since ranges
    /// are not stable across resizes.
    pub fn clear_ranges_after_resize(&mut self) {
        self.secret_ranges.clear();
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

/// A wrapper around a [`Point`] that implements [`StepLite`], allowing us to store it in a
/// `RangeMap`. Used for secret redaction so we efficiently map from a given range to an underlying
/// secret stored at that range.
#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
struct RangeMapPoint {
    point: Point,
    num_cols: usize,
}

impl RangeMapPoint {
    fn new(point: Point, num_cols: usize) -> Self {
        Self { point, num_cols }
    }

    fn as_point(&self) -> Point {
        self.point
    }
}

impl StepLite for RangeMapPoint {
    fn add_one(&self) -> Self {
        let mut new_point = self.point;
        new_point.col += 1;
        if new_point.col >= self.num_cols {
            new_point.col = 0;
            new_point.row += 1;
        }

        RangeMapPoint {
            point: new_point,
            num_cols: self.num_cols,
        }
    }

    fn sub_one(&self) -> Self {
        let mut new_point = self.point;
        if new_point.col == 0 {
            if new_point.row == 0 {
                return *self;
            }
            new_point.row -= 1;
            new_point.col = self.num_cols - 1;
        } else {
            new_point.col -= 1;
        }

        RangeMapPoint {
            point: new_point,
            num_cols: self.num_cols,
        }
    }
}

pub mod regexes {
    use crate::settings::RegexDisplayInfo;

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
#[path = "secrets_tests.rs"]
mod tests;
