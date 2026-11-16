//! Forward and reverse lazy DFAs for regex searches that scan a haystack byte by byte, in
//! either direction.

use std::borrow::Cow;

use regex::escape;
use regex_automata::hybrid::BuildError;
use regex_automata::hybrid::dfa::{Cache, DFA};
use regex_automata::nfa::thompson;
use regex_automata::util::pool::{Pool, PoolGuard};
use regex_automata::util::syntax::Config;

/// Describes the state of the find bar configuration options
///
/// Used to configure DFA in the correct way
pub struct FindConfig {
    pub is_regex_enabled: bool,
    pub is_case_sensitive: bool,
}

impl Default for FindConfig {
    fn default() -> Self {
        Self {
            is_regex_enabled: true,
            is_case_sensitive: false,
        }
    }
}

/// The type of the closure we use to create new caches.
pub type CachePoolFn = Box<dyn Fn() -> Cache + Send + Sync>;

/// Struct that provides forward and reverse DFAs to search for a regular expression pattern from
/// either direction.
#[derive(Debug)]
pub struct RegexDFAs {
    /// DFA used to search from left to right.
    forward_dfa: DFA,
    /// DFA used to search from right to left.
    reverse_dfa: DFA,
    /// Thread safe pool cache for the forward-DFA. Since we use "lazy" DFAs (which are built
    /// incrementally during search) we need to cache the DFA's transitional table. This is
    /// continuously updated when moving through states within the DFA.
    forward_pool: Pool<Cache, CachePoolFn>,
    /// Thread safe pool cache for the reverse-DFA. Since we use "lazy" DFAs (which are built
    /// incrementally during search) we need to cache the DFA's transitional table. This is
    /// continuously updated when moving through states within the DFA.
    reverse_pool: Pool<Cache, CachePoolFn>,
}

impl RegexDFAs {
    // Create case-insensitive Regex DFAs for all find directions.
    pub fn new(find: &str) -> Result<RegexDFAs, Box<BuildError>> {
        Self::new_with_config(find, FindConfig::default())
    }

    /// Constructs a [`RegexDFAs`] that matches any of the patterns provided.
    pub fn new_many(
        patterns: &[&str],
        enable_unicode_word_boundary: bool,
        case_sensitive: bool,
    ) -> Result<RegexDFAs, Box<BuildError>> {
        let patterns = patterns
            .iter()
            .map(|pattern| {
                if enable_unicode_word_boundary {
                    Cow::Borrowed(*pattern)
                } else {
                    Cow::Owned(replace_unicode_word_boundaries(pattern))
                }
            })
            .collect::<Vec<_>>();
        let pattern_refs = patterns
            .iter()
            .map(|pattern| pattern.as_ref())
            .collect::<Vec<_>>();

        let mut builder = DFA::builder();
        builder.configure(
            DFA::config()
                .unicode_word_boundary(enable_unicode_word_boundary)
                // Increase the default maximum cache capacity by 4x. The default is
                // 2MB, which isn't quite enough to efficiently handle large regexes.
                .cache_capacity(DFA::config().get_cache_capacity() << 2)
                // Just in case our increased cache capacity is somehow too small to
                // run the regex at all, we tell the builder to increase the cache
                // capacity even further if required to meet the minimum.
                .skip_cache_capacity_check(true),
        );
        if !case_sensitive {
            builder.syntax(Config::new().case_insensitive(true));
        }
        Self::new_internal(&pattern_refs, builder)
    }

    // Based on FindConfig, create DFAs for all directions
    pub fn new_with_config(
        find: &str,
        find_config: FindConfig,
    ) -> Result<RegexDFAs, Box<BuildError>> {
        let mut builder = DFA::builder();
        if !find_config.is_case_sensitive {
            builder.syntax(Config::new().case_insensitive(true));
        }
        if find_config.is_regex_enabled {
            let patched_find = replace_unicode_word_boundaries(find);
            Self::new_internal(&[&patched_find], builder)
        } else {
            Self::new_internal(&[&escape(find)], builder)
        }
    }

