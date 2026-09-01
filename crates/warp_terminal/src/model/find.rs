use std::ops::RangeInclusive;

use regex_automata::{Anchored, Input};
pub use regex_dfas::{CachePoolFn, FindConfig, RegexDFAs};

use super::grid::grapheme_cursor;
use super::grid::grid_handler::GridHandler;
use crate::model::grid::CellType;
use crate::model::index::{Direction, Point};

pub type Match = RangeInclusive<Point>;

impl GridHandler {
    /// Find the next regex match to the right of the origin point by beginning at the `left` Point
    /// and searching until the `right` Point is reached, inclusive of both points.
    ///
    /// The origin is always included in the regex.
    pub(crate) fn regex_search_rightwards(
        &self,
        dfas: &RegexDFAs,
        left: Point,
        right: Point,
    ) -> Option<Match> {
        // Scan from the left -> right to find the end (rightmost) point of the match.
        let match_right_point = self.search(dfas, left, right, Direction::Right, Anchored::No)?;

        // Scan leftwards from the match end to the left most point to find the beginning (leftmost) point of the match.
        let match_left_point = self.search(
            dfas,
            match_right_point,
            left,
            Direction::Left,
            Anchored::Yes,
        )?;

        Some(match_left_point..=match_right_point)
    }

    /// Find the next regex match to the left of the `right` Point by searching leftwards from
    /// `right` until the `left` Point is reached.
    ///
    /// The origin is always included in the regex.
    pub(crate) fn regex_search_leftwards(
        &self,
        dfas: &RegexDFAs,
        right: Point,
        left: Point,
    ) -> Option<Match> {
        // Scan leftwards to find the starting (leftmost) point of the match.
        let match_left_point = self.search(dfas, right, left, Direction::Left, Anchored::No)?;
        // Scan rightwards from the match start to the rightmost point to find the end (rightmost) point of the match.
        let match_right_point = self.search(
            dfas,
            match_left_point,
            right,
            Direction::Right,
            Anchored::Yes,
        )?;

        Some(match_left_point..=match_right_point)
    }

    /// Find the next regex match, given a direction.
    ///
    /// This will always return the side of the first match which is farthest from the start point.
    fn search(
        &self,
        dfas: &RegexDFAs,
        start: Point,
        end: Point,
        direction: Direction,
        anchored: Anchored,
    ) -> Option<Point> {
        let (dfa, mut cache) = match direction {
            Direction::Left => dfas.get_reverse(),
            Direction::Right => dfas.get_forward(),
        };

        let mut cursor = self.grapheme_cursor_from(start, grapheme_cursor::Wrap::All);

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
        if let Some(match_point) = &mut regex_match
            && direction == Direction::Right
            && matches!(self.cell_type(*match_point), Some(CellType::WideChar))
        {
            match_point.col += 1;
        }

        regex_match
    }
}
