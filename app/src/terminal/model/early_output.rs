use std::collections::{BTreeSet, HashMap, VecDeque};
use std::mem;

use pathfinder_color::ColorU;
use string_offset::CharOffset;
use warp_errors::report_error;
use warp_terminal::model::{KeyboardModes, KeyboardModesApplyBehavior};

use super::ansi;
use super::block::Block;
use super::blocks::BlockList;
use super::image_map::StoredImageMetadata;
use super::iterm_image::ITermImage;
use super::kitty::{KittyAction, KittyResponse};
use super::selection::ScrollDelta;
use super::session::SessionInfo;
use crate::safe_debug;
use crate::terminal::event::Event as TerminalEvent;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::view::CONTROL_MASTER_ERROR_REGEX;

#[cfg(test)]
#[path = "early_output_tests.rs"]
mod tests;

/// The approach we're using to detect user typeahead.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum TypeaheadMode {
    /// The shell reports its input buffer to Warp, and we use that for typeahead.
    ShellReported,
    /// Warp matches user input against characters echoed to the PTY to estimate typeahead.
    /// This is only used on bash 3.2 and should be removed if we stop supporting
    /// such old bash versions.
    InputMatching,
}

/// Model for "early" terminal output. Early output is output that Warp receives
/// from the PTY while no block is running. In concrete terms, it's output received
/// after a `BlockFinished` hook but before Warp has written the next command from
/// the input editor to the PTY.
///
/// This output belongs to one of two categories:
/// 1. Typeahead - if a user types while a command is running, but the command
///    doesn't read that input, it's echoed as the basis for the next command. This
///    lets users queue up commands if their connection is slow or a command
///    takes longer than expected.
/// 2. Background output - if a background job is running, it can print output
///    outside the context of a running block. Additionally, the shell might
///    print messages about job completion.
pub struct EarlyOutput {
    mode: TypeaheadMode,

    /// The currently-accumulated typeahead.
    typeahead: String,

    /// Counter for the number of typeahead characters inserted into the current
    /// input buffer. We can receive multiple typeahead events, so this counter
    /// tells us how many characters to replace with new typeahead.
    typeahead_chars_inserted: CharOffset,

    /// User input that may be typeahead, which is matched against echoed text.
    unmatched_input: VecDeque<char>,
    /// Characters registered via `push_expected_echo`, matched (possibly more than once, see
    /// `expected_echo_positions`) against echoed text. Unlike `unmatched_input`, a match here is
    /// dropped entirely rather than surfaced as typeahead -- see `push_expected_echo`.
    expected_echo: Vec<char>,
    /// Every position in `expected_echo` that could currently be "next" to match, given
    /// everything matched so far. Matched characters are never removed from `expected_echo`
    /// itself: a line editor's redraw can re-echo the same restored buffer an arbitrary number
    /// of times per restore, using more than one way of moving the cursor to do it. Which way a
    /// given session uses is not incidental: with Warp drawing the prompt, the restored line
    /// starts at column 0, so ZLE can rewind with a plain carriage return; with
    /// `terminal.input.honor_ps1 = true` (the shell draws its own prompt), the line no longer
    /// starts at column 0, so ZLE switches its rewind onto *relative* motion -- a backspace or
    /// CUB -- instead; PSReadLine's own redraw uses *absolute* cursor addressing (CUP, e.g.
    /// `\x1b[1;1H`) regardless. So `honor_ps1` and which line editor is driving are what select
    /// among the rewind mechanisms below, not just a detail of one particular measured session.
    /// Five motions are handled, all measured against real sessions:
    /// - A carriage return -- measured as either a restart of the whole line from the
    ///   beginning, or a brief mid-line return-to-column-0 that continues the same line from
    ///   wherever it left off, rather than restarting. Since it isn't known in advance which of
    ///   those a given carriage return means, `rearm_at_column(0)` adds position 0 as a new
    ///   candidate without discarding whatever was already live, so both possibilities stay
    ///   open until the characters that follow resolve it.
    /// - Absolute cursor addressing (CUP/CHA, `goto`/`goto_col`), measured from PSReadLine's own
    ///   redraw -- but only trusted when the column is 0. A carriage return unconditionally
    ///   means "column 0", by definition; an arbitrary absolute column from CUP/CHA does not
    ///   unconditionally mean "the buffer's own position 0" -- if a redraw re-renders a
    ///   nonempty prompt together with the buffer, the column addressed partway through that
    ///   redraw could just as easily be the prompt's own width as the buffer's start, and
    ///   nothing in the byte stream distinguishes the two cases. Column 0 is safe under either
    ///   reading (it is either genuinely the buffer's start, or so early into a redrawn prompt
    ///   that nothing of the pattern could be confused with prompt text there); anything else
    ///   is deliberately left as no information rather than risked as a wrong position. This is
    ///   narrower than what was measured (a real session with a nonempty prompt has not yet
    ///   been captured to confirm or rule out the wider-column concern) and should be widened
    ///   only once that measurement exists, via the same `rearm_at_column(column)` backing
    ///   both this and the carriage-return case above.
    /// - A backspace or CUB (`move_backward`), whose distance *is* known from the input itself
    ///   (always 1 for a backspace; the escape sequence's own parameter for CUB) --
    ///   `rearm_after_rewind` shifts every existing candidate back by that exact distance and
    ///   adds the result, again without discarding the originals, since whether a redraw
    ///   actually follows is also not known in advance.
    /// - CUF (`move_forward`), the forward counterpart, also with a known distance from the
    ///   escape sequence -- `advance_after_forward_move` shifts every existing candidate
    ///   forward by that distance and adds the result. Unlike a rewind, a forward move isn't
    ///   ambiguous about whether a redraw follows: the terminal is asserting the columns it
    ///   just skipped over already hold the correct content, so every live candidate's
    ///   position genuinely advances. Without this, a candidate left stranded at its pre-move
    ///   position after a CUF would still correctly match nothing further until something
    ///   arrives, but would mismatch (and leak) if that something turns out to be a
    ///   continuation of the pattern past what the CUF skipped over.
    /// A trailing-fragment-only redraw (e.g. re-echoing just the line's last character after a
    /// full match) turned out, in every measured case, to be a full match leaving a candidate
    /// at the pattern's length, followed by a backspace or CUB shifting it back by the rewind
    /// distance, not an ambiguous carriage return. A wider rule that seeded every position on a
    /// carriage return to cover this same shape was tried and reverted once the measured cause
    /// turned out to be a rewind with a known distance -- keep the rearm rules scoped to what's
    /// actually been measured rather than to what would also happen to work.
    /// Every rearm/advance method here follows the same shape: each subsequent character
    /// advances every candidate whose next expected character matches it (dropping the rest) --
    /// so a character counts as expected echo if *any* live candidate predicts it, however many
    /// candidates that turns out to be. This is meant to be scoped to a single restore's own
    /// redraw window (nothing populates `expected_echo` outside of `push_expected_echo`, each
    /// restore replaces it outright rather than accumulating across restores, and
    /// `reset_expected_echo` is what closes the window -- see its own doc comment for when).
    /// Cursor motions other than these five are not handled; a redraw shape using one would
    /// need the same treatment as those above.
    expected_echo_positions: BTreeSet<usize>,
    /// Whether the last potential typeahead character received on the PTY was a
    /// carriage return. We can't rely on the last character of `typeahead` for
    /// this, because it only stores _matched_ typeahead.
    just_matched_carriage_return: bool,

