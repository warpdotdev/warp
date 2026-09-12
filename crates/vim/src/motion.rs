use std::cmp;
use std::ops::Range;

use itertools::Either;
use string_offset::CharOffset;
use warpui_core::text::TextBuffer;

use crate::vim::{
    BracketChar, BracketEnd, CharacterMotion, Direction, FindCharMotion, LineMotion, VimMotion,
    WordBound, WordMotion, WordType,
};
use crate::vim_find_char_on_line;
use crate::word_iterator::CharacterKind;

/// Shared vim motion destination in backend-native offsets.
///
/// `valid` is a contiguous exclusive-end range of real text in the same coordinate space as
/// `offset`. Motions never leave that range. Non-contiguous regions need a richer primitive and
/// are out of scope.
pub fn motion_destination<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    offset: CharOffset,
    motion: &VimMotion,
    count: u32,
) -> CharOffset {
    motion_destination_with_jump(text, valid, offset, motion, count, true)
}

pub fn motion_destination_with_jump<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    offset: CharOffset,
    motion: &VimMotion,
    count: u32,
    jump_first_nonwhitespace: bool,
) -> CharOffset {
    let offset = clamp_offset(valid.clone(), offset);
    let jump = |offset: CharOffset| {
        if jump_first_nonwhitespace {
            first_nonwhitespace(text, valid.clone(), offset)
        } else {
            line_start(text, valid.clone(), offset)
        }
    };
    match motion {
        VimMotion::Character(CharacterMotion::Left) => {
            move_within_line(text, valid, offset, count, Direction::Backward)
        }
        VimMotion::Character(CharacterMotion::Right) => {
            move_within_line(text, valid, offset, count, Direction::Forward)
        }
        VimMotion::Character(CharacterMotion::WrappingLeft) => {
            move_crossing_lines(text, valid, offset, count, Direction::Backward)
        }
        VimMotion::Character(CharacterMotion::WrappingRight) => {
            move_crossing_lines(text, valid, offset, count, Direction::Forward)
        }
        VimMotion::Character(CharacterMotion::Up | CharacterMotion::Down) => offset,
        VimMotion::Word(word_motion) => move_by_word(text, valid, offset, count, word_motion),
        VimMotion::Line(LineMotion::Start) => line_start(text, valid, offset),
        VimMotion::Line(LineMotion::FirstNonWhitespace) => first_nonwhitespace(text, valid, offset),
        VimMotion::Line(LineMotion::End) => line_end_exclusive(text, valid, offset),
        VimMotion::FirstNonWhitespace(_) => first_nonwhitespace(text, valid, offset),
        VimMotion::FindChar(find) => move_to_found_char(text, valid, offset, count, find),
        VimMotion::Paragraph(direction) => {
            move_by_paragraph(text, valid, offset, count, *direction)
        }
        VimMotion::JumpToFirstLine => jump(valid.start),
        VimMotion::JumpToLastLine => {
            if ends_with_newline(text, valid.clone()) {
                valid.end
            } else {
                jump(valid.end)
            }
        }
        VimMotion::JumpToLine(line_number) => {
            jump(jump_to_line_start(text, valid.clone(), *line_number))
        }
        VimMotion::JumpToMatchingBracket => jump_to_matching_bracket(text, valid, offset),
        VimMotion::JumpToUnmatchedBracket(bracket) => {
            matching_bracket(text, valid, bracket, offset).unwrap_or(offset)
        }
    }
}

fn clamp_offset(valid: Range<CharOffset>, offset: CharOffset) -> CharOffset {
    CharOffset::from(
        offset
            .as_usize()
            .clamp(valid.start.as_usize(), valid.end.as_usize()),
    )
}