    fn new_internal(
        patterns: &[&str],
        mut builder: regex_automata::hybrid::dfa::Builder,
    ) -> Result<RegexDFAs, Box<BuildError>> {
        // Build a forward and reverse DFA to allow us to find matches either left-to-right or
        // right-to-left.
        // We don't use the hybrid Regex (https://docs.rs/regex-automata/latest/regex_automata/hybrid/regex/struct.Regex.html)
        // struct directly since it would require us to create two different instances of a `Regex`,
        // which internally would create 4 different DFAs when we really only need 2 to support the
        // functionality of searching through a grid from either direction.
        let forward_dfa = builder.clone().build_many(patterns)?;
        let reverse_dfa = builder
            .thompson(thompson::Config::new().reverse(true))
            .build_many(patterns)?;

        let forward_cache = forward_dfa.create_cache();
        let reverse_cache = reverse_dfa.create_cache();

        let forward_pool = {
            let create: CachePoolFn = Box::new(move || forward_cache.clone());
            Pool::new(create)
        };

        let reverse_pool = {
            let create: CachePoolFn = Box::new(move || reverse_cache.clone());
            Pool::new(create)
        };

        Ok(Self {
            forward_dfa,
            reverse_dfa,
            forward_pool,
            reverse_pool,
        })
    }

    /// Returns the DFA used to search from left to right, along with a cache for it.
    pub fn get_forward(&self) -> (&DFA, PoolGuard<'_, Cache, CachePoolFn>) {
        (&self.forward_dfa, self.forward_pool.get())
    }

    /// Returns the DFA used to search from right to left, along with a cache for it.
    pub fn get_reverse(&self) -> (&DFA, PoolGuard<'_, Cache, CachePoolFn>) {
        (&self.reverse_dfa, self.reverse_pool.get())
    }
}

/// By default, \b doesn't work in `regex-automata`. See this section in their docs:
/// https://docs.rs/regex/latest/regex/index.html#unicode-can-impact-memory-usage-and-search-speed
///
/// "This crate has first class support for Unicode and it is enabled by default... However, some
/// of the faster internal regex engines cannot handle a Unicode aware word boundary assertion. So
/// if you don’t need Unicode-aware word boundary assertions, you might consider using (?-u:\b)
/// instead of \b, where the former uses an ASCII-only definition of a word character."
///
/// Including a \b in a regex causes compilation of the regex to fail with a haystack containing
/// unicode. Therefore, we replace it with the ASCII-only version as the docs suggest.
///
/// This rewrite is intentionally syntax-aware. Custom secret regexes can contain literal escaped
/// backslashes (for example `\\b`) or byte escapes inside character classes (`[\b]`), neither of
/// which should be treated as word-boundary assertions. We only rewrite unescaped `\b` and `\B`
/// outside character classes so those patterns keep their original meaning while still avoiding
/// Unicode word-boundary DFA failures.
///
/// Note: One alternative could be use enable this option:
/// https://docs.rs/regex-automata/0.4.6/regex_automata/hybrid/dfa/struct.Config.html#method.unicode_word_boundary
/// However, "this only works when the search input is ASCII only." This assumption is
/// often false in the terminal context, which often contains emojis, box-drawing chars,
/// international text, etc.
fn replace_unicode_word_boundaries(pattern: &str) -> String {
    let mut result = String::with_capacity(pattern.len());
    let mut chars = pattern.char_indices().peekable();
    let mut in_character_class = false;

    while let Some((index, c)) = chars.next() {
        let is_escaped = count_preceding_backslashes(pattern, index) % 2 == 1;
        if c == '[' && !is_escaped {
            in_character_class = true;
        } else if c == ']' && !is_escaped {
            in_character_class = false;
        }

        if c == '\\' && !in_character_class && !is_escaped {
            match chars.peek().map(|(_, next)| *next) {
                Some('b') => {
                    result.push_str("(?-u:\\b)");
                    chars.next();
                }
                Some('B') => {
                    result.push_str("(?-u:\\B)");
                    chars.next();
                }
                _ => result.push(c),
            }
        } else {
            result.push(c);
        }
    }

    result
}

fn count_preceding_backslashes(pattern: &str, index: usize) -> usize {
    pattern[..index]
        .chars()
        .rev()
        .take_while(|c| *c == '\\')
        .count()
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
