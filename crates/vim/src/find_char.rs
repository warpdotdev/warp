use warpui::text::TextBuffer;

use crate::vim::{Direction, FindCharDestination, FindCharMotion};

/// Find the destination column for Vim's f/F/t/T motions on a single line.
///
/// This search has four variants based on the direction of search
/// and where the cursor is expected to end up relative to the target character.
/// When moving the cursor to the character before the target,
/// ve must start searching further away from the cursor to
/// ensure we don't re-match the previous target.
///
/// Given the string `abcdefgh` and a cursor on `d`,
/// here's where each type of search starts looking for the target character
/// when repeated:
///
/// 0 1 2 3 4 5 6 7
/// a b c d e f g h
/// __F_| ^ |__f___ (also t/T initially)
/// T_|       |__t_ (when repeated)
///
/// F: (Backward, at target) starts right at the cursor.
/// T: (Backward, before target)` starts one before the cursor when repeated.
/// f: (Forward, at target)` starts one after the cursor.
/// t: (Forward, before target)` starts two after the cursor when repeated.
///
/// Returns Some(new_column) if the target is found on the line based on the parameters; otherwise None.
pub fn vim_find_char_on_line(
    line: &str,
    current_column: usize,
    motion: &FindCharMotion,
    occurrence_count: u32,
    keep_selection: bool,
) -> Option<usize> {
    let FindCharMotion {
        direction,
        destination,
        is_repetition,
        c,
    } = motion;

    let search_start_column = match (direction, destination, is_repetition) {
        (Direction::Backward, FindCharDestination::AtChar, _)
        | (Direction::Backward, FindCharDestination::BeforeChar, false) => current_column,
        (Direction::Backward, FindCharDestination::BeforeChar, true) => {
            current_column.saturating_sub(1)
        }

        // When moving forward, skip the current character under the cursor.
        (Direction::Forward, FindCharDestination::AtChar, _)
        | (Direction::Forward, FindCharDestination::BeforeChar, false) => current_column + 1,
        (Direction::Forward, FindCharDestination::BeforeChar, true) => current_column + 2,
    };

    let found_char = match direction {
        Direction::Backward => {
            line.chars_rev_at(search_start_column.into())
                .ok()
                .and_then(|iter| {
                    iter.enumerate()
                        .filter(|(_, ch)| ch == c)
                        .nth(occurrence_count.saturating_sub(1) as usize)
                })
        }
        Direction::Forward => line
            .chars_at(search_start_column.into())
            .ok()
            .and_then(|iter| {
                iter.enumerate()
                    .filter(|(_, ch)| ch == c)
                    .nth(occurrence_count.saturating_sub(1) as usize)
            }),
    };

    if let Some((i, _)) = found_char {
        let move_distance = match (destination, is_repetition) {
            (FindCharDestination::AtChar, _) | (FindCharDestination::BeforeChar, true) => i + 1,
            (FindCharDestination::BeforeChar, false) => i,
        };

        let mut new_column = match direction {
            Direction::Backward => current_column.saturating_sub(move_distance),
            Direction::Forward => current_column + move_distance,
        };

        // When moving to the right and keeping selection, we include the matched character,
        // so we have to add 1 in this particular case.
        if keep_selection && direction == &Direction::Forward {
            new_column += 1;
        }

        Some(new_column)
    } else {
        None
    }
}