    /// The event proxy sends terminal events (in this case, typeahead), to the
    /// terminal view.
    event_proxy: ChannelEventListener,
    pending_background_block: Option<Block>,
}

impl EarlyOutput {
    /// Creates a new `EarlyOutput` model. The event proxy is used to notify
    /// the terminal view about new typeahead.
    pub fn new(event_proxy: ChannelEventListener) -> Self {
        Self {
            // Default to InputMatching as a baseline for all shells.
            mode: TypeaheadMode::InputMatching,
            typeahead: String::new(),
            typeahead_chars_inserted: 0.into(),
            unmatched_input: VecDeque::new(),
            expected_echo: Vec::new(),
            expected_echo_positions: BTreeSet::new(),
            just_matched_carriage_return: false,
            event_proxy,
            pending_background_block: None,
        }
    }

    /// Configures the typeahead mode to use given the features that the current
    /// shell session supports.
    pub fn init_session(&mut self, session_info: &SessionInfo) {
        let supports_input_reporting = session_info.shell.input_reporting_sequence().is_some();
        self.mode = if supports_input_reporting {
            TypeaheadMode::ShellReported
        } else {
            TypeaheadMode::InputMatching
        };
        log::info!("Configured typeahead mode as {:?}", self.mode);
    }

    /// Returns a reference to the current typeahead.
    pub fn typeahead(&self) -> &str {
        &self.typeahead
    }

    /// Record input from the user as potential typeahead.
    pub fn push_user_input(&mut self, input: &str) {
        if self.mode == TypeaheadMode::InputMatching {
            self.unmatched_input.extend(input.chars().filter(|ch| {
                // Only keep control characters that we expect to match in the echoed typeahead.
                !ch.is_ascii_control() || *ch == '\r'
            }));
        }
    }

    /// Registers `input` as characters expected to be echoed back on the pty, regardless of the
    /// active `TypeaheadMode`.
    ///
    /// Used when something other than normal user typing deliberately writes bytes to the pty
    /// outside of a command submission, where the caller already has an accurate copy of that
    /// text elsewhere (e.g. an input editor's own buffer) and just wants the resulting echo not
    /// to be treated as unexpected background output (which would otherwise start a background
    /// block). This is deliberately *not* the same as `push_user_input`/typeahead: a matched
    /// typeahead character is surfaced via `TerminalEvent::Typeahead` so it can be *inserted*
    /// into the input editor, which is correct when the editor doesn't already have it, but
    /// would duplicate it here, since the editor's copy was never cleared in the first place --
    /// only the real shell's buffer was. A match here is instead dropped entirely: neither
    /// rendered as background output nor surfaced as typeahead.
    ///
    /// Example: `PtyController` restoring the input buffer after an in-band/generator command
    /// that necessarily cleared the shell's real buffer to run it as a foreground command.
    ///
    /// Replaces any previously-registered content outright, rather than appending to it: each
    /// restore is a fresh, independent echo to expect, not a continuation of the last one.
    pub fn push_expected_echo(&mut self, input: &str) {
        self.expected_echo = input
            .chars()
            .filter(|ch| {
                // Only keep control characters that we expect to match in the echoed text.
                !ch.is_ascii_control() || *ch == '\r'
            })
            .collect();
        self.expected_echo_positions = BTreeSet::from([0]);
    }

