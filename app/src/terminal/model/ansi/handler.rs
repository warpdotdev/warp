use std::collections::HashMap;
use std::io;

use warp_terminal::model::ansi::control_sequence_parameters::*;
use warp_terminal::model::{KeyboardModes, KeyboardModesApplyBehavior};
use warpui::color::ColorU;

use super::dcs_hooks::*;
use super::ProcessorInput;
use crate::terminal::model::completions::{ShellCompletion, ShellCompletionUpdate};
use crate::terminal::model::image_map::StoredImageMetadata;
use crate::terminal::model::iterm_image::{ITermImage, ITermImageMetadata};
use crate::terminal::model::kitty::{KittyAction, KittyChunk, KittyResponse};
use crate::terminal::model::terminal_model::TmuxInstallationState;
use crate::terminal::model::{
    completions::ShellData as CompletionsShellData, index::VisibleRow, selection::ScrollDelta,
    tmux::ControlModeEvent,
};

/// Trait to be implemented by model objects that handle pty output. The
/// ansi::Performer (our pty output parser) delegates handling of specific
/// actions (e.g. `set_title()`, `input()`) to a struct that implements this
/// trait.
///
/// Default implementations are provided for some methods to reduce the amount
/// of necessary boilerplate required for a testing-only implementation.
pub trait Handler {
    /// OSC to set window title.
    fn set_title(&mut self, _: Option<String>);

    /// Set the cursor style.
    fn set_cursor_style(&mut self, _: Option<CursorStyle>);

    /// Set the cursor shape.
    fn set_cursor_shape(&mut self, _shape: CursorShape);

    /// A character to be displayed.
    fn input(&mut self, _c: char);

    /// Set cursor to position.
    fn goto(&mut self, _: VisibleRow, _: usize);

    /// Set cursor to specific row.
    fn goto_line(&mut self, _: VisibleRow);

    /// Set cursor to specific column.
    fn goto_col(&mut self, _: usize);

    /// Insert blank characters in current line starting from cursor.
    fn insert_blank(&mut self, _: usize);

    /// Move cursor up `rows`.
    fn move_up(&mut self, _: usize);

    /// Move cursor down `rows`.
    fn move_down(&mut self, _: usize);

    /// Identify the terminal (should write back to the pty stream).
    ///
    /// TODO this should probably return an io::Result
    fn identify_terminal<W: io::Write>(&mut self, _: &mut W, _intermediate: Option<char>);

    /// Report XTVERSION to uniquely identify terminal name and version (should write back to the pty stream).
    fn report_xtversion<W: io::Write>(&mut self, _: &mut W);

    /// Report device status.
    fn device_status<W: io::Write>(&mut self, _: &mut W, _: usize);

    /// Move cursor forward `cols`.
    fn move_forward(&mut self, _: usize);

    /// Move cursor backward `cols`.
    fn move_backward(&mut self, _: usize);

    /// Move cursor down `rows` and set to column 1.
    fn move_down_and_cr(&mut self, _: usize);

    /// Move cursor up `rows` and set to column 1.
    fn move_up_and_cr(&mut self, _: usize);

    /// Put `count` tabs.
    fn put_tab(&mut self, _count: u16);

    /// Backspace `count` characters.
    fn backspace(&mut self);

    /// Carriage return.
    fn carriage_return(&mut self);

    /// Line feed.
    fn linefeed(&mut self) -> ScrollDelta;

    /// Ring the bell.
    ///
    /// Hopefully this is never implemented.
    fn bell(&mut self);

    /// Substitute char under cursor.
    fn substitute(&mut self);

    /// Newline.
    fn newline(&mut self);

    /// Set current position as a tabstop.
    fn set_horizontal_tabstop(&mut self);

    /// Scroll up `rows` rows.
    fn scroll_up(&mut self, _: usize) -> ScrollDelta;

    /// Scroll down `rows` rows.
    fn scroll_down(&mut self, _: usize) -> ScrollDelta;

    /// Insert `count` blank lines.
    fn insert_blank_lines(&mut self, _: usize) -> ScrollDelta;

