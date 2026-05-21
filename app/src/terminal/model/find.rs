use std::ops::RangeInclusive;

use regex::escape;
use regex_automata::hybrid::dfa::{Cache, DFA};
use regex_automata::hybrid::BuildError;
use regex_automata::nfa::thompson;
use regex_automata::util::pool::Pool;
use regex_automata::util::syntax::Config;
use regex_automata::{Anchored, Input};
use warp_terminal::model::grid::CellType;

use super::grid::grapheme_cursor;
use super::grid::grid_handler::GridHandler;
use crate::terminal::model::index::{Direction, Point};

pub type Match = RangeInclusive<Point>;

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
type CachePoolFn = Box<dyn Fn() -> Cache + Send + Sync>;

/// Struct that provides APIs to search through a [`Grid`] using a regular expression pattern.
#[derive(Debug)]
pub struct RegexDFAs {
    /// DFA used to search the grid from left to right.
    forward_dfa: DFA,
    /// DFA used to search the grid from right to left.
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
        Self::new_internal(patterns, builder)
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

    /// Find the next regex match to the right of the origin point by beginning at the `left` Point
    /// and searching until the `right` Point is reached, inclusive of both points.
    ///
    /// The origin is always included in the regex.
    pub fn regex_search_rightwards(
        &self,
        left: Point,
        right: Point,
        grid: &GridHandler,
    ) -> Option<Match> {
        // Scan from the left -> right to find the end (rightmost) point of the match.
        let match_right_point = self.search(left, right, Direction::Right, grid, Anchored::No)?;

        // Scan leftwards from the match end to the left most point to find the beginning (leftmost) point of the match.
        let match_left_point = self.search(
            match_right_point,
            left,
            Direction::Left,
            grid,
            Anchored::Yes,
        )?;

        Some(match_left_point..=match_right_point)
    }

    /// Find the next regex match to the left of the `right` Point by searching leftwards from
    /// `right` until the `left` Point is reached.
    ///
    /// The origin is always included in the regex.
    pub fn regex_search_leftwards(
        &self,
        right: Point,
        left: Point,
        grid: &GridHandler,
    ) -> Option<Match> {
        // Scan leftwards to find the starting (leftmost) point of the match.
        let match_left_point = self.search(right, left, Direction::Left, grid, Anchored::No)?;
        // Scan rightwards from the match start to the rightmost point to find the end (rightmost) point of the match.
        let match_right_point = self.search(
            match_left_point,
            right,
            Direction::Right,
            grid,
            Anchored::Yes,
        )?;

        Some(match_left_point..=match_right_point)
    }

    /// Find the next regex match, given a direction.
    ///
    /// This will always return the side of the first match which is farthest from the start point.
    fn search(
        &self,
        start: Point,
        end: Point,
        direction: Direction,
        grid: &GridHandler,
        anchored: Anchored,
    ) -> Option<Point> {
        let (dfa, mut cache) = match direction {
            Direction::Left => (&self.reverse_dfa, self.reverse_pool.get()),
            Direction::Right => (&self.forward_dfa, self.forward_pool.get()),
        };

        let mut cursor = grid.grapheme_cursor_from(start, grapheme_cursor::Wrap::All);

        // Initialize the match state. DFAs can have multiple start states, but only when there are
        // look-around assertions. When there aren't any look-around assertions, as in this case,
        // we can ask for a start state without providing any of the haystack. See
        // https://blog.burntsushi.net/regex-internals.
        let mut state = dfa
            .start_state_forward(&mut cache, &Input::new("").anchored(anchored))
            .ok()?;

        let mut regex_match = None;

        // The state of a DFA is always delayed by one byte in order to support look-around
        // operators. Store the _previous_ point as we iterate through the grid to ensure that we
        // don't eagerly report the current point if the additional byte from the current point
        // triggers a match for the last point.
        let mut last_point = None;

        'outer: loop {
            let Some(cursor_item) = cursor.current_item() else {
                break;
            };
            let c = cursor_item.content_char();
            let current_point = cursor_item.point();

            // Convert char to array of bytes.
            let mut buf = [0; 4];
            let utf8_len = c.encode_utf8(&mut buf).len();

            // Pass char to DFA as individual bytes.
            for i in 0..utf8_len {
                // Inverse byte order when going left.
                let byte = match direction {
                    Direction::Right => buf[i],
                    Direction::Left => buf[utf8_len - i - 1],
                };

                state = dfa.next_state(&mut cache, state, byte).ok()?;
                if state.is_match() {
                    regex_match = last_point;
                } else if state.is_dead() {
                    // If regex is in a dead state, it will never reach a match state.
                    // Break out of the loop here.
                    break 'outer;
                }
            }

            last_point = Some(current_point);

            // Stop once we've reached the target point.
            if current_point == end {
                break;
            }

            // Handle linebreaks.
            let at_line_break = match direction {
                Direction::Left => cursor.is_at_start_of_line(),
                Direction::Right => cursor.is_at_end_of_line(),
            };
            if at_line_break {
                match regex_match {
                    // If we are at the line break and there is already a match, break out of the loop.
                    Some(_) => break,
                    // If we are at a line break and there is no match, reset the match state.
                    None => {
                        // Before resetting the match state, walk the special "EOI" transition to
                        // check if the DFA now has a match. Since the match state is always delayed
                        // by a byte, this can happen if the the last cell on a line would end up
                        // triggering a match.
                        state = dfa.next_eoi_state(&mut cache, state).ok()?;
                        if state.is_match() {
                            regex_match = last_point;
                            break;
                        }

                        state = dfa
                            .start_state_forward(&mut cache, &Input::new("").anchored(anchored))
                            .ok()?;
                    }
                }
            }

            // Advance grid cell iterator.
            match direction {
                Direction::Right => {
                    cursor.move_forward();
                }
                Direction::Left => {
                    cursor.move_backward();
                }
            };
        }

        state = dfa.next_eoi_state(&mut cache, state).ok()?;
        if state.is_match() {
            regex_match = last_point;
        }

        // Make sure the match point is at the "far" end of any wide character.
        if let Some(match_point) = &mut regex_match {
            if direction == Direction::Right
                && matches!(grid.cell_type(*match_point), Some(CellType::WideChar))
            {
                match_point.col += 1;
            }
        }

        regex_match
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
/// Note: One alternative could be use enable this option:
/// https://docs.rs/regex-automata/0.4.6/regex_automata/hybrid/dfa/struct.Config.html#method.unicode_word_boundary
/// However, "this only works when the search input is ASCII only." This assumption is
/// often false in the terminal context, which often contains emojis, box-drawing chars,
/// international text, etc.
fn replace_unicode_word_boundaries(pattern: &str) -> String {
    pattern.replace("\\b", "(?-u:\\b)")
}