    /// Reset the unmatched user input. This is called between blocks so that
    /// unmatched potential typeahead from one command doesn't throw off input
    /// matching for the rest of the session.
    pub fn reset_user_input(&mut self) {
        self.unmatched_input.clear();
    }

    /// Clears any registration made via `push_expected_echo`. Called from two places:
    /// `BlockList::start_active_block` (never `start_active_block_for_in_band_command`, which
    /// is what a generator/completions request's own command uses), so a pattern left over from
    /// the last restore can't outlive into a real, unrelated command's own output; and
    /// `EarlyOutputHandler::input()`, on the first *real* character the pattern can't explain
    /// (see that method for why control-byte probes must not trigger this). Two prior attempts
    /// at bounding this window were tried and found wrong before landing here:
    /// `LineEditorStatusEvent::Active` fires *before* the restore write is even flushed, so
    /// clearing there discarded the whole restored buffer rather than protecting it; and
    /// clearing inside `consume_expected_echo` itself on *any* mismatch broke every
    /// carriage-return-driven redraw, since `carriage_return()`/`linefeed()` probe it with
    /// `'\r'`/`'\n'`, which a restored buffer's text never contains, guaranteeing a mismatch on
    /// every single redraw regardless of whether the real echo matched.
    pub fn reset_expected_echo(&mut self) {
        self.expected_echo.clear();
        self.expected_echo_positions.clear();
    }

    /// Returns whether the next user input character matches `ch`. If it does
    /// match, the character is consumed.
    fn consume_user_input(&mut self, ch: char) -> bool {
        let is_match = self.unmatched_input.front() == Some(&ch);
        if is_match {
            self.unmatched_input.pop_front();
        }
        is_match
    }

    /// Returns whether `ch` matches the next expected character for *any* currently-live
    /// candidate position (see `expected_echo_positions`). If at least one does, every matching
    /// candidate advances by one and every non-matching one is dropped (a character can only be
    /// consumed once, so a candidate that guessed wrong here can't be right going forward
    /// either). A match should be dropped entirely by the caller rather than surfaced as
    /// typeahead.
    ///
    /// Deliberately has no side effect on a mismatch: this is also probed with `'\r'`/`'\n'`
    /// from `carriage_return()`/`linefeed()` to decide whether those bytes are part of the
    /// registered pattern (they never are -- a restored buffer's text contains neither), and a
    /// mismatch there is not evidence the echo is over. See `EarlyOutputHandler::input()` for
    /// where a mismatch instead ends the window.
    fn consume_expected_echo(&mut self, ch: char) -> bool {
        let next_positions: BTreeSet<usize> = self
            .expected_echo_positions
            .iter()
            .filter_map(|&position| {
                (self.expected_echo.get(position) == Some(&ch)).then_some(position + 1)
            })
            .collect();
        let is_match = !next_positions.is_empty();
        if is_match {
            self.expected_echo_positions = next_positions;
        }
        is_match
    }

    /// If anything is registered via `push_expected_echo`, adds `column` to the set of live
    /// candidates (see `expected_echo_positions`) without discarding whatever was already
    /// there. Called regardless of how much of the current pass matched -- see
    /// `expected_echo_positions`'s doc comment for the two shapes a carriage return in
    /// particular covers.
    ///
    /// This assumes `column` is itself a valid absolute position in the pattern -- i.e. that
    /// the pattern's echo starts at that same column. That is unconditionally true for a
    /// carriage return, whose `column` argument is always 0 by definition. It is *not*
    /// unconditionally true for absolute cursor addressing (see the call sites in `goto`/
    /// `goto_col`): when a prompt occupies columns before the buffer starts, a redraw that
    /// re-renders the prompt and buffer together would address a column reflecting the prompt's
    /// width, not 0, and only the caller can know whether that is the case here. Column 0
    /// itself is always safe to trust regardless -- it is either genuinely the buffer's start,
    /// in which case this is correct, or it is mid-prompt, in which case no candidate the
    /// pattern's own characters could confuse with prompt text lives there anyway.
    fn rearm_at_column(&mut self, column: usize) {
        if !self.expected_echo.is_empty() {
            self.expected_echo_positions.insert(column);
        }
    }