    /// Delete `count` lines.
    fn delete_lines(&mut self, _: usize) -> ScrollDelta;

    /// Erase `count` chars in current line following cursor.
    ///
    /// Erase means resetting to the default state (default colors, no content,
    /// no mode flags).
    fn erase_chars(&mut self, _: usize);

    /// Delete `count` chars.
    ///
    /// Deleting a character is like the delete key on the keyboard - everything
    /// to the right of the deleted things is shifted left.
    fn delete_chars(&mut self, _: usize);

    /// Move backward `count` tabs.
    fn move_backward_tabs(&mut self, _count: u16);

    /// Move forward `count` tabs.
    fn move_forward_tabs(&mut self, _count: u16);

    /// Save current cursor position.
    fn save_cursor_position(&mut self);

    /// Restore cursor position.
    fn restore_cursor_position(&mut self);

    /// Clear current line.
    fn clear_line(&mut self, _mode: LineClearMode);

    /// Clear screen.
    fn clear_screen(&mut self, _mode: ClearMode);

    /// Clear tab stops.
    fn clear_tabs(&mut self, _mode: TabulationClearMode);

    /// Reset terminal state.
    fn reset_state(&mut self);

    /// Reverse Index.
    ///
    /// Move the active position to the same horizontal position on the
    /// preceding line. If the active position is at the top margin, a scroll
    /// down is performed.
    fn reverse_index(&mut self) -> ScrollDelta;

    /// Set a terminal attribute.
    fn terminal_attribute(&mut self, _attr: Attr);

    /// Set mode.
    fn set_mode(&mut self, _mode: Mode);

    /// Unset mode.
    fn unset_mode(&mut self, _: Mode);

    /// Set keyboard enhancement flags (Kitty keyboard protocol: CSI = flags u).
    fn set_keyboard_enhancement_flags(
        &mut self,
        mode: KeyboardModes,
        apply: KeyboardModesApplyBehavior,
    );

    /// Push keyboard enhancement flags (Kitty keyboard protocol: CSI > flags u).
    fn push_keyboard_enhancement_flags(&mut self, mode: KeyboardModes);

    /// Pop keyboard enhancement flags (Kitty keyboard protocol: CSI < flags u).
    fn pop_keyboard_enhancement_flags(&mut self, count: u16);

    /// Query keyboard enhancement flags (Kitty keyboard protocol: CSI ? u).
    /// Should respond with CSI ? flags u where flags is the currently enabled flags.
    fn query_keyboard_enhancement_flags<W: io::Write>(&mut self, writer: &mut W);

    /// DECSTBM - Set the terminal scrolling region.
    fn set_scrolling_region(&mut self, _top: usize, _bottom: Option<usize>);

    /// DECKPAM - Set keypad to applications mode (ESCape instead of digits).
    fn set_keypad_application_mode(&mut self);

    /// DECKPNM - Set keypad to numeric mode (digits instead of ESCape seq).
    fn unset_keypad_application_mode(&mut self);

    /// Set one of the graphic character sets, G0 to G3, as the active charset.
    ///
    /// 'Invoke' one of G0 to G3 in the GL area. Also referred to as shift in,
    /// shift out and locking shift depending on the set being activated.
    fn set_active_charset(&mut self, _: CharsetIndex);

    /// Assign a graphic character set to G0, G1, G2 or G3.
    ///
    /// 'Designate' a graphic character set as one of G0 to G3, so that it can
    /// later be 'invoked' by `set_active_charset`.
    fn configure_charset(&mut self, _: CharsetIndex, _: StandardCharset);

    /// Set an indexed color value.
    fn set_color(&mut self, _: usize, _: ColorU);

    /// Write a foreground/background color escape sequence with the current color.
    fn dynamic_color_sequence<W: io::Write>(&mut self, _: &mut W, _: u8, _: usize, _: &str);

    /// Reset an indexed color to original value.
    fn reset_color(&mut self, _: usize);

    /// Store data into clipboard.
    fn clipboard_store(&mut self, _: u8, _: &[u8]);

    /// Load data from clipboard.
    fn clipboard_load(&mut self, _: u8, _: &str);