fn forward_chars<'a, T: TextBuffer + ?Sized>(
    text: &'a T,
    valid: Range<CharOffset>,
    offset: CharOffset,
) -> impl Iterator<Item = char> + 'a {
    let offset = offset.max(valid.start);
    if offset >= valid.end {
        return Either::Right(std::iter::empty());
    }
    let remaining = valid.end.as_usize().saturating_sub(offset.as_usize());
    match text.chars_at(offset) {
        Ok(iter) => Either::Left(iter.take(remaining)),
        Err(_) => Either::Right(std::iter::empty()),
    }
}

fn reverse_chars<'a, T: TextBuffer + ?Sized>(
    text: &'a T,
    valid: Range<CharOffset>,
    offset: CharOffset,
) -> impl Iterator<Item = char> + 'a {
    let offset = offset.min(valid.end);
    if offset <= valid.start {
        return Either::Right(std::iter::empty());
    }
    let remaining = offset.as_usize().saturating_sub(valid.start.as_usize());
    match text.chars_rev_at(offset) {
        Ok(iter) => Either::Left(iter.take(remaining)),
        Err(_) => Either::Right(std::iter::empty()),
    }
}

fn char_at<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    offset: CharOffset,
) -> Option<char> {
    forward_chars(text, valid, offset).next()
}

fn ends_with_newline<T: TextBuffer + ?Sized>(text: &T, valid: Range<CharOffset>) -> bool {
    valid.end > valid.start
        && char_at(
            text,
            valid.clone(),
            CharOffset::from(valid.end.as_usize() - 1),
        ) == Some('\n')
}

fn line_start<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    offset: CharOffset,
) -> CharOffset {
    let mut steps = 0;
    for c in reverse_chars(text, valid.clone(), offset) {
        if c == '\n' {
            break;
        }
        steps += 1;
    }
    CharOffset::from(offset.as_usize().saturating_sub(steps)).max(valid.start)
}

fn line_end_exclusive<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    offset: CharOffset,
) -> CharOffset {
    let mut steps = 0;
    for c in forward_chars(text, valid.clone(), offset) {
        if c == '\n' {
            break;
        }
        steps += 1;
    }
    cmp::min(valid.end, offset + steps)
}

fn first_nonwhitespace<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    offset: CharOffset,
) -> CharOffset {
    let start = line_start(text, valid.clone(), offset);
    let end = line_end_exclusive(text, valid.clone(), offset);
    for (steps, c) in forward_chars(text, valid, start)
        .take(end.as_usize().saturating_sub(start.as_usize()))
        .enumerate()
    {
        if !c.is_whitespace() {
            return start + steps;
        }
    }
    start
}

fn move_within_line<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    offset: CharOffset,
    count: u32,
    direction: Direction,
) -> CharOffset {
    let start = line_start(text, valid.clone(), offset);
    let end = line_end_exclusive(text, valid, offset);
    match direction {
        Direction::Backward => {
            let dist = u32::min(
                count,
                offset.as_usize().saturating_sub(start.as_usize()) as u32,
            );
            CharOffset::from(offset.as_usize().saturating_sub(dist as usize))
        }
        Direction::Forward => {
            let dist = u32::min(
                count,
                end.as_usize().saturating_sub(offset.as_usize()) as u32,
            );
            cmp::min(end, offset + dist as usize)
        }
    }
}

fn move_crossing_lines<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    mut offset: CharOffset,
    count: u32,
    direction: Direction,
) -> CharOffset {
    for _ in 0..count {
        match direction {
            Direction::Forward => {
                if offset >= valid.end {
                    break;
                }
                let next = cmp::min(valid.end, offset + 1);
                if char_at(text, valid.clone(), next) == Some('\n') {
                    let after = cmp::min(valid.end, next + 1);
                    offset = if char_at(text, valid.clone(), after) == Some('\n') {
                        next
                    } else {
                        after
                    };
                } else {
                    offset = next;
                }
            }
            Direction::Backward => {
                if offset <= valid.start {
                    break;
                }
                let prev = CharOffset::from(offset.as_usize().saturating_sub(1)).max(valid.start);
                if char_at(text, valid.clone(), prev) == Some('\n') {
                    let prev2 =
                        CharOffset::from(prev.as_usize().saturating_sub(1)).max(valid.start);
                    offset = if char_at(text, valid.clone(), prev2) == Some('\n') {
                        prev
                    } else {
                        prev2
                    };
                } else {
                    offset = prev;
                }
            }
        }
    }
    offset
}