    /// Shifts every currently-live candidate position back by `distance` and adds the result to
    /// the set of live candidates, without discarding the originals (see
    /// `expected_echo_positions`'s doc comment for why the originals stay live too). Called on
    /// a backspace (`distance` 1) or CUB (`distance` the column count from the escape
    /// sequence), where -- unlike a carriage return -- the exact rewind distance is known, so
    /// each candidate's new position can be computed directly rather than seeding every
    /// position in the pattern. A candidate whose position is less than `distance` (the rewind
    /// would move before where this candidate's matching started) is dropped rather than
    /// underflowing.
    fn rearm_after_rewind(&mut self, distance: usize) {
        let rewound: BTreeSet<usize> = self
            .expected_echo_positions
            .iter()
            .filter_map(|&position| position.checked_sub(distance))
            .collect();
        self.expected_echo_positions.extend(rewound);
    }

    /// Whether at least one live candidate position (see `expected_echo_positions`) still has
    /// more of the registered pattern ahead of it. Used to scope wrapped-line space-fill
    /// swallowing (see `EarlyOutputHandler::input()`) to redraws that are genuinely still in
    /// progress, rather than to every space received for as long as the window happens to stay
    /// open.
    fn awaiting_more_expected_echo(&self) -> bool {
        self.expected_echo_positions
            .iter()
            .any(|&position| position < self.expected_echo.len())
    }

    /// Shifts every currently-live candidate position forward by `distance` and adds the
    /// result to the set of live candidates, without discarding the originals. Called on CUF
    /// (`move_forward`) with the column count from the escape sequence. Unlike a rewind, where
    /// a redraw of the skipped-over content may or may not follow, a forward move is not
    /// ambiguous: the terminal is asserting that the columns it just skipped over already hold
    /// the correct content (moving over content without redrawing it would otherwise leave
    /// wrong content on screen), so every live candidate's logical position genuinely advances
    /// by `distance`. Measured case: a syntax-highlighting recolour pass that ends with a CUF
    /// skipping over an unchanged trailing fragment of the line, after which a live candidate
    /// left at its pre-move position would be stranded -- correctly matching nothing further
    /// until whatever comes after the skipped fragment arrives, and mismatching (and leaking)
    /// if that turns out to be a continuation of the pattern rather than something new.
    fn advance_after_forward_move(&mut self, distance: usize) {
        let advanced: BTreeSet<usize> = self
            .expected_echo_positions
            .iter()
            .map(|&position| position + distance)
            .collect();
        self.expected_echo_positions.extend(advanced);
    }

    /// Check a character received on the PTY, which may be typeahead or
    /// background output.
    fn handle_potential_typeahead(&mut self, ch: char) -> bool {
        let is_typeahead = match self.mode {
            TypeaheadMode::InputMatching => {
                // By default, the ONLCR TTY option is set, so carriage returns (from
                // the enter key) are echoed as `\r\n`. If we match a carriage return
                // as typeahead, we want to match the newline as well.
                self.consume_user_input(ch) || (self.just_matched_carriage_return && ch == '\n')
            }
            _ => false,
        };
        self.just_matched_carriage_return = is_typeahead && ch == '\r';

        if is_typeahead {
            self.typeahead.push(ch);
            safe_debug!(
                safe: ("Matched PTY output as typeahead"),
                full: ("Matched {ch:?} as typeahead")
            );

            if warp_core::channel::ChannelState::channel()
                == warp_core::channel::Channel::Integration
            {
                log::info!(
                    "Sending input-matched typeahead event for {:?}",
                    self.typeahead
                );
            }

            self.event_proxy
                .send_terminal_event(TerminalEvent::Typeahead);
        }
        is_typeahead
    }

    /// Fetch and advance the current typeahead state. This returns the accumulated
    /// typeahead along with the count of previous typeahead to overwrite. The
    /// internal count is then updated to match the new typeahead length.
    pub fn advance_typeahead(&mut self) -> Option<(&str, CharOffset)> {
        if self.typeahead.is_empty() {
            if warp_core::channel::ChannelState::channel()
                == warp_core::channel::Channel::Integration
            {
                log::warn!("Tried to advance typeahead, but it was empty");
            }

            None
        } else {
            let prev_inserted = self.typeahead_chars_inserted;
            self.typeahead_chars_inserted = self.typeahead.chars().count().into();
            Some((&self.typeahead, prev_inserted))
        }
    }

    /// Update typeahead state before the next command. This is called from the
    /// blocklist's precmd hook, but doesn't implement the [`ansi::Handler`]
    /// interface because it doesn't need precmd data.
    pub fn precmd(&mut self) {
        // On precmd, clear accumulated typeahead for the previous command.
        safe_debug!(
            safe: ("Clearing accumulated typeahead"),
            full: ("Clearing accumulated typeahead: {:?}", self.typeahead)
        );
        self.typeahead.clear();
        self.typeahead_chars_inserted = 0.into();

        // Deliberately not clearing `expected_echo` here: `CompletionsFinished` (and the
        // `push_expected_echo` call it triggers) fires when `9280;B` is parsed, which precedes
        // the shell's own in-band-command precmd DCS -- so precmd normally lands *inside* the
        // restore window, not after it, and clearing here would wipe a registration before its
        // own echo has even arrived (measured: roughly five in six restores). Staleness is
        // already bounded by `push_expected_echo` replacing its content outright on every call.
    }