    /// Run the decaln routine.
    fn decaln(&mut self);

    /// Push a title onto the stack.
    fn push_title(&mut self);

    /// Pop the last title from the stack.
    fn pop_title(&mut self);

    /// Report text area size in pixels.
    fn text_area_size_pixels<W: io::Write>(&mut self, _: &mut W);

    /// Report text area size in characters.
    fn text_area_size_chars<W: io::Write>(&mut self, _: &mut W);

    /// Callback for the Warp CommandFinished hook.
    fn command_finished(&mut self, _data: CommandFinishedValue) {}

    /// Process a prompt marker control sequence.
    fn prompt_marker(&mut self, _marker: PromptMarker) {}

    /// Callback for the Warp precmd hook.
    fn precmd(&mut self, _data: PrecmdValue) {}

    /// Update the active block's current working directory, independent of the
    /// prompt cycle. Invoked from OSC 7 (`\e]7;file://host/path`) so external
    /// tools can notify the terminal of CWD changes mid-command, without
    /// waiting for the shell to redraw its prompt.
    fn set_current_working_directory(&mut self, _path: String) {}

    /// Callback for the Warp preexec hook.
    fn preexec(&mut self, _data: PreexecValue) {}

    /// Callback for the Warp bootstrapped hook - called once when the shell is
    /// bootstrapped
    fn bootstrapped(&mut self, _data: BootstrappedValue) {}

    /// Callback for the Warp pre-interactive SSH session hook - called once
    /// before initiating an interactive SSH session (either with or without the
    /// SSH wrapper).
    fn pre_interactive_ssh_session(&mut self, _data: PreInteractiveSSHSessionValue) {}

    /// Callback for the Warp ssh hook - called once after successfully connecting to
    /// an SSH server
    fn ssh(&mut self, _data: SSHValue) {}

    /// Callback for the terminal to initialize the shell by writing the bootstrap
    /// logic into the PTY
    fn init_shell(&mut self, _data: InitShellValue) {}

    /// Callback for the Warp exit-shell hook — emitted by the remote shell right
    /// before it exits. Gives the Warp client a chance to drop per-session
    /// resources (e.g. the `ssh … remote-server-proxy` child) before the outer
    /// ssh tunnel starts tearing down, so its ControlMaster can exit cleanly
    /// rather than hanging on orphaned multiplexed channels.
    fn exit_shell(&mut self, _data: ExitShellValue) {}

    /// Callback for the terminal to when user executes `clear` command.
    fn clear(&mut self, _data: ClearValue) {}

    /// Callback for the terminal when the shell reports the current line editor
    /// input buffer (the reporting is itself triggered by Warp).
    fn input_buffer(&mut self, _data: InputBufferValue) {}

    /// Callback emitted during the initialization process for subshells with where the shell type
    /// is initiall not known.
    fn init_subshell(&mut self, _data: InitSubshellValue) {}

    /// Callback emitted when executing the user's RC file, which signals a new session is being
    /// created. If the session is for a subshell, this should triggers Warp's bootstrap process.
    /// Otherwise, it's ignored.
    fn sourced_rc_file(&mut self, _data: SourcedRcFileForWarpValue) {}

    /// Callback emitted during the initialization process for ssh sessions
    fn init_ssh(&mut self, _data: InitSshValue) {}

    /// Callback emitted to notify the app that we're ready to complete an
    /// assisted auto-update.
    fn finish_update(&mut self, _data: FinishUpdateValue) {}

    /// Callback emitted from the warpify_ssh_session script if it's discovered
    /// that we can't warpify the remote session.
    fn remote_warpification_is_unavailable(&mut self, _data: WarpificationUnavailableReason) {}

    /// How tmux was installed.
    fn notify_ssh_tmux_is_installed(&mut self, _tmux_installation: TmuxInstallationState) {}

    fn tmux_install_failed(&mut self, _data: TmuxInstallFailedInfo) {}

    /// Callback to handle an "in-band command output start" OSC.
    ///
    /// Chars received via `handler::input()` represent the in-band command output itself.
    /// Subsequent non-printable chars (e.g. control sequences) should be handled normally.
    fn start_in_band_command_output(&mut self) {}

