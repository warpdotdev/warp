//! Regex-based search through a [`Buffer`].
//!
//! As in other cases, this uses the `regex_automata` [`DFA`] API directly, so that it does not
//! need to copy the buffer into a string (the regex APIs all require an `&[u8]` or equivalent).
//! This has a few downsides, however:
//! * Lazy DFAs do not handle Unicode word boundaries well (see
//!   [`regex_automata::hybrid::dfa::Config::unicode_word_boundary`])
//! * We miss out on optimizations that the [`regex_automata::meta::Regex`] matcher makes by
//!   choosing between different engines (like avoiding a regex engine entirely for simple
//!   literals). In practice, this is likely not a major loss because the literal search
//!   optimizations rely on fast substring searches (like highly-optimized platform-specific
//!   `memchr` implementations) that we can't use with a non-contiguous buffer.

use std::{borrow::Cow, future::Future};

use anyhow::{Context, bail};
use rangemap::RangeSet;
use regex_automata::{
    Anchored, Input, MatchError, MatchKind,
    hybrid::{
        BuildError, LazyStateID,
        dfa::{Cache, DFA},
    },
    nfa::thompson,
    util::syntax::Config,
};
use sum_tree::SumTree;

use string_offset::CharOffset;

use crate::search::RestorableSearchResults;

use super::{
    buffer::Buffer,
    cursor::BufferCursor,
    text::{BufferSummary, BufferText},
};

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;

/// A match for a text search.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Match {
    /// The starting offset of the match (inclusive).
    pub start: CharOffset,
    /// The ending offset of the match (exclusive).
    pub end: CharOffset,
}

/// A compiled, reusable search query.
#[derive(Debug)]
pub struct Query {
    /// Box the inner [`Engine`] because it may be large and [`Query`] is moved frequently.
    engine: Box<Engine>,
}

/// Results of a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResults {
    pub matches: Vec<Match>,
}

impl RestorableSearchResults for &SearchResults {
    fn valid_matches(&self) -> impl Iterator<Item = (usize, CharOffset)> {
        self.matches
            .iter()
            .enumerate()
            .map(|(index, m)| (index, m.start))
    }
}

/// Configuration for a search query.
#[derive(Debug, Clone, Copy)]
pub struct SearchConfig<'a> {
    query: &'a str,
    case_sensitive: bool,
    regex: bool,
    skip_hidden: bool,
    hidden_ranges: Option<&'a RangeSet<CharOffset>>,
}

impl<'a> SearchConfig<'a> {
    /// Build configuration to search for the literal `query`. By default, the search is
    /// case-sensitive and includes hidden content.
    pub fn new(query: &'a str) -> Self {
        Self {
            query,
            case_sensitive: true,
            regex: false,
            skip_hidden: false,
            hidden_ranges: None,
        }
    }

    /// Build configuration to search for `pattern` as a regular expression. By default, the search
    /// is case-sensitive and includes hidden content.
    pub fn regex(pattern: &'a str) -> Self {
        Self {
            query: pattern,
            case_sensitive: true,
            regex: true,
            skip_hidden: false,
            hidden_ranges: None,
        }
    }

    /// Set whether or not this search is case-sensitive.
    pub fn with_case_sensitive(mut self, case_sensitive: bool) -> Self {
        self.case_sensitive = case_sensitive;
        self
    }

    /// Set whether or not the search query is interpreted as a regular expression.
    pub fn with_regex(mut self, regex: bool) -> Self {
        self.regex = regex;
        self
    }

    /// Set whether or not to skip hidden content during search. When enabled, text within
    /// hidden regions (marked by `BufferTextStyle::Hidden`) will not be searched and will
    /// act as match boundaries.
    pub fn with_skip_hidden(mut self, skip_hidden: bool) -> Self {
        self.skip_hidden = skip_hidden;
        self
    }

    pub fn with_hidden_ranges(mut self, hidden_lines: &'a RangeSet<CharOffset>) -> Self {
        self.hidden_ranges = Some(hidden_lines);
        self
    }
}

