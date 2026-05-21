use std::{collections::HashSet, ops::Range};

use lazy_static::lazy_static;
use regex::Regex;
use settings::{macros::define_settings_group, Setting, SupportedPlatforms, SyncToCloud};
use string_offset::ByteOffset;
use warpui::elements::SmartSelectFn;

use warpui::text::{
    word_boundaries::WordBoundariesPolicy,
    words::{is_default_word_boundary, DEFAULT_WORD_BOUNDARY_CHARS},
};

/// Upper limit for how many characters in either direction we'll search for patterns. Need to
/// limit this to avoid running regex on absurdly long words
pub const SMART_SELECT_MATCH_WINDOW_LIMIT: u32 = 1000;

pub const DEFAULT_WORD_CHAR_ALLOWLIST: &str = "-.~/\\";

lazy_static! {
    static ref DEFAULT_WORD_BOUNDARY_CHAR_SET: HashSet<char> = HashSet::from(DEFAULT_WORD_BOUNDARY_CHARS);

    /// These regexes are the specifications for all recognized smart-select objects, sorted in
    /// precedence-order (highest precedence first). The precedence is determined by specificity.
    /// It is possible for multiple patterns to apply to the same double-click. In that case, we
    /// want the more "specific" pattern to take precedence. For example, double-clicking on the
    /// path portion of a URL will match the URL regex, filepath regex, and identifier regex. In
    /// order to make sure the whole URL actually gets selected, we make the URL highest
    /// precedence.
    static ref REGEXES: [Regex; 5] = [
        // URL pattern, any scheme
        // https://en.wikipedia.org/wiki/Uniform_Resource_Identifier#Syntax
        // Note: This regex is not 100% rigorously correct for all valid URLs. For example, the
        // "web+" scheme is not recognized here. Punctuation characters in the username/password
        // component (though use of those in URLs in cleartext is discouraged). The "//" is
        // actually optional. Overall we don't need perfect recall for recognizing all URLs.
        // These compromises were made for precision.
        //   [a-z][a-z\d.-]*  - scheme e.g. https, ssh
        //   ://  - literal
        //   ([\w.-]+(:[\w.-]+)?@)?  - maybe a username/password e.g. andy:secret@
        //   ([\w-]+((\.[\w-]+)+)|(\[[:\da-f]+\]))  - host, either a domain name (google.com), or
        //     an IPv4 or IPv6 address
        //   (:\d{1,5})?  - maybe a port number
        //   ([\w.,@?^=%&:/~+#-]*[\w@?^=%&/~+#-])?  - maybe query string and/or fragment. this
        //     syntax isn't actuall well-defined in general for URLs
        Regex::new(
            r"(?i)[a-z][a-z\d.-]*://(([\w.-]+(:[\w.-]+)?@)?([\w-]+((\.[\w-]+)+)|(\[[:\da-f]+\]))(:\d{1,5})?)?([\w.,@?^=%&:/~+#-]*[\w@?^=%&/~+#-])?"
        )
        .expect("URL regex malformed"),
        // Email pattern
        // https://en.wikipedia.org/wiki/Email_address#Syntax
        // Note: This regex is not 100% rigorously correct either. The "local-part" of the address
        // cannot contain consecutive dots ".." but this regex does not enforce that. It also does
        // not check the UTF-8 byte range for the characters.
        //   [\w\d!#$%&'*+-/=?^`{|}~.]+  - "local-part" of the email address
        //   @  - literal "at sign"
        //   [a-z\d-]+\.[a-z\d.-]+[a-z\d-]  - domain with arbitrary subdomains (foo@a.b.c.com)
        Regex::new(
            r"(?i)[\w\d!#$%&'*+-/=?^`{|}~.]+@[a-z\d-]+\.[a-z\d.-]+[a-z\d-]"
        )
        .expect("email regex malformed"),
        // Float in scientific notation pattern, e.g. 6.02e+23
        //   -?  - may have negative sign
        //   \d  - mantissa, integer portion
        //   (\.\d+)?  - mantissa, may have non-integer portion
        //   e   - literal "e"
        //   [+-]?  - may have sign
        //   \d+  - exponent part
        Regex::new(
            r"(?i)-?\d(\.\d+)?(e[+-]?\d+)"
        )
        .expect("scientific notation regex malformed"),
        // Filepath pattern
        // Note: Filepaths may contain ANY punctuation characters or whitespace, but we aren't
        // attempting to match that all.
        //   (~|\b[a-z]:|[\w.*-]+)?  - On *nix, may start with tilde. On windows, may start with a
        //     drive letter. Or the prefix might be totally ordinary.
        //   [/\\]  - A slash of some kind is required.
        //   [/\\\w.*-]*  - All stuff after the slash, any number of word-chars, (back)slashes,
        //     dots, asterisks, and dashes
        Regex::new(
            r"(?i)(~|\b[a-z]:|[\w.*-]+)?[/\\][/\\\w.*-]*"
        )
        .expect("filepath regex malformed"),
        // Identifier pattern
        // This is the least rigorously-defined pattern, so it's last. It is a common set of
        // characters people use in names for things. Underscores are already considered part of
        // words universally, but hyphens and dots are also commonly-used separators in names.
        // Other than filepaths, hyphenated names were the most commonly-requested entities to be
        // double-clicked. This pattern also happens to recognize floats and IP addresses
        //   \w+  - Ordinary word-chars
        //   ([.-]\w+)*  - Any dot or dash separators must be followed by more word characters.
        //     Cannot have multiple consecutive separators.
        Regex::new(
            r"(?i)\w+([.-]\w+)*"
        )
        .expect("identifier regex"),
    ];
}