    /// Callback to handle an "in-band command output end" OSC.
    ///
    /// Marks the end of the in-band command output payload.
    fn end_in_band_command_output(&mut self, _from_osc_sequence: bool) {}

    /// Hook that gets called upon processing a chunk of input from the PTY.
    /// Implementors can use this to perform any extra, one-off, logic with the
    /// input after it's been parsed.
    fn on_finish_byte_processing(&mut self, _input: &ProcessorInput<'_>) {}

    /// Hook that gets called upon receiving a "Reset Grid" OSC from ConPTY.
    fn on_reset_grid(&mut self) {}

    /// tmux control mode event
    fn tmux_control_mode_event(&mut self, _event: ControlModeEvent) {}

    /// Callback that tells the terminal that the shell is ready to receive
    /// the string to run completions for.
    fn send_completions_prompt(&mut self) {}

    /// Callback to handle the OSC for starting completions.
    ///
    /// Depending on the output format, subsequent data from the PTY will be
    /// considered as completions output.
    fn start_completions_output(&mut self, _format: CompletionsShellData) {}

    /// Callback to handle the OSC for finishing completions.
    ///
    /// Marks the end of the in-band command output payload.
    fn end_completions_output(&mut self) {}

    /// Callback invoked when we've received a _typed_ native completion result from the shell.
    /// This is a noop if we are in "raw" completions mode.
    fn on_completion_result_received(&mut self, _completion_result: ShellCompletion) {}

    /// Update the last completion result with the metadata in [`ShellCompletionUpdate`].
    /// This is a noop if we are in "raw" completions mode.
    fn update_last_completion_result(&mut self, _completion_update: ShellCompletionUpdate) {}

    /// Callback to handle the OSC to start receiving an iTerm image.
    /// This will either be a MultipartFile or File (legacy).
    /// This will take the metadata given by the first message.
    fn start_iterm_image_receiving(&mut self, _metadata: ITermImageMetadata) {}

    /// Callback to handle the OSC to finish receiving an iTerm image.
    fn end_iterm_image_receiving(&mut self) {}

    /// Callback to handle the OSC to receive a chunk of an iTerm image.
    fn on_iterm_image_data_received(&mut self, _image_data: &[u8]) {}

    /// Callback to handle the fully transmitted iTerm image on a grid handler level.
    /// Returns whether or not the image was saved to memory.
    fn handle_completed_iterm_image(&mut self, _image: ITermImage) {}

    /// Callback that tells the terminal to prepare for receiving the a shell hook via
    /// key-value pairs.
    fn start_receiving_hook(&mut self, _hook_name: String) {}

    /// Callback that tells the terminal that the pending shell hook is done receiving key-value
    /// pairs.
    ///
    /// Returns the pending shell hook.
    fn finish_receiving_hook(&mut self) -> Option<PendingHook> {
        None
    }

    // Callback that tells the terminal to update the pending shell hook with a new key-value pair.
    fn update_hook(&mut self, _key: String, _value: String) {}

    /// Callback to handle the APC to finish receiving a kitty action.
    fn end_kitty_action_receiving<W: io::Write>(&mut self, _writer: &mut W) {}

    /// Callback to handle the APC to receive a chunk of a kitty action.
    fn on_kitty_image_chunk_received(&mut self, _chunk: KittyChunk) {}

    /// Callback to handle the fully transmitted Kitty action on a grid handler level.
    fn handle_completed_kitty_action(
        &mut self,
        _action: KittyAction,
        _metadata: &mut HashMap<u32, StoredImageMetadata>,
    ) -> Option<KittyResponse> {
        None
    }

    /// Callback for pluggable notifications triggered via OSC 9 or OSC 777 escape sequences.
    /// These allow external programs to trigger notifications in Warp.
    /// - OSC 9: Simple notification with just a body (iTerm2/Windows Terminal style)
    /// - OSC 777: Notification with title and body (urxvt style)
    fn pluggable_notification(&mut self, _title: Option<String>, _body: String) {}
}