fn clamp_to_valid(valid: &Range<CharOffset>, offset: CharOffset) -> CharOffset {
    offset.max(valid.start).min(valid.end)
}

fn reverse_origin(offset: CharOffset, valid: &Range<CharOffset>) -> CharOffset {
    (offset + 1).min(valid.end)
}

fn move_by_word<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    offset: CharOffset,
    count: u32,
    word_motion: &WordMotion,
) -> CharOffset {
    let WordMotion {
        direction,
        bound,
        word_type,
    } = word_motion;
    let dest = word_offsets(text, valid.clone(), offset, *direction, *bound, *word_type)
        .take(count as usize)
        .last()
        .unwrap_or(offset);
    clamp_to_valid(&valid, dest)
}

fn word_offsets<'a, T: TextBuffer + ?Sized>(
    text: &'a T,
    valid: Range<CharOffset>,
    offset: CharOffset,
    direction: Direction,
    bound: WordBound,
    word_type: WordType,
) -> Box<dyn Iterator<Item = CharOffset> + 'a> {
    match (direction, bound) {
        (Direction::Forward, WordBound::Start) | (Direction::Backward, WordBound::End) => {
            Box::new(WordHeads::new(text, valid, offset, direction, word_type))
        }
        (Direction::Forward, WordBound::End) | (Direction::Backward, WordBound::Start) => {
            Box::new(WordTails::new(text, valid, offset, direction, word_type))
        }
    }
}

struct WordHeads<'a> {
    offset: CharOffset,
    start: CharOffset,
    end: CharOffset,
    chars: std::iter::Peekable<Box<dyn Iterator<Item = char> + 'a>>,
    cursor_context: CharacterKind,
    direction: Direction,
    word_type: WordType,
    done: bool,
}

impl<'a> WordHeads<'a> {
    fn new<T: TextBuffer + ?Sized>(
        text: &'a T,
        valid: Range<CharOffset>,
        offset: CharOffset,
        direction: Direction,
        word_type: WordType,
    ) -> Self {
        let start = valid.start;
        let end = valid.end;
        let (offset, chars) = match direction {
            Direction::Backward => {
                let offset = reverse_origin(offset, &valid);
                (
                    offset,
                    Box::new(reverse_chars(text, valid, offset))
                        as Box<dyn Iterator<Item = char> + 'a>,
                )
            }
            Direction::Forward => (
                offset,
                Box::new(forward_chars(text, valid, offset)) as Box<dyn Iterator<Item = char> + 'a>,
            ),
        };
        let mut chars = chars.peekable();
        Self {
            start,
            end,
            offset,
            cursor_context: chars
                .peek()
                .map_or(CharacterKind::WordChars, |c| (*c).into()),
            chars,
            direction,
            word_type,
            done: false,
        }
    }

    fn step(&mut self) {
        self.chars.next();
        match self.direction {
            Direction::Backward => {
                self.offset =
                    CharOffset::from(self.offset.as_usize().saturating_sub(1)).max(self.start);
            }
            Direction::Forward => {
                if self.offset < self.end {
                    self.offset += 1;
                }
            }
        }
    }
}