    /// Update early output state once the next command has started running. After
    /// this point, output we receive is no longer "early". This is called from
    /// the blocklist's preexec hook, but doesn't implement the [`ansi::Handler`]
    /// interface because it doesn't need preexec data.
    pub fn preexec(block_list: &mut BlockList) {
        if block_list.early_output().mode == TypeaheadMode::ShellReported {
            // We use this to fill in the command grid for commands that are submitted as typeahead (the
            // user types in a command and hits Enter before the previous command) finishes.
            // When commands are queued up like this, the shell runs them back-to-back.
            // For most user-entered commands, we know when to switch from background
            // output to the active block's command grid because the input editor
            // marks the block as started right before it sends the command to the pty.
            // When the command doesn't come from Warp, however, the active block isn't
            // started until we receive the preexec hook. At this point, the shell has
            // already written the command to the pty, resulting in Warp treating it as
            // background output.
            // We can't correctly identify the command in advance when this happens, so
            // instead we fix the block list afterwards.
            if !block_list.active_block().started()
                && let Some(background_block) = block_list.remove_background_block()
            {
                log::debug!("Repairing command from background block");
                block_list
                    .active_block_mut()
                    .copy_command_grid(background_block.output_grid());
                block_list.update_active_block_height();
            }
        }
    }

    /// Returns an [`ansi::Handler`] adapter for early output.
    pub(super) fn handler(block_list: &mut BlockList) -> impl ansi::Handler + '_ {
        EarlyOutputHandler { block_list }
    }

    /// Returns a mutable reference to the pending background block, if one
    /// exists.
    pub(super) fn pending_background_block_mut(&mut self) -> Option<&mut Block> {
        self.pending_background_block.as_mut()
    }
}

/// [`ansi::Handler`] adapter for [`EarlyOutput`]. To handle early output, we
/// need a reference to the [`BlockList`], for creating background output blocks.
/// Since `BlockList` owns `EarlyOutput`, `EarlyOutput` can't hold a reference to
/// its parent. Instead, this adapter temporarily references the `BlockList` and,
/// by extension, the `EarlyOutput`.
struct EarlyOutputHandler<'a> {
    block_list: &'a mut BlockList,
}

impl EarlyOutputHandler<'_> {
    fn inner(&mut self) -> &mut EarlyOutput {
        self.block_list.early_output_mut()
    }

    /// Runs `f` against the current background output block, creating a new one
    /// if needed.
    ///
    /// If the block was already live, this updates the block heights SumTree
    /// if needed. If the block starts as a result of `f`, it's added to the
    /// block list.
    fn with_background_output<T>(&mut self, f: impl FnOnce(&mut Block) -> T) -> T {
        fn store_pending_block(block_list: &mut BlockList, block: Block) {
            if block.started() {
                block_list.insert_background_block(block);
            } else {
                block_list.early_output_mut().pending_background_block = Some(block);
            }
        }

        match self.inner().pending_background_block.take() {
            Some(mut block) => {
                debug_assert!(
                    !block.started(),
                    "Started background blocks should be in the block list"
                );
                let retval = f(&mut block);
                store_pending_block(self.block_list, block);
                retval
            }
            _ => {
                if let Some(block) = self.block_list.background_block_mut() {
                    f(block)
                } else {
                    let mut block = self.block_list.create_pending_background_block();
                    let retval = f(&mut block);
                    store_pending_block(self.block_list, block);
                    retval
                }
            }
        }
    }
}

/// Delegate for `EarlyOutput` that will eventually delegate the method to the
/// background block/grid
macro_rules! delegate {
    ($self:ident.$method:ident( $( $arg:expr_2021 ),* )) => {
        $self.with_background_output(|block| {
            block.$method($( $arg ),*)
        })
    };
}