#[derive(Debug)]
struct Engine {
    // For now, this uses the same DFA approach as the block list. We should
    // consider using the meta::Regex implementation instead for better Unicode support (and
    // possibly better performance for literal searches).
    // Because all matches are constructed up front, we only need forward searching.
    forward_dfa: DFA,
    forward_cache: Cache,
    reverse_dfa: DFA,
    reverse_cache: Cache,
    skip_hidden: bool,
    hidden_ranges: Option<RangeSet<CharOffset>>,
}

impl Engine {
    fn new(config: &SearchConfig) -> Result<Self, Box<BuildError>> {
        log::trace!("Compiling {config:?}");
        let mut builder = DFA::builder();
        builder
            .syntax(
                Config::new()
                    .case_insensitive(!config.case_sensitive)
                    // Enable multi-line mode by default - because the buffer always starts with a
                    // block marker, `^` anchors are otherwise useless.
                    .multi_line(true),
            )
            .configure(DFA::config().unicode_word_boundary(true));

        let query = if config.regex {
            Cow::Borrowed(config.query)
        } else {
            Cow::Owned(regex_syntax::escape(config.query))
        };

        let forward_dfa = builder.clone().build(query.as_ref())?;
        // See https://github.com/rust-lang/regex/blob/837fd85e79fac2a4ea64030411b9a4a7b17dfa42/regex-automata/src/hybrid/regex.rs#L793-L802
        // and https://github.com/rust-lang/regex/blob/837fd85e79fac2a4ea64030411b9a4a7b17dfa42/regex-automata/src/hybrid/regex.rs#L87-L94
        //
        // This configuration ensures we find the right match.
        let reverse_dfa = builder
            .configure(
                DFA::config()
                    .specialize_start_states(false)
                    .match_kind(MatchKind::All),
            )
            .thompson(thompson::Config::new().reverse(true))
            .build(query.as_ref())?;

        let forward_cache = forward_dfa.create_cache();
        let reverse_cache = reverse_dfa.create_cache();
        Ok(Self {
            forward_dfa,
            forward_cache,
            reverse_dfa,
            reverse_cache,
            skip_hidden: config.skip_hidden,
            hidden_ranges: config.hidden_ranges.cloned(),
        })
    }

    /// Run a search, blocking until it finishes completely. See [`Engine::find`].
    #[cfg(test)]
    fn find_blocking(
        &mut self,
        buffer: &SumTree<BufferText>,
        buffer_offset: CharOffset,
    ) -> anyhow::Result<Vec<Match>> {
        warpui::r#async::block_on(self.find(buffer, buffer_offset))
    }