impl Iterator for WordHeads<'_> {
    type Item = CharOffset;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        loop {
            self.step();
            let Some(&c) = self.chars.peek() else {
                break;
            };
            let prev_cursor_context = self.cursor_context;
            self.cursor_context = CharacterKind::from(c);

            if !self
                .cursor_context
                .equivalent_char_kind(&prev_cursor_context, self.word_type)
                && self.cursor_context != CharacterKind::Whitespace
            {
                return Some(match self.direction {
                    Direction::Backward => {
                        CharOffset::from(self.offset.as_usize().saturating_sub(1)).max(self.start)
                    }
                    Direction::Forward => self.offset.min(self.end),
                });
            }
        }

        self.done = true;
        Some(self.offset.max(self.start).min(self.end))
    }
}

struct WordTails<'a> {
    offset: CharOffset,
    start: CharOffset,
    end: CharOffset,
    chars: itertools::PeekNth<Box<dyn Iterator<Item = char> + 'a>>,
    direction: Direction,
    word_type: WordType,
    done: bool,
}

impl<'a> WordTails<'a> {
    fn new<T: TextBuffer + ?Sized>(
        text: &'a T,
        valid: Range<CharOffset>,
        offset: CharOffset,
        direction: Direction,
        word_type: WordType,
    ) -> Self {
        let start = valid.start;
        let end = valid.end;
        let (offset, chars) = match direction {
            Direction::Backward => {
                let offset = reverse_origin(offset, &valid);
                (
                    offset,
                    itertools::peek_nth(Box::new(reverse_chars(text, valid, offset))
                        as Box<dyn Iterator<Item = char> + 'a>),
                )
            }
            Direction::Forward => (
                offset,
                itertools::peek_nth(Box::new(forward_chars(text, valid, offset))
                    as Box<dyn Iterator<Item = char> + 'a>),
            ),
        };
        Self {
            start,
            end,
            offset,
            chars,
            direction,
            word_type,
            done: false,
        }
    }

    fn step(&mut self) {
        self.chars.next();
        match self.direction {
            Direction::Backward => {
                self.offset =
                    CharOffset::from(self.offset.as_usize().saturating_sub(1)).max(self.start);
            }
            Direction::Forward => {
                if self.offset < self.end {
                    self.offset += 1;
                }
            }
        }
    }
}

impl Iterator for WordTails<'_> {
    type Item = CharOffset;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        loop {
            if self.chars.peek_nth(1).is_none() {
                break;
            }
            self.step();

            let Some(&c) = self.chars.peek() else {
                break;
            };
            let Some(&c_next) = self.chars.peek_nth(1) else {
                break;
            };

            let cursor_context = CharacterKind::from(c);
            let cursor_context_next = CharacterKind::from(c_next);

            if !cursor_context.equivalent_char_kind(&cursor_context_next, self.word_type)
                && cursor_context != CharacterKind::Whitespace
            {
                return Some(match self.direction {
                    Direction::Backward => {
                        CharOffset::from(self.offset.as_usize().saturating_sub(1)).max(self.start)
                    }
                    Direction::Forward => self.offset.min(self.end),
                });
            }
        }

        self.done = true;
        Some(match self.direction {
            Direction::Backward => {
                CharOffset::from(self.offset.as_usize().saturating_sub(1)).max(self.start)
            }
            Direction::Forward => self.offset.min(self.end),
        })
    }
}

fn move_to_found_char<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    offset: CharOffset,
    occurrence_count: u32,
    motion: &FindCharMotion,
) -> CharOffset {
    let start = line_start(text, valid.clone(), offset);
    let end = line_end_exclusive(text, valid.clone(), offset);
    let Some(line) = char_slice_owned(text, valid, start, end) else {
        return offset;
    };
    let column = offset.as_usize().saturating_sub(start.as_usize());
    match vim_find_char_on_line(&line, column, motion, occurrence_count, false) {
        Some(new_column) => start + new_column,
        None => offset,
    }
}

fn move_by_paragraph<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    offset: CharOffset,
    count: u32,
    direction: Direction,
) -> CharOffset {
    let mut current = offset;
    match direction {
        Direction::Forward => {
            for _ in 0..count {
                current = paragraph_end(text, valid.clone(), current).unwrap_or(valid.end);
            }
        }
        Direction::Backward => {
            for _ in 0..count {
                current = paragraph_start(text, valid.clone(), current).unwrap_or(valid.start);
            }
        }
    }
    current
}