impl ansi::Handler for EarlyOutputHandler<'_> {
    fn input(&mut self, c: char) {
        if self.inner().consume_expected_echo(c) {
            return;
        }
        // A wrapped line's redraw clears the remainder of a display row with literal space
        // characters, not an erase-to-end-of-line escape, before moving to the next row (CUD;
        // see `move_down`) -- measured on real zsh sessions. Those spaces are still part of the
        // very redraw being matched, not real output, but they don't appear in the buffer text
        // itself, so they'd otherwise be treated as the first real mismatch and end the window
        // (see below), leaking everything from the wrap point on even though it does go on to
        // match. A space is therefore swallowed without ending the window whenever at least one
        // live candidate still expects more of the pattern -- i.e. this is deliberately
        // narrower than "the window is open at all": once every live candidate has reached the
        // pattern's own end, there is nothing left to redraw, so a stray space at that point is
        // far more likely to be unrelated output than wrap padding, and is left to the normal
        // mismatch handling below. This can still swallow a leading space of unrelated
        // background output that happens to start while a redraw is genuinely still in
        // progress, the same bounded, already-accepted trade-off as the single-character
        // exposure described below.
        if c == ' ' && self.inner().awaiting_more_expected_echo() {
            return;
        }
        // The first *real* character the pattern can't explain ends its window (see
        // `reset_expected_echo`'s doc comment for why this lives here and not inside
        // `consume_expected_echo` itself): no external signal reliably lands only after a
        // restore's own echo has fully arrived, so this is treated as proof the echo is over,
        // real or not. This bounds the damage from ending the window too early or too late to
        // a single leaked character rather than a registration surviving to corrupt arbitrary
        // later, unrelated output -- measured, unbounded: a carriage-return-driven progress
        // message (`\rloading 10%\rloading 20%...`) from an unrelated background job lost
        // several characters to a pattern left over from an earlier completions request.
        if !self.inner().expected_echo.is_empty() {
            self.inner().reset_expected_echo();
        }
        let session_id = self.block_list.active_block().session_id();
        if !self.inner().handle_potential_typeahead(c) {
            self.with_background_output(|block| {
                // We don't start background blocks until they have content because
                // the shell often prints control characters in between commands
                // to reset terminal state. If we eagerly added background blocks,
                // there would be an empty one before almost every command.
                if !block.started() {
                    block.start_background(session_id);
                }
                block.input(c);
            })
        }
    }

    /// Replace the current typeahead. We use this when we have complete typeahead
    /// information, such as when the shell reports its input buffer.
    fn input_buffer(&mut self, data: ansi::InputBufferValue) {
        if data.buffer.is_empty() {
            if warp_core::channel::ChannelState::channel()
                == warp_core::channel::Channel::Integration
            {
                log::info!("Ignoring empty input buffer");
            }
            // avoids a race condition when the user enters multiple lines of
            // typeahead. Suppose the user enters the following typeahead:
            // > cd foo <ENTER>
            // > pwd
            // When the running command finishes, we'll fetch `pwd` as typeahead
            // from the shell and then clear its input buffer. The shell will
            // immediately execute `cd foo`, which will start a new block. We'll
            // ask the shell for its input buffer again, but at this point, we've
            // already cleared it. If we overwrite our stored typeahead before
            // the terminal view has added it to the input buffer, it will be
            // lost.
            return;
        }

        let me = self.inner();
        if me.mode == TypeaheadMode::ShellReported {
            me.typeahead = data.buffer;
            if warp_core::channel::ChannelState::channel()
                == warp_core::channel::Channel::Integration
            {
                log::info!(
                    "Sending shell-reported typeahead event for {:?}",
                    me.typeahead
                );
            }
            me.event_proxy.send_terminal_event(TerminalEvent::Typeahead);
            safe_debug!(
                safe: ("Received shell input buffer for typeahead"),
                full: ("Received shell input buffer for typeahead: {:?}", me.typeahead)
            );
        }
    }

    fn carriage_return(&mut self) {
        if self.inner().consume_expected_echo('\r') {
            return;
        }
        // A carriage return means the cursor just returned to the start of the line -- if a
        // line editor is about to redraw (and therefore re-echo) the buffer this expected a
        // single echo of, the redraw's characters start arriving right after this. See
        // `rearm_at_column`.
        self.inner().rearm_at_column(0);
        if !self.inner().handle_potential_typeahead('\r') {
            delegate!(self.carriage_return());
        }
    }

    fn backspace(&mut self) {
        // A backspace is a rewind of exactly one column -- unlike a carriage return, whose
        // redraw might restart from any position, a redraw following a backspace can only
        // continue from one column earlier than wherever matching currently stands. See
        // `rearm_after_rewind`.
        self.inner().rearm_after_rewind(1);
        delegate!(self.backspace());
    }

    fn move_backward(&mut self, columns: usize) {
        // CUB: the same rewind as a backspace, but by a given column count rather than
        // always one. See `rearm_after_rewind`.
        self.inner().rearm_after_rewind(columns);
        delegate!(self.move_backward(columns));
    }

    fn move_forward(&mut self, columns: usize) {
        // CUF: the forward counterpart to move_backward/backspace. See
        // `advance_after_forward_move`.
        self.inner().advance_after_forward_move(columns);
        delegate!(self.move_forward(columns));
    }

    fn linefeed(&mut self) -> ScrollDelta {
        if self.inner().consume_expected_echo('\n') {
            return ScrollDelta::zero();
        }
        if self.inner().handle_potential_typeahead('\n') {
            // If we match a newline as typeahead, this means the shell will
            // execute the accumulated typeahead as a new command. In that case,
            // the shell doesn't re-echo the command, so we fill in the command
            // grid here.
            if self.inner().mode == TypeaheadMode::InputMatching {
                let command = mem::take(&mut self.inner().typeahead);
                safe_debug!(
                    safe: ("Initializing command grid from matched typeahead"),
                    full: ("Initializing command grid from matched typeahead: {command:?}")
                );
                self.block_list.active_block_mut().init_command(command);
                self.block_list.update_active_block_height();
            }

            ScrollDelta::zero()
        } else {
            let lines_scrolled = delegate!(self.linefeed());

            // SSH ControlMaster errors _should_ be categorized as background output.
            // To avoid checking on every character of background input, we only
            // match the most recent line after it is completed.
            if let Some(block) = self.block_list.background_block_mut() {
                let last_line = block
                    .output_grid()
                    .contents_to_string(false /* include_escape_sequences */, Some(1));
                if CONTROL_MASTER_ERROR_REGEX.is_match(&last_line) {
                    self.inner()
                        .event_proxy
                        .send_terminal_event(TerminalEvent::SSHControlMasterError);
                }
            }

            lines_scrolled
        }
    }

    /*
     * Handler methods which should not be reached.
     */

    fn set_title(&mut self, _: Option<String>) {
        log::warn!(
            "Handler method EarlyOutput::set_title should never be called. This should be handled by TerminalModel"
        );
    }

    fn push_title(&mut self) {
        log::warn!(
            "Handler method EarlyOutput::push_title should never be called. This should be handled by TerminalModel"
        );
    }

    fn pop_title(&mut self) {
        log::warn!(
            "Handler method EarlyOutput::pop_title should never be called. This should be handled by TerminalModel"
        );
    }

    fn precmd_with_completion_metadata(&mut self, _data: ansi::PrecmdValue) {
        panic!(
            "Called EarlyOutput::precmd_with_completion_metadata handler method instead of Block::precmd_with_completion_metadata"
        );
    }

    fn prompt_only_precmd(&mut self, _data: ansi::PromptMetadata) {
        panic!(
            "Called EarlyOutput::prompt_only_precmd handler method instead of Block::prompt_only_precmd"
        );
    }

    /*
     * Handler methods only relevant to background output.
     */

    fn set_cursor_style(&mut self, style: Option<ansi::CursorStyle>) {
        delegate!(self.set_cursor_style(style));
    }

    fn set_cursor_shape(&mut self, shape: ansi::CursorShape) {
        delegate!(self.set_cursor_shape(shape));
    }

    fn goto(&mut self, row: super::index::VisibleRow, col: usize) {
        // Absolute cursor addressing (CUP) is a rewind like carriage return, backspace and CUB,
        // just to an absolute rather than relative column -- see `rearm_at_column`. The row is
        // irrelevant to matching, which only tracks a linear character stream. Deliberately
        // conservative about which column to trust: unlike a carriage return, which always
        // means "column 0" by construction, an absolute column here could reflect a prompt's
        // width rather than the buffer's own start if the redraw re-renders both together, and
        // there is no way to tell from this byte alone which case applies. Column 0 is safe
        // either way (see `rearm_at_column`'s doc comment); anything else is treated as no
        // information rather than risked as a wrong position.
        if col == 0 {
            self.inner().rearm_at_column(col);
        }
        delegate!(self.goto(row, col));
    }

    fn goto_line(&mut self, row: super::index::VisibleRow) {
        delegate!(self.goto_line(row));
    }

    fn goto_col(&mut self, col: usize) {
        // CHA: the same absolute rewind as `goto`, just without a row component, and the same
        // deliberate conservatism about trusting only column 0. See `goto`.
        if col == 0 {
            self.inner().rearm_at_column(col);
        }
        delegate!(self.goto_col(col));
    }

    fn insert_blank(&mut self, count: usize) {
        delegate!(self.insert_blank(count));
    }

    fn move_up(&mut self, rows: usize) {
        delegate!(self.move_up(rows));
    }

    fn move_down(&mut self, rows: usize) {
        delegate!(self.move_down(rows));
    }

    fn identify_terminal<W: std::io::Write>(&mut self, writer: &mut W, intermediate: Option<char>) {
        delegate!(self.identify_terminal(writer, intermediate));
    }

    fn report_xtversion<W: std::io::Write>(&mut self, writer: &mut W) {
        delegate!(self.report_xtversion(writer));
    }

    fn device_status<W: std::io::Write>(&mut self, writer: &mut W, arg: usize) {
        delegate!(self.device_status(writer, arg));
    }

    fn move_down_and_cr(&mut self, rows: usize) {
        delegate!(self.move_down_and_cr(rows));
    }

    fn move_up_and_cr(&mut self, rows: usize) {
        delegate!(self.move_up_and_cr(rows));
    }

    fn put_tab(&mut self, count: u16) {
        delegate!(self.put_tab(count));
    }

    fn bell(&mut self) {
        delegate!(self.bell());
    }

    fn substitute(&mut self) {
        delegate!(self.substitute());
    }

    fn newline(&mut self) {
        delegate!(self.newline());
    }

    fn set_horizontal_tabstop(&mut self) {
        delegate!(self.set_horizontal_tabstop());
    }

    fn scroll_up(&mut self, rows: usize) -> ScrollDelta {
        delegate!(self.scroll_up(rows))
    }

    fn scroll_down(&mut self, rows: usize) -> ScrollDelta {
        delegate!(self.scroll_down(rows))
    }

    fn insert_blank_lines(&mut self, rows: usize) -> ScrollDelta {
        delegate!(self.insert_blank_lines(rows))
    }

    fn delete_lines(&mut self, rows: usize) -> ScrollDelta {
        delegate!(self.delete_lines(rows))
    }

    fn erase_chars(&mut self, count: usize) {
        delegate!(self.erase_chars(count));
    }

    fn delete_chars(&mut self, count: usize) {
        delegate!(self.delete_chars(count));
    }

    fn move_backward_tabs(&mut self, count: u16) {
        delegate!(self.move_backward_tabs(count));
    }

    fn move_forward_tabs(&mut self, count: u16) {
        delegate!(self.move_forward_tabs(count));
    }

    fn save_cursor_position(&mut self) {
        delegate!(self.save_cursor_position());
    }

    fn restore_cursor_position(&mut self) {
        delegate!(self.restore_cursor_position());
    }

    fn clear_line(&mut self, mode: ansi::LineClearMode) {
        delegate!(self.clear_line(mode));
    }

    fn clear_screen(&mut self, mode: ansi::ClearMode) {
        delegate!(self.clear_screen(mode));
    }

    fn clear_tabs(&mut self, mode: ansi::TabulationClearMode) {
        delegate!(self.clear_tabs(mode));
    }

    fn reset_state(&mut self) {
        delegate!(self.reset_state());
    }

    fn reverse_index(&mut self) -> ScrollDelta {
        delegate!(self.reverse_index())
    }

    fn terminal_attribute(&mut self, attribute: ansi::Attr) {
        delegate!(self.terminal_attribute(attribute));
    }

    fn set_mode(&mut self, mode: ansi::Mode) {
        delegate!(self.set_mode(mode));
    }

    fn unset_mode(&mut self, mode: ansi::Mode) {
        delegate!(self.unset_mode(mode));
    }

    fn set_scrolling_region(&mut self, top: usize, bottom: Option<usize>) {
        delegate!(self.set_scrolling_region(top, bottom));
    }

    fn set_keypad_application_mode(&mut self) {
        delegate!(self.set_keypad_application_mode());
    }

    fn unset_keypad_application_mode(&mut self) {
        delegate!(self.unset_keypad_application_mode());
    }

    fn set_active_charset(&mut self, index: ansi::CharsetIndex) {
        delegate!(self.set_active_charset(index));
    }

    fn configure_charset(&mut self, index: ansi::CharsetIndex, charset: ansi::StandardCharset) {
        delegate!(self.configure_charset(index, charset));
    }

    fn set_color(&mut self, index: usize, color: ColorU) {
        delegate!(self.set_color(index, color));
    }

    fn dynamic_color_sequence<W: std::io::Write>(
        &mut self,
        writer: &mut W,
        code: u8,
        index: usize,
        terminator: &str,
    ) {
        delegate!(self.dynamic_color_sequence(writer, code, index, terminator));
    }

    fn reset_color(&mut self, index: usize) {
        delegate!(self.reset_color(index));
    }

    fn clipboard_store(&mut self, clipboard: u8, data: &[u8]) {
        delegate!(self.clipboard_store(clipboard, data));
    }

    fn clipboard_load(&mut self, clipboard: u8, terminator: &str) {
        delegate!(self.clipboard_load(clipboard, terminator));
    }

    fn decaln(&mut self) {
        delegate!(self.decaln());
    }

    fn text_area_size_pixels<W: std::io::Write>(&mut self, writer: &mut W) {
        delegate!(self.text_area_size_pixels(writer));
    }

    fn text_area_size_chars<W: std::io::Write>(&mut self, writer: &mut W) {
        delegate!(self.text_area_size_chars(writer));
    }

    fn on_finish_byte_processing(&mut self, input: &ansi::ProcessorInput<'_>) {
        delegate!(self.on_finish_byte_processing(input));
    }

    fn prompt_marker(&mut self, _marker: ansi::PromptMarker) {
        report_error!(
            "Received prompt_marker in EarlyOutput, but it should be sent to the active block by the blocklist"
        );
    }

    fn set_keyboard_enhancement_flags(
        &mut self,
        mode: KeyboardModes,
        apply: KeyboardModesApplyBehavior,
    ) {
        delegate!(self.set_keyboard_enhancement_flags(mode, apply));
    }

    fn push_keyboard_enhancement_flags(&mut self, mode: KeyboardModes) {
        delegate!(self.push_keyboard_enhancement_flags(mode));
    }

    fn pop_keyboard_enhancement_flags(&mut self, count: u16) {
        delegate!(self.pop_keyboard_enhancement_flags(count));
    }

    fn query_keyboard_enhancement_flags<W: std::io::Write>(&mut self, writer: &mut W) {
        delegate!(self.query_keyboard_enhancement_flags(writer));
    }

    fn handle_completed_iterm_image(&mut self, image: ITermImage) {
        let session_id = self.block_list.active_block().session_id();
        self.with_background_output(|block| {
            let had_visible_content = block.output_grid().has_visible_content();
            block.handle_completed_iterm_image(image);
            if !had_visible_content && block.output_grid().has_visible_content() && !block.started()
            {
                block.start_background(session_id);
            }
        });
    }

    fn handle_completed_kitty_action(
        &mut self,
        action: KittyAction,
        metadata: &mut HashMap<u32, StoredImageMetadata>,
    ) -> Option<KittyResponse> {
        let session_id = self.block_list.active_block().session_id();
        self.with_background_output(|block| {
            let had_visible_content = block.output_grid().has_visible_content();
            let retval = block.handle_completed_kitty_action(action, metadata);
            if !had_visible_content && block.output_grid().has_visible_content() && !block.started()
            {
                block.start_background(session_id);
            }
            retval
        })
    }
}