    /// Find all matches for this pattern in the given slice of content.
    ///
    /// All returned offsets are shifted by `buffer_offset` - if the `SumTree` doesn't
    /// start at the beginning of the buffer, use this to adjust accordingly (for example, to
    /// search in a range of text).
    ///
    /// The search will yield to the scheduler periodically so that it may be cancelled.
    async fn find(
        &mut self,
        buffer: &SumTree<BufferText>,
        buffer_offset: CharOffset,
    ) -> anyhow::Result<Vec<Match>> {
        let mut results = vec![];
        let mut cursor = buffer.cursor::<CharOffset, BufferSummary>();
        cursor.descend_to_first_item(buffer, |_| true);
        let mut buffer_cursor = BufferCursor::new(cursor);

        while let Some(match_end) = self.next_match(&mut buffer_cursor, SearchDirection::Forward)? {
            log::trace!("Found match ending at {match_end}");
            // The forward DFA reports the _end_ of the match, so we then use the reverse DFA to
            // find its start.

            let cursor = buffer.cursor::<CharOffset, BufferSummary>();
            buffer_cursor = BufferCursor::new(cursor);
            // Seek to the match end location - this might not be the current cursor position,
            // because next_match will keep advancing to look for longer matches if possible.
            buffer_cursor.seek_to_offset_before_markers(match_end);
            // Since match_end is exclusive, move to the prev char position to make sure we are reverse
            // iterating from the correct location.
            buffer_cursor.prev_char_position();

            // Clone the cursor to not lose the search position - the next iteration will resume
            // searching after this match.
            let mut reverse_cursor = buffer_cursor.clone();
            if let Some(match_start) =
                self.next_match(&mut reverse_cursor, SearchDirection::Reverse)?
            {
                // The DFA-reported match end will be the first character _after_ the match - the
                // DFA state is always delayed by 1 byte to support look-around operators. For the
                // match end, this is what we want, since it's an exclusive end.
                // For the start, we add 1 to get the inclusive start offset (adding because the
                // start is found with a DFA moving backwards).
                results.push(Match {
                    start: match_start + buffer_offset + 1,
                    end: match_end + buffer_offset,
                });
                log::trace!("Match started at {}", match_start + 1);
            } else {
                log::warn!("Forward DFA found a match end, but reverse DFA did not find a start");
            }

            // Because we sought to the left of the match end, move to the next item after the
            // match to prevent an infinite loop.
            buffer_cursor.next_char_position();

            // Individual calls to `next_match` should be fairly fast (barring a pathologically
            // slow regular expression), but searching a buffer with many matches could still be
            // slow. As a rough heuristic, we yield every 1000 matches so that the future can be
            // meaningfully cancelled. If this is insufficient, we could also yield every X
            // characters.
            if results.len() % 1000 == 1 {
                futures_lite::future::yield_now().await;
            }
        }

        Ok(results)
    }

    /// Find the next pattern match in the given direction, starting at the current `cursor` location.
    /// Where variable-length matches are allowed, this will find the longest one (for example, if
    /// searching for `a+` in `aaab`, this will return the offset up to `aaa`, even though `a` by
    /// itself is also a match).
    fn next_match(
        &mut self,
        cursor: &mut BufferCursor<BufferSummary>,
        direction: SearchDirection,
    ) -> anyhow::Result<Option<CharOffset>> {
        let skip_hidden = self.skip_hidden;
        let (dfa, cache) = direction.dfa_and_cache(
            &self.forward_dfa,
            &mut self.forward_cache,
            &self.reverse_dfa,
            &mut self.reverse_cache,
        );
        let mut state = direction.start_state(dfa, cache)?;

        // We want to find the _longest_ match in a given direction. This means we have to keep
        // searching past a match state, until we reach a dead state or end of input.
        let mut match_location = None;

        'items: while let Some(item) = cursor.item() {
            let start_char = cursor.start().text.chars;
            log::trace!("Advancing state machine by {item:?} @ {start_char}");
            match item {
                BufferText::Text { .. } => {
                    if skip_hidden
                        && self
                            .hidden_ranges
                            .as_ref()
                            .map(|hl| hl.contains(&start_char))
                            .unwrap_or(false)
                    {
                        // Hidden text interrupts search. If we've already found a match, return it.
                        // Otherwise, see if advancing to the EOI state triggers a match (e.g. if the
                        // last character before the hidden text was a match).
                        if match_location.is_some() {
                            break 'items;
                        } else {
                            state = dfa.next_eoi_state(cache, state)?;
                            if state.is_match() {
                                match_location = Some(cursor.start().text.chars);
                                break 'items;
                            }
                            state = direction.start_state(dfa, cache)?;
                        }
                    } else if let Some(character) = cursor.char() {
                        let mut bytes = [0u8; 4];
                        for byte in character.encode_utf8(&mut bytes).bytes() {
                            state = dfa
                                .next_state(cache, state, byte)
                                .context("Couldn't advance to next state")?;
                            if state.is_quit() {
                                bail!("DFA entered quit state");
                            } else if state.is_dead() {
                                break 'items;
                            } else if state.is_match() {
                                match_location = Some(cursor.offset());
                            }
                        }
                    };
                }
                BufferText::Newline | BufferText::BlockMarker { .. } => {
                    state = dfa.next_state(cache, state, b'\n')?;
                    if state.is_quit() {
                        bail!("DFA entered quit state");
                    } else if state.is_dead() {
                        break 'items;
                    } else if state.is_match() {
                        match_location = Some(cursor.start().text.chars);
                    }
                }
                BufferText::Marker { .. } | BufferText::Link(_) | BufferText::Color(_) => {
                    // Inline styling is ignored by search.
                }
                BufferText::Placeholder { .. } | BufferText::BlockItem { .. } => {
                    // Non-text / non-interactive items interrupt search. If we've already found a
                    // match, return it. Otherwise, see if advancing to the EOI state triggers a
                    // match (e.g. if the last character before the item was a match).
                    if match_location.is_some() {
                        break 'items;
                    } else {
                        state = dfa.next_eoi_state(cache, state)?;
                        if state.is_match() {
                            match_location = Some(cursor.start().text.chars);
                            // We don't need to keep searching in this case, because that would
                            // allow matching across the boundary.
                            break 'items;
                        }
                        state = direction.start_state(dfa, cache)?;
                    }
                }
            }

            direction.advance(cursor);
        }