fn paragraph_start<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    offset: CharOffset,
) -> Option<CharOffset> {
    let origin = reverse_origin(offset, &valid);
    let iter = reverse_chars(text, valid.clone(), origin)
        .enumerate()
        .skip_while(|(_, c)| *c == '\n');
    let mut prev_was_newline = false;
    for (curr, c) in iter {
        if c == '\n' {
            if prev_was_newline {
                return Some(
                    CharOffset::from(origin.as_usize().saturating_sub(curr)).max(valid.start),
                );
            }
            prev_was_newline = true;
        } else {
            prev_was_newline = false;
        }
    }
    None
}

fn paragraph_end<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    offset: CharOffset,
) -> Option<CharOffset> {
    let iter = forward_chars(text, valid, offset)
        .enumerate()
        .skip_while(|(_, c)| *c == '\n');
    let mut prev_was_newline = false;
    for (curr, c) in iter {
        if c == '\n' {
            if prev_was_newline {
                return Some(offset + curr);
            }
            prev_was_newline = true;
        } else {
            prev_was_newline = false;
        }
    }
    None
}

fn jump_to_line_start<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    line_number: u32,
) -> CharOffset {
    let mut start = valid.start;
    let max = valid.end;
    let target = line_number.max(1);
    for _ in 1..target {
        let end = line_end_exclusive(text, valid.clone(), start);
        if end >= max {
            break;
        }
        start = end + 1;
    }
    start
}

fn jump_to_matching_bracket<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    offset: CharOffset,
) -> CharOffset {
    let mut iter = forward_chars(text, valid.clone(), offset).take_while(|c| *c != '\n');
    let Some(c) = iter.next() else {
        return offset;
    };
    let (bracket, start_offset) = match BracketChar::try_from(c) {
        Ok(bracket) => (bracket, offset),
        Err(()) => match iter
            .enumerate()
            .find_map(|(i, ch)| Some((i, BracketChar::try_from(ch).ok()?)))
        {
            None => return offset,
            Some((i, bracket)) => (bracket, offset + i + 1),
        },
    };
    matching_bracket(text, valid, &bracket, start_offset).unwrap_or(offset)
}

fn matching_bracket<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    bracket_char: &BracketChar,
    offset: CharOffset,
) -> Option<CharOffset> {
    let mut iter: Box<dyn Iterator<Item = char>> = match bracket_char.end {
        BracketEnd::Opening => Box::new(forward_chars(text, valid.clone(), offset + 1)),
        BracketEnd::Closing => Box::new(reverse_chars(text, valid.clone(), offset)),
    };
    let mut depth: u32 = 0;
    let (i, _) = itertools::Itertools::find_position(&mut iter, |c| {
        if bracket_char.is_char(*c) {
            depth += 1;
        } else if bracket_char.complements(*c) {
            if depth == 0 {
                return true;
            } else {
                depth -= 1;
            }
        }
        false
    })?;
    match bracket_char.end {
        BracketEnd::Opening => Some(offset + i + 1),
        BracketEnd::Closing => {
            Some(CharOffset::from(offset.as_usize().saturating_sub(i + 1)).max(valid.start))
        }
    }
}

fn char_slice_owned<T: TextBuffer + ?Sized>(
    text: &T,
    valid: Range<CharOffset>,
    start: CharOffset,
    end: CharOffset,
) -> Option<String> {
    let s = start.as_usize();
    let e = end.as_usize();
    if e < s {
        return None;
    }
    Some(
        forward_chars(text, valid, start)
            .take(e.saturating_sub(s))
            .collect(),
    )
}

#[cfg(test)]
#[path = "motion_tests.rs"]
mod tests;