define_settings_group!(SemanticSelection, settings: [
    smart_select_enabled: SmartSelectEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        storage_key: "SmartSelect",
        toml_path: "terminal.smart_select.enabled",
        description: "Whether double-click smart selection is enabled for URLs, emails, file paths, and identifiers.",
    },
    word_char_allowlist: WordCharAllowlist {
        type: String,
        default: DEFAULT_WORD_CHAR_ALLOWLIST.to_owned(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        storage_key: "WordCharAllowlist",
        toml_path: "terminal.smart_select.word_char_allowlist",
        description: "Characters that are considered part of a word for double-click selection when smart select is disabled.",
    },
]);

impl SemanticSelection {
    #[cfg(any(test, feature = "test-util"))]
    pub fn mock(smart_select_enabled: bool, word_char_allowlist: impl Into<String>) -> Self {
        Self {
            smart_select_enabled: SmartSelectEnabled::new(Some(smart_select_enabled)),
            word_char_allowlist: WordCharAllowlist::new(Some(word_char_allowlist.into())),
        }
    }

    pub fn smart_select_enabled(&self) -> bool {
        *self.smart_select_enabled.value()
    }

    pub fn word_char_allowlist_changed_from_default(&self) -> bool {
        *self.word_char_allowlist.value() != DEFAULT_WORD_CHAR_ALLOWLIST
    }

    pub fn word_char_allowlist_string(&self) -> String {
        self.word_char_allowlist.value().clone()
    }

    fn to_char_set(char_list: &str) -> HashSet<char> {
        HashSet::from_iter(char_list.chars().filter(|c| !c.is_whitespace()))
    }

    fn word_char_allowlist_set(&self) -> HashSet<char> {
        Self::to_char_set(self.word_char_allowlist.value())
    }

    /// The core logic for smart-select. This takes the extracted string window and runs the known
    /// regexes on it in precedence-order. It needs the byte offset of the cursor in order to
    /// guarantee that the range of the matching pattern actually contains the cursor, otherwise we
    /// might match something in the window which doesn't overlap with the cursor.
    pub fn smart_search(
        &self,
        content: &str,
        click_offset: ByteOffset,
    ) -> Option<Range<ByteOffset>> {
        if !self.smart_select_enabled() {
            return None;
        }

        Self::smart_select(content, click_offset)
    }

    pub fn smart_select_fn(&self) -> Option<SmartSelectFn> {
        if !self.smart_select_enabled() {
            return None;
        }

        Some(Self::smart_select)
    }

    fn smart_select(content: &str, click_offset: ByteOffset) -> Option<Range<ByteOffset>> {
        for regex in REGEXES.iter() {
            for hit in regex.find_iter(content) {
                if hit.range().contains(&click_offset.as_usize()) {
                    return Some(
                        ByteOffset::from(hit.range().start)..ByteOffset::from(hit.range().end),
                    );
                }
            }
        }

        None
    }

    /// This function determines if a particular character is word-breaking depending on the
    /// semantic selection settings. Used specifically by the GridHandler.
    pub fn is_word_boundary_char(&self, c: char) -> bool {
        if !self.smart_select_enabled() && self.word_char_allowlist_set().contains(&c) {
            return false;
        }
        is_default_word_boundary(c)
    }

    /// This function fulfills the same purpose as Self::is_word_boundary_char, but in the way
    /// specific to the Editor Buffer. That data structure has a custom iterator which is
    /// configurable via a WordBoundariesPolicy enum. Ideally, WordBoundariesPolicy would hold a
    /// reference to this model so it could call Self::is_word_boundary_char and share the exact
    /// same logic with the GridHandler. However, in order to do that, the reference to this
    /// model would need to live for 'static which is not feasible.
    pub fn word_boundary_policy(&self) -> WordBoundariesPolicy {
        if self.smart_select_enabled() {
            WordBoundariesPolicy::Default
        } else {
            WordBoundariesPolicy::Custom(
                DEFAULT_WORD_BOUNDARY_CHAR_SET
                    .difference(&self.word_char_allowlist_set())
                    .copied()
                    .collect(),
            )
        }
    }
}

#[cfg(test)]
#[cfg(feature = "test-util")]
#[path = "mod_tests.rs"]
mod tests;