        state = dfa.next_eoi_state(cache, state)?;
        if state.is_match() {
            match_location = Some(cursor.offset())
        }

        Ok(match_location)
    }
}

#[derive(Debug, Clone, Copy)]
enum SearchDirection {
    Forward,
    Reverse,
}

impl SearchDirection {
    /// The starting state for searches in this direction.
    fn start_state(self, dfa: &DFA, cache: &mut Cache) -> Result<LazyStateID, MatchError> {
        match self {
            Self::Forward => dfa.start_state_forward(cache, &Input::new("").anchored(Anchored::No)),
            // See https://github.com/rust-lang/regex/blob/837fd85e79fac2a4ea64030411b9a4a7b17dfa42/regex-automata/src/hybrid/regex.rs#L483-L488
            // For a reverse search, we need to anchor and disable 'earliest'. This makes sure we
            // match as much as possible (find the leftmost match) and don't find any matches
            // besides the result of the forward search.
            Self::Reverse => dfa.start_state_reverse(
                cache,
                &Input::new("").anchored(Anchored::Yes).earliest(false),
            ),
        }
    }

    /// Advance the cursor in this direction.
    fn advance(self, cursor: &mut BufferCursor<BufferSummary>) {
        match self {
            Self::Forward => cursor.next_char_position(),
            Self::Reverse => {
                cursor.prev_char_position();
            }
        }
    }

    fn dfa_and_cache<'a>(
        self,
        forward_dfa: &'a DFA,
        forward_cache: &'a mut Cache,
        reverse_dfa: &'a DFA,
        reverse_cache: &'a mut Cache,
    ) -> (&'a DFA, &'a mut Cache) {
        match self {
            Self::Forward => (forward_dfa, forward_cache),
            Self::Reverse => (reverse_dfa, reverse_cache),
        }
    }
}

impl Buffer {
    /// Compile a search into a reusable [`Query`].
    pub fn prepare_search(&self, config: &SearchConfig) -> anyhow::Result<Query> {
        let engine = Box::new(Engine::new(config)?);
        Ok(Query { engine })
    }

    /// Asynchronously run a search from a precompiled query. The search will yield periodically so
    /// that it may be cancelled.
    ///
    /// See [`Buffer::prepare_search`].
    pub fn search(
        &self,
        mut query: Query,
    ) -> impl Future<Output = (Query, anyhow::Result<SearchResults>)> + use<> {
        // Cloning the SumTree is cheap, as it wraps an `Arc` of the root node.
        let content = self.content.clone();
        async move {
            let results = query
                .engine
                .find(&content, CharOffset::zero())
                .await
                .map(|matches| SearchResults { matches });
            (query, results)
        }
    }
}
