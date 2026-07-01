use std::collections::{HashMap, VecDeque};
use std::mem;

use pathfinder_color::ColorU;
use string_offset::CharOffset;
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

    /// Reset the unmatched user input. This is called between blocks so that
    /// unmatched potential typeahead from one command doesn't throw off input
    /// matching for the rest of the session.
    pub fn reset_user_input(&mut self) {
        self.unmatched_input.clear();
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
            if !block_list.active_block().started() {
                if let Some(background_block) = block_list.remove_background_block() {
                    log::debug!("Repairing command from background block");
                    block_list
                        .active_block_mut()
                        .copy_command_grid(background_block.output_grid());
                    block_list.update_active_block_height();
                }
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

        if let Some(mut block) = self.inner().pending_background_block.take() {
            debug_assert!(
                !block.started(),
                "Started background blocks should be in the block list"
            );
            let retval = f(&mut block);
            store_pending_block(self.block_list, block);
            retval
        } else if let Some(block) = self.block_list.background_block_mut() {
            f(block)
        } else {
            let mut block = self.block_list.create_pending_background_block();
            let retval = f(&mut block);
            store_pending_block(self.block_list, block);
            retval
        }
    }
}

/// Delegate for `EarlyOutput` that will eventually delegate the method to the
/// background block/grid
macro_rules! delegate {
    ($self:ident.$method:ident( $( $arg:expr ),* )) => {
        $self.with_background_output(|block| {
            block.$method($( $arg ),*)
        })
    };
}

impl ansi::Handler for EarlyOutputHandler<'_> {
    fn input(&mut self, c: char) {
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
        if !self.inner().handle_potential_typeahead('\r') {
            delegate!(self.carriage_return());
        }
    }

    fn linefeed(&mut self) -> ScrollDelta {
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
        delegate!(self.goto(row, col));
    }

    fn goto_line(&mut self, row: super::index::VisibleRow) {
        delegate!(self.goto_line(row));
    }

    fn goto_col(&mut self, col: usize) {
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

    fn move_forward(&mut self, columns: usize) {
        delegate!(self.move_forward(columns));
    }

    fn move_backward(&mut self, columns: usize) {
        delegate!(self.move_backward(columns));
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

    fn backspace(&mut self) {
        delegate!(self.backspace());
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
        log::error!(
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
