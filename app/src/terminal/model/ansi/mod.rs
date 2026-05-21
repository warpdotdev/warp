//! This module implements processing logic for pty output.
//!
//! The top-level export is [`Processor`], which is a lightweight wrapper
//! around Alacritty's [`VteParser`] that delegates handling of the parsed PTY
//! output to [`Performer`], which is our implementation of [`VtePerform`].
//!
//! Internally, [`Performer`] delegates to finer-grained methods for handling
//! PTY output implemented by the [`Handler`] trait -- this could be printing to
//! the terminal, executing actions as a result of CSI or OSC sequences,
//! executing one of Warp's DCS hooks, etc. [`Handler`] should be implemented by
//! an app-level model that updates the terminal's state accordingly.
mod ansi_c_decoder;
mod dcs_hooks;
mod handler;

use ansi_c_decoder::*;
pub use dcs_hooks::*;
pub use handler::*;
use instant::Instant;
use itertools::Itertools;
pub use warp_terminal::model::ansi::control_sequence_parameters::*;
use warp_terminal::model::{KeyboardModes, KeyboardModesApplyBehavior};

use crate::features::FeatureFlag;
use crate::terminal::model::completions::{
    ShellCompletion, ShellCompletionUpdate, ShellData as CompletionsShellData,
};
use crate::terminal::model::escape_sequences::C0;
use crate::terminal::model::index::VisibleRow;

use crate::terminal::model::iterm_image::parse_iterm_image_metadata;

use crate::terminal::model::tmux::{
    commands::{parse_command, TmuxCommandResponse},
    format_input,
    parser::{TmuxControlModeHandler, TmuxControlModeParser, TmuxMessage},
};

use crate::terminal::model::tmux::ControlModeEvent;
use crate::{safe_debug, safe_error};
use byte_unit::{Byte, Unit as ByteUnit};
use hex;
use lazy_static::lazy_static;
use log::debug;
use std::collections::HashMap;
use std::fmt::Write;

use std::str::FromStr as _;
use std::time::Duration;
use std::{io, str};
use vte::{Params, Parser as VteParser, Perform as VtePerform};
use warpui::color::ColorU;

use super::kitty::parse_kitty_chunk;
use super::terminal_model::TmuxInstallationState;

/// Marks an OSC as one that is sent by Warp logic registered in the shell.
///
/// 9277 spells out "WARP" on a dialpad :).
const WARP_IN_BAND_GENERATOR_OSC_MARKER: &[u8] = b"9277";
const WARP_IN_BAND_GENERATOR_START_BYTE: &[u8] = b"A";
const WARP_IN_BAND_GENERATOR_END_BYTE: &[u8] = b"B";

/// Marks an OSC that is used for messages containing shell hooks.
const WARP_OSC_MARKER: &[u8] = b"9278";
/// Marks an OSC that is used for resetting ConPTY's grid. This is useful for performing a series
/// of checks ensuring that Warp's grids and ConPTY's grid are in sync.
const WARP_RESET_GRID_OSC_MARKER: &[u8] = b"9279";

/// The amount of time a single synchronized update can take from the time the corresponding
/// 'Set Mode' escape sequence is processed before a redraw is forced.
///
/// Note that this can be reset during a synchronized output update if another
/// 'Set Mode' escape sequence is processed.
const SYNC_OUTPUT_MAX_TIMEOUT: Duration = Duration::from_millis(150);

lazy_static! {
    /// The maximum number of bytes that can be processed in a single synchronized update.
    /// This means that synchronized updates can only be as large before a redraw is forced.
    ///
    /// Defined as a lazy static to get around the fact that `unwrap` is not a const fn.
    static ref SYNC_OUTPUT_MAX_BUFFER_SIZE: Byte = Byte::from_u64_with_unit(2, ByteUnit::MiB).expect("Can create byte size for sync output max buffer size");
}

const WARP_COMPLETIONS_OSC_MARKER: &[u8] = b"9280";
const WARP_COMPLETIONS_START_BYTE: &[u8] = b"A";
const WARP_COMPLETIONS_END_BYTE: &[u8] = b"B";
const WARP_COMPLETIONS_MATCH_RESULT_BYTE: &[u8] = b"C";

/// Denotes an OSC that sends metadata about the last match result.
/// The sequence begins with `D?` followed by the field that should be updated.
/// For example: `D?description'{OSC_PAYLOAD}` updates the description of the last match.
const WARP_COMPLETIONS_MATCH_UPDATE_METADATA: &[u8] = b"D?";

/// Marks an OSC that tells the terminal that the shell is ready to receive
/// the the string to run completions for.
const WARP_COMPLETIONS_PROMPT_BYTE: &[u8] = b"P";

const WARP_KV_START_BYTE: &[u8] = b"A";
const WARP_KV_ENTRY_BYTE: &[u8] = b"B";
const WARP_KV_END_BYTE: &[u8] = b"C";

/// Parse colors in XParseColor format.
#[allow(dead_code)]
fn xparse_color(color: &[u8]) -> Option<ColorU> {
    if !color.is_empty() && color[0] == b'#' {
        parse_legacy_color(&color[1..])
    } else if color.len() >= 4 && &color[..4] == b"rgb:" {
        parse_rgb_color(&color[4..])
    } else {
        None
    }
}

/// Parse colors in `rgb:r(rrr)/g(ggg)/b(bbb)` format.
fn parse_rgb_color(color: &[u8]) -> Option<ColorU> {
    let colors = str::from_utf8(color).ok()?.split('/').collect::<Vec<_>>();

    if colors.len() != 3 {
        return None;
    }

    // Scale values instead of filling with `0`s.
    let scale = |input: &str| {
        if input.len() > 4 {
            None
        } else {
            let max = u32::pow(16, input.len() as u32) - 1;
            let value = u32::from_str_radix(input, 16).ok()?;
            Some((255 * value / max) as u8)
        }
    };

    Some(ColorU::new(
        scale(colors[0])?,
        scale(colors[1])?,
        scale(colors[2])?,
        0xff,
    ))
}

/// Parse colors in `#r(rrr)g(ggg)b(bbb)` format.
fn parse_legacy_color(color: &[u8]) -> Option<ColorU> {
    let item_len = color.len() / 3;

    // Truncate/Fill to two byte precision.
    let color_from_slice = |slice: &[u8]| {
        let col = usize::from_str_radix(str::from_utf8(slice).ok()?, 16).ok()? << 4;
        Some((col >> (4 * slice.len().saturating_sub(1))) as u8)
    };

    Some(ColorU::new(
        color_from_slice(&color[0..item_len])?,
        color_from_slice(&color[item_len..item_len * 2])?,
        color_from_slice(&color[item_len * 2..])?,
        0xff,
    ))
}

fn parse_number(input: &[u8]) -> Option<u8> {
    if input.is_empty() {
        return None;
    }
    let mut num: u8 = 0;
    for c in input {
        let c = *c as char;
        if let Some(digit) = c.to_digit(10) {
            num = num
                .checked_mul(10)
                .and_then(|v| v.checked_add(digit as u8))?
        } else {
            return None;
        }
    }
    Some(num)
}

#[derive(Debug, Default)]
struct DcsData {
    data: Vec<u8>,
    intermediates: Vec<u8>,
    final_char: char,
}

impl DcsData {
    pub fn push(&mut self, byte: u8) {
        self.data.push(byte);
    }

    pub fn on_hook(&mut self, intermediates: &[u8], final_char: char) {
        self.intermediates = Vec::from(intermediates);
        self.final_char = final_char;
        self.data.clear();
    }
}

struct TmuxControlMode {
    control_mode_parser: TmuxControlModeParser,
    control_mode_state: TmuxControlModeState,
}

impl TmuxControlMode {
    fn new() -> Self {
        TmuxControlMode {
            control_mode_parser: TmuxControlModeParser::new(),
            control_mode_state: TmuxControlModeState {
                ansi_processor: Box::new(Processor::new()),
                pane_for_window: Default::default(),
                primary_pane: PrimaryPaneState::new(),
            },
        }
    }
}

struct TmuxControlModeState {
    ansi_processor: Box<Processor>,
    pane_for_window: HashMap<u32, u32>,
    primary_pane: PrimaryPaneState,
}

/// State to keep track of Synchronized Output.
///
/// Synchronized output is a protocol that allows terminal applications
/// to issue multiple updates to the state of the PTY without causing a redraw
/// between each update.
///
/// There are two mechanisms to prevent Warp from falling too behind:
/// 1. a timeout. After [`SYNC_OUTPUT_MAX_TIMEOUT`] has elapsed, a redraw will be forced.
/// 2. a max buffer limit. After [`SYNC_OUTPUT_MAX_BUFFER_SIZE`] bytes have been buffered,
///    a redraw will be forced.
///
/// See https://gist.github.com/christianparpart/d8a62cc1ab659194337d73e399004036
/// for the exact spec.
#[derive(Clone, Debug)]
enum SyncOutputState {
    /// Synchronized output is disabled.
    Inactive,
    /// There is a synchronized update in progress.
    Active {
        /// Bytes read during the synchronized update.
        /// These must be processed when the update is finished.
        buffer: Vec<u8>,

        /// The last time we got a 'Set Mode' escape sequence
        /// for synchronized output. It's possible to receive
        /// one while synchronized output is already active;
        /// in that case, we should extend the timeout.
        last_activated_at: Instant,
    },
}

impl SyncOutputState {
    fn is_active(&self) -> bool {
        matches!(self, Self::Active { .. })
    }

    fn activate(&mut self) {
        match self {
            Self::Inactive => {
                *self = Self::Active {
                    buffer: Vec::with_capacity(SYNC_OUTPUT_MAX_BUFFER_SIZE.as_u64() as usize),
                    last_activated_at: Instant::now(),
                }
            }
            Self::Active {
                last_activated_at, ..
            } => {
                *last_activated_at = Instant::now();
            }
        }
    }

    fn deactivate(&mut self) -> Option<Vec<u8>> {
        let old = std::mem::replace(self, Self::Inactive);
        match old {
            Self::Inactive => None,
            Self::Active { buffer, .. } => Some(buffer),
        }
    }

    fn remaining_timeout(&self) -> Option<Duration> {
        match self {
            Self::Inactive => None,
            Self::Active {
                last_activated_at, ..
            } => {
                let end = *last_activated_at + SYNC_OUTPUT_MAX_TIMEOUT;
                Some(end.saturating_duration_since(Instant::now()))
            }
        }
    }

    fn buffer_mut(&mut self) -> Option<&mut Vec<u8>> {
        match self {
            Self::Inactive => None,
            Self::Active { buffer, .. } => Some(buffer),
        }
    }

    fn buffer_len(&self) -> Option<usize> {
        match self {
            Self::Inactive => None,
            Self::Active { buffer, .. } => Some(buffer.len()),
        }
    }
}

/// Input to the ANSI processor.
#[derive(Copy, Clone)]
pub struct ProcessorInput<'a> {
    bytes: &'a [u8],

    /// Whether these bytes originated from a synchronized output frame flush.
    /// With synchronized output, [`Handler::on_finish_byte_processing`] may be called twice:
    /// * Once when the synchronized output frame is flushed
    /// * Once when the processor finishes handling the original raw bytes
    is_synchronized_output_frame: bool,
}

impl<'a> ProcessorInput<'a> {
    /// Creates a new `ProcessorInput` given a block of bytes.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            is_synchronized_output_frame: false,
        }
    }

    pub fn bytes(&self) -> &[u8] {
        self.bytes
    }

    pub fn is_synchronized_output_frame(&self) -> bool {
        self.is_synchronized_output_frame
    }
}

/// Internal state for VTE processor.
struct ProcessorState {
    preceding_char: Option<char>,
    dcs_data: DcsData,
    apc_data: Vec<u8>,
    tmux_control_mode: Option<TmuxControlMode>, // Present if control mode is active
    sync_output: SyncOutputState,
}

/// The processor wraps a [`VteParser`] to ultimately call methods on a Handler.
pub struct Processor {
    state: ProcessorState,
    parser: VteParser,
}

impl Default for Processor {
    fn default() -> Processor {
        Processor {
            state: ProcessorState {
                preceding_char: None,
                dcs_data: DcsData::default(),
                apc_data: vec![],
                tmux_control_mode: None,
                sync_output: SyncOutputState::Inactive,
            },
            parser: VteParser::new(),
        }
    }
}

impl Processor {
    pub fn new() -> Processor {
        Default::default()
    }

    /// Advance the parser by a series of bytes. If the addition of the bytes trigger any actions,
    /// such as changing the text color or inputting a character, the requisite function is called
    /// on the `handler`.
    #[inline]
    pub fn parse_bytes<H, W>(&mut self, handler: &mut H, bytes: &[u8], writer: &mut W)
    where
        H: Handler,
        W: io::Write,
    {
        self.parse_bytes_internal(
            handler,
            ProcessorInput {
                bytes,
                is_synchronized_output_frame: false,
            },
            writer,
        );
    }

    /// Internal implementation of [`Self::parse_bytes`] that accepts a full [`ProcessorInput`].
    fn parse_bytes_internal<H, W>(&mut self, handler: &mut H, input: ProcessorInput, writer: &mut W)
    where
        H: Handler,
        W: io::Write,
    {
        let mut bytes = input.bytes;

        // Bytes are parsed fundamentally differently if they're coming from a connection to tmux
        // control mode vs a standard pty. Though quite uncommon, tmux control mode can potentially
        // be entered and exited multiple times in a batch of bytes. This loop is so we can
        // continue to process bytes until we've processed through all tmux control mode state
        // changes.
        while !bytes.is_empty() {
            match &mut self.state.tmux_control_mode {
                None => {
                    let mut remaining_bytes_index = bytes.len();
                    for (idx, byte) in bytes.iter().enumerate() {
                        let was_sync_output = self.state.sync_output.is_active();
                        self.parse_byte(handler, writer, *byte);
                        let is_sync_output = self.state.sync_output.is_active();

                        if self.state.tmux_control_mode.is_some()
                            || was_sync_output != is_sync_output
                        {
                            // We split up the batch processing in two cases:
                            // 1. Tmux control mode started. Remaining bytes must be processed in a
                            //    control mode context.
                            // 2. Synchronized output was toggled. The pre- and post-toggle bytes should be
                            //    handled in separate `on_finish_byte_processing` calls.
                            remaining_bytes_index = idx + 1;
                            break;
                        }
                    }
                    handler.on_finish_byte_processing(&ProcessorInput {
                        bytes: &bytes[..remaining_bytes_index],
                        ..input
                    });

                    if self.state.tmux_control_mode.is_some() {
                        // Tmux control mode has just started -- notify the handler.
                        handler.tmux_control_mode_event(ControlModeEvent::Starting);
                    }

                    bytes = &bytes[remaining_bytes_index..];
                }

                Some(tmux_control_mode) => {
                    let mut tmux_performer = TmuxPerformer::new(
                        &mut tmux_control_mode.control_mode_state,
                        handler,
                        writer,
                    );
                    let mut remaining_bytes_index = bytes.len();
                    for (idx, byte) in bytes.iter().enumerate() {
                        tmux_control_mode
                            .control_mode_parser
                            .advance(&mut tmux_performer, *byte);

                        if tmux_performer.exited {
                            remaining_bytes_index = idx + 1;
                            break;
                        }
                    }

                    let control_mode_exited = tmux_performer.exited;
                    let parse_error = tmux_performer.parse_error;
                    let primary_pane_output = tmux_performer.finish();
                    handler.on_finish_byte_processing(&ProcessorInput {
                        bytes: &primary_pane_output,
                        ..input
                    });

                    if control_mode_exited {
                        self.state.tmux_control_mode = None;

                        // Tmux control mode has just exited -- notify the handler.
                        handler.tmux_control_mode_event(ControlModeEvent::Exited);

                        if parse_error {
                            // A parse error means that means control mode has exited unexpectedly and we
                            // shouldn't expect to get the OSC end marker, so we reset our state back to
                            // default manually.
                            *self = Default::default();

                            // A parse error also indicates that the last byte read (that caused the parse
                            // error) actually isn't from tmux control mode, so we should process it along
                            // with the rest of the remaining input.
                            remaining_bytes_index -= 1;
                        }
                    }

                    bytes = &bytes[remaining_bytes_index..];
                }
            }
        }
    }

    /// Parses an individual byte that is not part of a tmux control sequence.
    fn parse_byte<H, W>(&mut self, handler: &mut H, writer: &mut W, byte: u8)
    where
        H: Handler,
        W: io::Write,
    {
        let Some(buffer) = self.state.sync_output.buffer_mut() else {
            // If we're not in the middle of a synchronous output update,
            // then simply process the byte immediately.
            //
            // TODO (suraj): ideally, we'd create this performer once in `parse_bytes` to
            // improve performance: while the performer ctor is cheap, there is still a non-zero cost
            // that adds up if we do this per-byte. Implementing this is a challenge due to the fact
            // that the performer requires a mutable reference to [`ProcessorState`].
            let mut performer = Performer::new(&mut self.state, handler, writer);
            self.parser.advance(&mut performer, byte);
            return;
        };

        // At this point, we know this is a sync output update so
        // buffer the byte for processing later.
        buffer.push(byte);

        // Since we can't advancing the parser normally via the handler
        // during synchronized output (because we don't want the bytes to have
        // immediate effect), we need to somehow check if the synchronized output
        // is done (or extended). So we do that by explicitly checking if the
        // 'Set/Unset Mode' escape sequence was buffered.
        let len = buffer.len();
        let offset = len.saturating_sub(8);
        let end = &buffer[offset..];
        if end == b"\x1b[?2026h" {
            // This is an extension of the update.
            // Note: we remove these bytes so that they don't end up enabling a
            // new sync update when they're actually processed.
            buffer.truncate(offset);
            self.state.sync_output.activate();
        } else if end == b"\x1b[?2026l" {
            // No need to actually process the 'Unset Mode' escape sequence.
            buffer.truncate(offset);
            self.finish_sync_output(handler, writer);
        } else if Byte::from(len) >= *SYNC_OUTPUT_MAX_BUFFER_SIZE {
            // The update is too large, so let's end it.
            self.finish_sync_output(handler, writer);
        }
    }

    /// Completes a synchronized output update.
    pub fn finish_sync_output<H, W>(&mut self, handler: &mut H, writer: &mut W)
    where
        H: Handler,
        W: io::Write,
    {
        let Some(buffer) = self.state.sync_output.deactivate() else {
            return;
        };

        // Process all synchronized bytes.
        self.parse_bytes_internal(
            handler,
            ProcessorInput {
                bytes: buffer.as_slice(),
                is_synchronized_output_frame: true,
            },
            writer,
        );

        // Report that the update finished. We do this explicitly here rather than relying on
        // the 'Set Mode' escape sequence because we could have terminated the update due
        // to a timeout (in which case we won't have that escape sequence).
        handler.unset_mode(Mode::SyncOutput);
    }

    /// Returns the number of bytes buffered as part of the
    /// ongoing synchronous update, if any.
    pub fn sync_output_buffer_len(&self) -> Option<usize> {
        self.state.sync_output.buffer_len()
    }

    /// Returns the amount of time that should be waited for
    /// the current synchronous update, if any, to complete
    /// before force-ending it.
    pub fn sync_output_remaining_timeout(&self) -> Option<Duration> {
        self.state.sync_output.remaining_timeout()
    }
}

/// Helper type that implements [`VtePerform`].
///
/// Processor creates a Performer when running advance and passes the Performer
/// to [`VteParser`].
struct Performer<'a, H: Handler, W: io::Write> {
    state: &'a mut ProcessorState,
    handler: &'a mut H,
    writer: &'a mut W,
}

impl<'a, H: Handler + 'a, W: io::Write> Performer<'a, H, W> {
    /// Create a performer.
    #[inline]
    pub fn new<'b>(
        state: &'b mut ProcessorState,
        handler: &'b mut H,
        writer: &'b mut W,
    ) -> Performer<'b, H, W> {
        Performer {
            state,
            handler,
            writer,
        }
    }

    /// Calls the appropriate `ansi::Handler` function according to the given hook. This function
    /// assumes that the hook was encoded originally.
    fn handle_decoded_hook(&mut self, hook: Result<DProtoHook, serde_json::Error>) {
        match hook {
            Ok(DProtoHook::CommandFinished { value }) => self.handler.command_finished(value),
            Ok(DProtoHook::Precmd { value }) => self.handler.precmd(value),
            Ok(DProtoHook::Preexec { value }) => self.handler.preexec(value),
            Ok(DProtoHook::Bootstrapped { value }) => self.handler.bootstrapped(*value),
            Ok(DProtoHook::PreInteractiveSSHSession { value }) => {
                self.handler.pre_interactive_ssh_session(value)
            }
            Ok(DProtoHook::SSH { value }) => self.handler.ssh(value),
            Ok(DProtoHook::InitShell { value }) => self.handler.init_shell(value),
            Ok(DProtoHook::InputBuffer { value }) => self.handler.input_buffer(value),
            Ok(DProtoHook::Clear { value }) => self.handler.clear(value),
            Ok(DProtoHook::InitSubshell { value }) => self.handler.init_subshell(value),
            Ok(DProtoHook::InitSsh { value }) => self.handler.init_ssh(value),
            Ok(DProtoHook::SourcedRcFileForWarp { .. }) => {
                // The SourcedRCFileForWarp hook should only be emitted by the
                // shell without hex encoding. The RC file snippet given to
                // users is not hex-encoded for the sake of transparency and
                // debugability.
                log::error!("Received hex-encoded SourcedRcFileForWarp escape sequence.");
            }
            Ok(DProtoHook::FinishUpdate { value }) => self.handler.finish_update(value),
            Ok(DProtoHook::RemoteWarpificationIsUnavailable { value }) => {
                self.handler.remote_warpification_is_unavailable(value)
            }
            Ok(DProtoHook::SshTmuxInstaller { value }) => {
                if let Ok(tmux_installation) = TmuxInstallationState::from_str(&value) {
                    self.handler.notify_ssh_tmux_is_installed(tmux_installation)
                } else {
                    log::error!("Received invalid SSH tmux installer value: '{value}'");
                }
            }
            Ok(DProtoHook::TmuxInstallFailed { value }) => self.handler.tmux_install_failed(value),
            Ok(DProtoHook::ExitShell { value }) => self.handler.exit_shell(value),

            Err(e) => safe_error!(
                safe: ("Error when deserializing escape sequence data"),
                full: ("Error when deserializing escape sequence data: {:?}", e)
            ),
        }
    }

    /// Calls the appropriate `ansi::Handler` function according to the given hook. This function
    /// assumes that the hook was never encoded.
    fn handle_unencoded_hook(&mut self, hook: Result<DProtoHook, serde_json::Error>) {
        // Currently, only the `SourcedRcFileForWarp`, `InitShell`, `InitSubshell`, and `InitSsh`
        // DCS's may be emitted without hex-encoding -- other DCS hooks should be sent hex-encoded.
        // This is because we can guarantee that theses RC file hook don't contain non-ASCII chars
        // that might otherwise corrupt parsing of the PTY output (the same can't be said for the
        // payloads of other DCS hooks).
        match hook {
            Ok(DProtoHook::InitShell { value }) => self.handler.init_shell(value),
            Ok(DProtoHook::InitSubshell { value }) => {
                self.handler.init_subshell(value);
            }
            Ok(DProtoHook::SourcedRcFileForWarp { value }) => {
                self.handler.sourced_rc_file(value);
            }
            Ok(DProtoHook::InitSsh { value }) => {
                self.handler.init_ssh(value);
            }
            Ok(_) => {
                log::error!("Received non hex-encoded hook that is not SourcedRcFileForWarp");
            }
            Err(err) => {
                log::warn!("Received malformed SourcedRcFileForWarp hook {err:#}");
            }
        }
    }

    /// Handles hex-encoded data from a DCS or an OSC by decoding it and then handling the
    /// contained hook.
    fn handle_decoded_data(&mut self, decoded_data: Result<Vec<u8>, hex::FromHexError>) {
        match decoded_data {
            Ok(decoded_data) => {
                safe_debug!(
                    safe: ("Decoded payload"),
                    full: ("Decoded payload string: {:?}", std::str::from_utf8(&decoded_data))
                );

                let hook = serde_json::from_slice::<DProtoHook>(&decoded_data);
                if let Ok(hook) = &hook {
                    log::info!("Received {} hook", hook.name());
                }
                self.handle_decoded_hook(hook);
            }
            Err(e) => safe_error!(
                safe: ("Error when decoding payload"),
                full: ("Error when decoding payload: {:?}", e)
            ),
        }
    }

    fn handle_kv_marker(&mut self, params: &[&[u8]]) {
        match params.get(2) {
            Some(&WARP_KV_START_BYTE) => {
                let Some(hook) = params.get(3).map(|data| String::from_utf8_lossy(data)) else {
                    log::error!("Start pending hook OSC did not contain shell hook");
                    return;
                };
                self.handler.start_receiving_hook(hook.into());
            }
            Some(&WARP_KV_END_BYTE) => {
                let Some(pending_shell_hook) = self.handler.finish_receiving_hook() else {
                    return;
                };
                let hook = pending_shell_hook.finish();
                safe_debug!(
                    safe: ("Decoded payload"),
                    full: ("Decoded payload string: {:?}", serde_json::to_string(&hook))
                );
                self.handle_decoded_hook(Ok(hook));
            }
            Some(&WARP_KV_ENTRY_BYTE) => {
                let Some(key) = params.get(3) else {
                    log::error!("Pending hook update OSC did not contain key");
                    return;
                };
                let key = String::from_utf8_lossy(key);
                // We reconstruct the value here because if it contains a semicolon, it will get
                // separated by the parser. We are guaranteed that the value is intended to be the
                // last parameter.
                let value = params[4..]
                    .iter()
                    .map(|v| String::from_utf8_lossy(v))
                    .join(";");
                self.handler.update_hook(key.to_string(), value);
            }
            invalid_marker => {
                log::error!(
                    "Invalid marker {invalid_marker:?} received for pending shell hook OSC"
                );
            }
        }
    }
}

impl<'a, H, W> VtePerform for Performer<'a, H, W>
where
    H: Handler + 'a,
    W: io::Write + 'a,
{
    #[inline]
    fn print(&mut self, c: char) {
        self.handler.input(c);
        self.state.preceding_char = Some(c);
    }

    #[inline]
    fn execute(&mut self, byte: u8) {
        match byte {
            C0::HT => self.handler.put_tab(1),
            C0::BS => self.handler.backspace(),
            C0::CR => self.handler.carriage_return(),
            C0::LF | C0::VT | C0::FF => {
                self.handler.linefeed();
            }
            C0::BEL => self.handler.bell(),
            C0::SUB => self.handler.substitute(),
            C0::SI => self.handler.set_active_charset(CharsetIndex::G0),
            C0::SO => self.handler.set_active_charset(CharsetIndex::G1),
            _ => debug!("[unhandled] execute byte={byte:02x}"),
        }
    }

    #[inline]
    fn hook(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, c: char) {
        if FeatureFlag::SSHTmuxWrapper.is_enabled()
            && c == 'p'
            && params.len() == 1
            && params.iter().next() == Some(&[1000])
        {
            debug!("Entering tmux control mode, pending pane information.");
            self.state.tmux_control_mode = Some(TmuxControlMode::new());
        }
        self.state.dcs_data.on_hook(intermediates, c);
    }

    #[inline]
    fn put(&mut self, byte: u8) {
        self.state.dcs_data.push(byte);
    }

    #[inline]
    fn unhook(&mut self) {
        match self.state.dcs_data.final_char {
            HEX_ENCODED_JSON_MARKER => {
                let dcs_data_str = String::from_utf8_lossy(&self.state.dcs_data.data);
                safe_debug!(
                    safe: ("Received DCS string"),
                    full: ("Received DCS string with JSON payload: {:?}", dcs_data_str)
                );
                let decoded_data = hex::decode(&*dcs_data_str);
                self.handle_decoded_data(decoded_data);
            }
            UNENCODED_JSON_MARKER => {
                let dcs_data_str = String::from_utf8_lossy(&self.state.dcs_data.data);
                let hook = serde_json::from_str::<DProtoHook>(&dcs_data_str);
                self.handle_unencoded_hook(hook)
            }
            _ => (),
        }
    }

    // TODO replace OSC parsing with parser combinators.
    #[inline]
    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        let writer = &mut self.writer;
        let terminator = if bell_terminated { "\x07" } else { "\x1b\\" };

        fn format_params(params: &[&[u8]]) -> String {
            let mut buf = String::new();
            for items in params {
                buf.push('[');
                for item in *items {
                    let _ = write!(buf, "{}", *item as char);
                }
                buf.push_str("],");
            }
            buf
        }

        fn unhandled(params: &[&[u8]]) {
            debug!(
                "[unhandled osc_dispatch]: [{}] at line {}",
                format_params(params),
                line!()
            );
        }

        if params.is_empty() || params[0].is_empty() {
            return;
        }

        match params[0] {
            // Note that we're currently interpreting the TITLE event as a
            // tab title instead of a window title.
            b"0" | b"2" => {
                if params.len() >= 2 {
                    let title = params[1..]
                        .iter()
                        .flat_map(|x| str::from_utf8(x))
                        .collect::<Vec<&str>>()
                        .join(";")
                        .trim()
                        .to_owned();
                    self.handler.set_title(Some(title));
                    return;
                }
                unhandled(params);
            }

            // Set color index.
            b"4" => {
                if params.len() > 1 && !params.len().is_multiple_of(2) {
                    for chunk in params[1..].chunks(2) {
                        let index = parse_number(chunk[0]);
                        let color = xparse_color(chunk[1]);
                        if let (Some(i), Some(c)) = (index, color) {
                            self.handler.set_color(i as usize, c);
                            return;
                        }
                    }
                }
                unhandled(params);
            }

            // OSC 9: Desktop notification (iTerm2/xterm style)
            // Format: OSC 9 ; <message> ST
            //
            // ConEmu uses OSC 9 with a numeric subcommand for various system integrations:
            //   9;4  - progress reports
            //   9;9  - current working directory (also adopted by Windows Terminal)
            // Reference: https://conemu.github.io/en/AnsiEscapeCodes.html#ConEmu_specific_OSC
            // Reference: https://github.com/microsoft/terminal/issues/8166
            //
            // We distinguish notification messages from subcommands by checking if params[1]
            // is purely numeric. ConEmu subcommands always have a numeric second parameter;
            // notification messages are freeform text and are extremely unlikely to be purely numeric.
            b"9" => {
                if params.len() >= 2 {
                    if !params[1].is_empty() && params[1].iter().all(u8::is_ascii_digit) {
                        return;
                    }
                    let body = params[1..]
                        .iter()
                        .map(|x| str::from_utf8(x))
                        .collect::<Result<Vec<_>, _>>()
                        .map(|parts| parts.join(";").trim().to_owned());
                    if let Ok(body) = body {
                        if !body.is_empty() {
                            log::info!("Received OSC 9 notification: {}", body);
                            self.handler.pluggable_notification(None, body);
                            return;
                        }
                    }
                }
                unhandled(params);
            }

            // Get/set Foreground, Background, Cursor colors.
            b"10" | b"11" | b"12" => {
                if params.len() >= 2 {
                    if let Some(mut dynamic_code) = parse_number(params[0]) {
                        for param in &params[1..] {
                            // 10 is the first dynamic color, also the foreground.
                            let offset = dynamic_code as usize - 10;
                            let index = color_index::FOREGROUND + offset;

                            // End of setting dynamic colors.
                            if index > color_index::CURSOR {
                                unhandled(params);
                                break;
                            }

                            if let Some(color) = xparse_color(param) {
                                self.handler.set_color(index, color);
                            } else if param == b"?" {
                                self.handler.dynamic_color_sequence(
                                    writer,
                                    dynamic_code,
                                    index,
                                    terminator,
                                );
                            } else {
                                unhandled(params);
                            }
                            dynamic_code += 1;
                        }
                        return;
                    }
                }
                unhandled(params);
            }

            // Set cursor style.
            b"50" => {
                if params.len() >= 2
                    && params[1].len() >= 13
                    && params[1][0..12] == *b"CursorShape="
                {
                    let shape = match params[1][12] as char {
                        '0' => CursorShape::Block,
                        '1' => CursorShape::Beam,
                        '2' => CursorShape::Underline,
                        _ => return unhandled(params),
                    };
                    self.handler.set_cursor_shape(shape);
                    return;
                }
                unhandled(params);
            }

            // Set clipboard.
            b"52" => {
                if params.len() < 3 {
                    return unhandled(params);
                }

                let clipboard = params[1].first().unwrap_or(&b'c');
                match params[2] {
                    b"?" => self.handler.clipboard_load(*clipboard, terminator),
                    base64 => self.handler.clipboard_store(*clipboard, base64),
                }
            }

            // Reset color index.
            b"104" => {
                // Reset all color indexes when no parameters are given.
                if params.len() == 1 {
                    for i in 0..256 {
                        self.handler.reset_color(i);
                    }
                    return;
                }

                // Reset color indexes given as parameters.
                for param in &params[1..] {
                    match parse_number(param) {
                        Some(index) => self.handler.reset_color(index as usize),
                        None => unhandled(params),
                    }
                }
            }

            // Reset foreground color.
            b"110" => self.handler.reset_color(color_index::FOREGROUND),

            // Reset background color.
            b"111" => self.handler.reset_color(color_index::BACKGROUND),

            // Reset text cursor color.
            b"112" => self.handler.reset_color(color_index::CURSOR),

            // FinalTerm prompt start/end marks.
            b"133" => match PromptMarker::try_from(&params[1..]) {
                Ok(marker) => {
                    debug!("Received prompt marker: {marker:?}");
                    self.handler.prompt_marker(marker)
                }
                _ => unhandled(params),
            },

            // OSC 777: Desktop notification (urxvt/foot style)
            // Format: OSC 777 ; notify ; <title> ; <body> ST
            // Title is optional but body is required.
            // Reference: https://man.archlinux.org/man/urxvtperl.3.en
            b"777" => {
                if params.len() >= 4 && params[1] == b"notify" {
                    let title = params
                        .get(2)
                        .and_then(|t| str::from_utf8(t).ok())
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty());
                    let body = params
                        .get(3..)
                        .map(|rest| {
                            rest.iter()
                                .flat_map(|x| str::from_utf8(x))
                                .collect::<Vec<&str>>()
                                .join(";")
                                .trim()
                                .to_owned()
                        })
                        .unwrap_or_default();
                    if !body.is_empty() {
                        log::info!(
                            "Received OSC 777 notification: title={:?}, body={}",
                            title,
                            body
                        );
                        self.handler.pluggable_notification(title, body.to_owned());
                        return;
                    }
                }
                unhandled(params);
            }

            // iTerm inline image protocol.
            b"1337" => {
                if params[1].starts_with(b"File=") {
                    let metadata = parse_iterm_image_metadata(params);

                    let last_param = match params.last() {
                        Some(param) => param,
                        None => return unhandled(params),
                    };

                    let image_data = match last_param.iter().position(|&byte| byte == b':') {
                        Some(position) => &last_param[position + 1..],
                        None => return unhandled(params),
                    };

                    self.handler.start_iterm_image_receiving(metadata);
                    self.handler.on_iterm_image_data_received(image_data);
                    self.handler.end_iterm_image_receiving();
                } else if params[1].starts_with(b"MultipartFile=") {
                    let metadata = parse_iterm_image_metadata(params);
                    self.handler.start_iterm_image_receiving(metadata);
                } else if params[1].starts_with(b"FilePart=") {
                    let image_data = match params[1].iter().position(|&byte| byte == b'=') {
                        Some(position) => &params[1][position + 1..],
                        None => return unhandled(params),
                    };
                    self.handler.on_iterm_image_data_received(image_data);
                } else if params[1].starts_with(b"FileEnd") {
                    self.handler.end_iterm_image_receiving();
                } else {
                    unhandled(params)
                }
            }

            // Received a Warp OSC used for in-band generators.
            WARP_IN_BAND_GENERATOR_OSC_MARKER => match params.get(1) {
                Some(&WARP_IN_BAND_GENERATOR_START_BYTE) => {
                    log::info!("Received a Warp OSC marker for starting in-band command output.");
                    self.handler.start_in_band_command_output();
                }
                Some(&WARP_IN_BAND_GENERATOR_END_BYTE) => {
                    self.handler.end_in_band_command_output(true);
                }
                _ => {
                    log::warn!("Received a Warp OSC marker missing required param.");
                }
            },

            // Received a Warp OSC used for shell hooks.
            WARP_OSC_MARKER => {
                let Some(json_marker_char) = params
                    .get(1)
                    .map(|json_marker_bytes| String::from_utf8_lossy(json_marker_bytes))
                    .and_then(|json_marker_str| json_marker_str.chars().next())
                else {
                    log::error!("Could not retrieve OSC JSON marker");
                    return;
                };

                match json_marker_char {
                    HEX_ENCODED_JSON_MARKER => {
                        // The payload for the OSC is contained in the third parameter.
                        let Some(data_str) = params
                            .get(2)
                            .map(|osc_data| String::from_utf8_lossy(osc_data))
                        else {
                            log::error!("Warp OSC marker did not contain payload");
                            return;
                        };
                        safe_debug!(
                            safe: ("Received Warp OSC string for shell hook"),
                            full: ("Received Warp OSC string for shell hook with JSON payload: {:?}", data_str)
                        );
                        let decoded_data = hex::decode(&*data_str);
                        self.handle_decoded_data(decoded_data);
                    }
                    UNENCODED_JSON_MARKER => {
                        // The payload for the OSC is contained in the third parameter.
                        let Some(data_str) = params
                            .get(2)
                            .map(|osc_data| String::from_utf8_lossy(osc_data))
                        else {
                            log::error!("Warp OSC marker did not contain payload");
                            return;
                        };
                        safe_debug!(
                            safe: ("Received Warp OSC string for shell hook"),
                            full: ("Received Warp OSC string for shell hook with JSON payload: {:?}", data_str)
                        );
                        let hook = serde_json::from_str::<DProtoHook>(&data_str);
                        self.handle_unencoded_hook(hook)
                    }
                    UNENCODED_KV_MARKER => self.handle_kv_marker(params),
                    _ => {
                        log::error!("Invalid OSC JSON marker found: {json_marker_char}");
                    }
                }
            }

            WARP_RESET_GRID_OSC_MARKER => {
                log::debug!("Received Warp OSC string for reset grid");
                self.handler.on_reset_grid();
            }

            // Received a Warp OSC used for completions.
            WARP_COMPLETIONS_OSC_MARKER => match params.get(1) {
                Some(&WARP_COMPLETIONS_START_BYTE) => {
                    let Some(format) = params
                        .get(2)
                        .map(|osc_data| String::from_utf8_lossy(osc_data))
                        .and_then(|format| CompletionsShellData::from_format_type(&format))
                    else {
                        log::warn!("Warp start completions OSC marker contained invalid format.");
                        return;
                    };
                    self.handler.start_completions_output(format);
                }
                Some(&WARP_COMPLETIONS_END_BYTE) => {
                    self.handler.end_completions_output();
                }
                Some(&WARP_COMPLETIONS_MATCH_RESULT_BYTE) => {
                    // The payload for the OSC is contained in the third parameter.
                    let Some(data_str) = params
                        .get(2)
                        .map(|osc_data| String::from_utf8_lossy(osc_data))
                    else {
                        log::warn!(
                            "Warp completions match result OSC marker did not contain payload"
                        );
                        return;
                    };

                    let shell_completion_result = ShellCompletion::new(data_str.to_string());

                    self.handler
                        .on_completion_result_received(shell_completion_result);
                }
                Some(bytes) if bytes.starts_with(WARP_COMPLETIONS_MATCH_UPDATE_METADATA) => {
                    let Ok(parameter) = String::from_utf8(bytes.to_vec()) else {
                        log::warn!(
                            "Unable to convert update completions match parameter into a string"
                        );
                        return;
                    };

                    // Read out the payload for the OSC (stored in the 3rd parameter).
                    let Some(data_str) = params
                        .get(2)
                        .map(|osc_data| String::from_utf8_lossy(osc_data))
                    else {
                        log::warn!(
                            "Warp completions match metadata OSC marker did not contain payload"
                        );
                        return;
                    };

                    // Determine which field we are trying to update.
                    match &parameter[2..] {
                        "description" => {
                            self.handler.update_last_completion_result(
                                ShellCompletionUpdate::Description {
                                    value: data_str.into(),
                                },
                            );
                        }
                        _ => {
                            log::warn!("Invalid Warp OSC marker parameter for completions match metadata: {parameter}");
                        }
                    }
                }
                Some(&WARP_COMPLETIONS_PROMPT_BYTE) => {
                    self.handler.send_completions_prompt();
                }
                _ => {
                    log::warn!("Received a Warp OSC completions marker missing required param.");
                }
            },

            // This is a totally random OSC identifier to test how often we parse unexpected OSCs.
            b"781378" => {
                log::error!(
                    "Received unexpected OSC identifier (781378) with parameters: [{}]",
                    format_params(params)
                );
            }

            _ => unhandled(params),
        }
    }

    #[allow(clippy::cognitive_complexity)]
    #[inline]
    fn csi_dispatch(
        &mut self,
        params: &Params,
        intermediates: &[u8],
        has_ignored_intermediates: bool,
        action: char,
    ) {
        macro_rules! unhandled {
            () => {{
                debug!(
                    "[Unhandled CSI] action={:?}, params={:?}, intermediates={:?}",
                    action, params, intermediates
                );
            }};
        }

        if has_ignored_intermediates || intermediates.len() > 2 {
            unhandled!();
            return;
        }

        let mut params_iter = params.iter();
        let handler = &mut self.handler;
        let writer = &mut self.writer;

        let mut next_param_or = |default: u16| {
            params_iter
                .next()
                .map(|param| param[0])
                .filter(|&param| param != 0)
                .unwrap_or(default)
        };

        match (action, intermediates.first()) {
            ('@', None) => handler.insert_blank(next_param_or(1) as usize),
            ('A', None) => {
                handler.move_up(next_param_or(1) as usize);
            }
            ('B', None) | ('e', None) => handler.move_down(next_param_or(1) as usize),
            ('b', None) => {
                if let Some(c) = self.state.preceding_char {
                    for _ in 0..next_param_or(1) {
                        handler.input(c);
                    }
                } else {
                    debug!("tried to repeat with no preceding char");
                }
            }
            ('C', None) | ('a', None) => handler.move_forward(next_param_or(1) as usize),
            ('c', intermediate) if next_param_or(0) == 0 => {
                handler.identify_terminal(writer, intermediate.map(|&i| i as char))
            }
            ('D', None) => handler.move_backward(next_param_or(1) as usize),
            ('d', None) => handler.goto_line(VisibleRow(next_param_or(1) as usize - 1)),
            ('E', None) => handler.move_down_and_cr(next_param_or(1) as usize),
            ('F', None) => handler.move_up_and_cr(next_param_or(1) as usize),
            ('G', None) | ('`', None) => handler.goto_col(next_param_or(1) as usize - 1),
            ('g', None) => {
                let mode = match next_param_or(0) {
                    0 => TabulationClearMode::Current,
                    3 => TabulationClearMode::All,
                    _ => {
                        unhandled!();
                        return;
                    }
                };

                handler.clear_tabs(mode);
            }
            ('H', None) | ('f', None) => {
                let y = next_param_or(1) as usize;
                let x = next_param_or(1) as usize;
                handler.goto(VisibleRow(y - 1), x - 1);
            }
            ('h', intermediate) => {
                for param in params_iter.map(|param| param[0]) {
                    match Mode::from_primitive(intermediate, param) {
                        Some(mode) => {
                            handler.set_mode(mode);
                            if mode == Mode::SyncOutput {
                                self.state.sync_output.activate();
                            }
                        }
                        None => unhandled!(),
                    }
                }
            }
            ('I', None) => handler.move_forward_tabs(next_param_or(1)),
            ('J', None) => {
                let mode = match next_param_or(0) {
                    0 => ClearMode::Below,
                    1 => ClearMode::Above,
                    2 => ClearMode::All,
                    3 => ClearMode::Saved,
                    _ => {
                        unhandled!();
                        return;
                    }
                };

                handler.clear_screen(mode);
            }
            ('K', None) => {
                let mode = match next_param_or(0) {
                    0 => LineClearMode::Right,
                    1 => LineClearMode::Left,
                    2 => LineClearMode::All,
                    _ => {
                        unhandled!();
                        return;
                    }
                };

                handler.clear_line(mode);
            }
            ('L', None) => {
                handler.insert_blank_lines(next_param_or(1) as usize);
            }
            ('l', intermediate) => {
                for param in params_iter.map(|param| param[0]) {
                    match Mode::from_primitive(intermediate, param) {
                        Some(mode) => {
                            // We expect to only handle the end of a sync output update
                            // when sync output is active, in which case it would be handled
                            // via [`Performer::stop_sync`].
                            if mode != Mode::SyncOutput {
                                handler.unset_mode(mode);
                            }
                        }
                        None => unhandled!(),
                    }
                }
            }
            ('M', None) => {
                handler.delete_lines(next_param_or(1) as usize);
            }
            ('m', None) => {
                if params.is_empty() {
                    handler.terminal_attribute(Attr::Reset);
                } else {
                    for attr in attrs_from_sgr_parameters(&mut params_iter) {
                        match attr {
                            Some(attr) => handler.terminal_attribute(attr),
                            None => unhandled!(),
                        }
                    }
                }
            }
            ('n', None) => handler.device_status(writer, next_param_or(0) as usize),
            ('P', None) => handler.delete_chars(next_param_or(1) as usize),
            ('p', intermediate) => {
                // This is a DECRQM (request mode) query [1], which expects a
                // DECRPM (report mode) response [2].
                // [1] https://vt100.net/docs/vt510-rm/DECRQM.html
                // [2] https://vt100.net/docs/vt510-rm/DECRPM.html.
                //
                // Currently, it's easier to issue the response in the performer
                // rather than routed through ansi::Handler. If we wanted to route it
                // through ansi::Handler, we'd need to propagate state (like whether or not
                // sync output is active). This might be necessary at some point, but it isn't today.
                match Mode::from_primitive(intermediate, next_param_or(0)) {
                    Some(Mode::SyncOutput) => {
                        // Based on https://gist.github.com/christianparpart/d8a62cc1ab659194337d73e399004036#feature-detection.
                        let response = format!(
                            "\x1b[?2026;{}$y",
                            if self.state.sync_output.is_active() {
                                "1"
                            } else {
                                "2"
                            }
                        );
                        let _ = writer.write_all(response.as_bytes());
                    }
                    _ => unhandled!(),
                }
            }
            ('q', Some(b' ')) => {
                // DECSCUSR (CSI Ps SP q) -- Set Cursor Style.
                let cursor_style_id = next_param_or(0);
                let shape = match cursor_style_id {
                    0 => None,
                    1 | 2 => Some(CursorShape::Block),
                    3 | 4 => Some(CursorShape::Underline),
                    5 | 6 => Some(CursorShape::Beam),
                    _ => {
                        unhandled!();
                        return;
                    }
                };
                let cursor_style = shape.map(|shape| CursorStyle {
                    shape,
                    blinking: cursor_style_id % 2 == 1,
                });

                handler.set_cursor_style(cursor_style);
            }
            ('q', Some(b'>')) => handler.report_xtversion(self.writer),
            ('r', None) => {
                let top = next_param_or(1) as usize;
                let bottom = params_iter
                    .next()
                    .map(|param| param[0] as usize)
                    .filter(|&param| param != 0);

                handler.set_scrolling_region(top, bottom);
            }
            ('S', None) => {
                handler.scroll_up(next_param_or(1) as usize);
            }
            ('s', None) => handler.save_cursor_position(),
            ('T', None) => {
                handler.scroll_down(next_param_or(1) as usize);
            }
            ('t', None) => match next_param_or(1) as usize {
                14 => handler.text_area_size_pixels(writer),
                18 => handler.text_area_size_chars(writer),
                22 => handler.push_title(),
                23 => handler.pop_title(),
                _ => unhandled!(),
            },
            ('u', None) => handler.restore_cursor_position(),
            // Kitty keyboard protocol:
            // - CSI = flags ; mode u  (set flags, with apply mode)
            // - CSI > flags u         (push current flags and set new flags)
            // - CSI < count u         (pop keyboard modes)
            // - CSI ? u               (query current flags)
            // Disabled on Windows because ConPTY cannot forward kitty-encoded key
            // events or terminal query responses, causing applications (e.g. running
            // in WSL) to hang.
            ('u', Some(b'=')) if cfg!(not(windows)) => {
                let flags = KeyboardModes::from_bits_truncate(next_param_or(0) as u32);
                let apply_mode = next_param_or(1);
                let Some(apply) = KeyboardModesApplyBehavior::from_kitty_apply_mode(apply_mode)
                else {
                    return;
                };
                handler.set_keyboard_enhancement_flags(flags, apply);
            }
            ('u', Some(b'>')) if cfg!(not(windows)) => {
                let flags = KeyboardModes::from_bits_truncate(next_param_or(0) as u32);
                handler.push_keyboard_enhancement_flags(flags);
            }
            ('u', Some(b'<')) if cfg!(not(windows)) => {
                let count = next_param_or(1);
                handler.pop_keyboard_enhancement_flags(count);
            }
            ('u', Some(b'?')) if cfg!(not(windows)) => {
                handler.query_keyboard_enhancement_flags(writer);
            }
            ('X', None) => handler.erase_chars(next_param_or(1) as usize),
            ('Z', None) => handler.move_backward_tabs(next_param_or(1)),
            _ => unhandled!(),
        }
    }

    #[inline]
    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        macro_rules! unhandled {
            () => {{
                debug!(
                    "[unhandled] esc_dispatch ints={:?}, byte={:?} ({:02x})",
                    intermediates, byte as char, byte
                );
            }};
        }

        macro_rules! configure_charset {
            ($charset:path, $intermediates:expr) => {{
                let index: CharsetIndex = match $intermediates {
                    [b'('] => CharsetIndex::G0,
                    [b')'] => CharsetIndex::G1,
                    [b'*'] => CharsetIndex::G2,
                    [b'+'] => CharsetIndex::G3,
                    _ => {
                        unhandled!();
                        return;
                    }
                };
                self.handler.configure_charset(index, $charset)
            }};
        }

        match (byte, intermediates) {
            (b'B', intermediates) => configure_charset!(StandardCharset::Ascii, intermediates),
            (b'D', []) => {
                self.handler.linefeed();
            }
            (b'E', []) => {
                self.handler.linefeed();
                self.handler.carriage_return();
            }
            (b'H', []) => self.handler.set_horizontal_tabstop(),
            (b'M', []) => {
                self.handler.reverse_index();
            }
            (b'Z', []) => self.handler.identify_terminal(self.writer, None),
            (b'c', []) => self.handler.reset_state(),
            (b'0', intermediates) => configure_charset!(
                StandardCharset::SpecialCharacterAndusizeDrawing,
                intermediates
            ),
            (b'7', []) => self.handler.save_cursor_position(),
            (b'8', [b'#']) => self.handler.decaln(),
            (b'8', []) => self.handler.restore_cursor_position(),
            (b'=', []) => self.handler.set_keypad_application_mode(),
            (b'>', []) => self.handler.unset_keypad_application_mode(),
            // String terminator, do nothing (parser handles as string terminator).
            (b'\\', []) => (),
            _ => unhandled!(),
        }
    }

    fn apc_start(&mut self) {
        self.state.apc_data.clear();
    }

    fn apc_put(&mut self, byte: u8) {
        self.state.apc_data.push(byte);
    }

    fn apc_end(&mut self) {
        let first_byte = match self.state.apc_data.first() {
            Some(&first_byte) => first_byte,
            None => return,
        };

        let writer = &mut self.writer;

        // 'G' identifies a Kitty image APC message
        if first_byte == b'G' {
            if !FeatureFlag::KittyImages.is_enabled() {
                return;
            }

            let parsed_chunk = parse_kitty_chunk(self.state.apc_data[1..].to_vec());

            let further_chunks = parsed_chunk.control_data.further_chunks;

            self.handler.on_kitty_image_chunk_received(parsed_chunk);

            if !further_chunks {
                self.handler.end_kitty_action_receiving(writer)
            }
        }
    }
}

enum PrimaryPaneState {
    Pending {
        /// Buffer of output from { pane_id: bytes }
        pane_output_map: HashMap<u32, Vec<u8>>,
    },
    Ready {
        pane_id: u32,
    },
}

/// A writer that wraps another writer and converts writes to tmux send-keys format.
/// This is used when processing pane output in tmux control mode, so that responses
/// to terminal queries (like cursor position requests) are sent back to the application
/// inside the pane via tmux's send-keys mechanism.
struct TmuxPaneWriter<'a, W: io::Write> {
    inner: &'a mut W,
    pane_id: u32,
}

impl<'a, W: io::Write> TmuxPaneWriter<'a, W> {
    fn new(inner: &'a mut W, pane_id: u32) -> Self {
        Self { inner, pane_id }
    }
}

impl<W: io::Write> io::Write for TmuxPaneWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        // Format the bytes as a tmux send-keys command.
        // The format is: send-keys -Ht %{pane_id} {hex} {hex}...\n
        let formatted = format_input(self.pane_id, buf);
        self.inner.write_all(formatted.as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl PrimaryPaneState {
    fn new() -> Self {
        PrimaryPaneState::Pending {
            pane_output_map: HashMap::new(),
        }
    }
}

struct TmuxPerformer<'a, H: Handler, W: io::Write> {
    state: &'a mut TmuxControlModeState,
    exited: bool,
    parse_error: bool,
    handler: &'a mut H,
    writer: &'a mut W,
    primary_pane_output: Vec<u8>,
}

impl<'a, H: Handler + 'a, W: io::Write> TmuxPerformer<'a, H, W> {
    /// Create a performer.
    #[inline]
    pub fn new<'b>(
        state: &'b mut TmuxControlModeState,
        handler: &'b mut H,
        writer: &'b mut W,
    ) -> TmuxPerformer<'b, H, W> {
        TmuxPerformer {
            state,
            exited: false,
            parse_error: false,
            handler,
            writer,
            primary_pane_output: Vec::new(),
        }
    }

    fn init_primary_pane(&mut self, primary_window: u32, primary_pane: u32) {
        let previous_pane_state = std::mem::replace(
            &mut self.state.primary_pane,
            PrimaryPaneState::Ready {
                pane_id: primary_pane,
            },
        );

        let PrimaryPaneState::Pending { pane_output_map } = previous_pane_state else {
            log::error!("Received primary pane initialization message after primary pane was already initialized!");
            return;
        };

        self.handler
            .tmux_control_mode_event(ControlModeEvent::ControlModeReady {
                primary_window,
                primary_pane,
            });

        for (pane, bytes) in pane_output_map.into_iter() {
            if primary_pane == pane {
                for byte in bytes.iter() {
                    self.process_primary_pane_output(*byte, primary_pane);
                }
                self.handler
                    .on_finish_byte_processing(&ProcessorInput::new(&bytes));
            } else {
                for byte in bytes {
                    self.handler
                        .tmux_control_mode_event(ControlModeEvent::BackgroundPaneOutput {
                            pane,
                            byte,
                        });
                }
            }
        }
    }

    fn process_primary_pane_output(&mut self, byte: u8, pane_id: u32) {
        // TODO(ddfisher): don't create a new performer on every byte
        //
        // Use a TmuxPaneWriter to wrap the writer so that responses to terminal
        // queries (like cursor position requests via ESC[6n) are sent back to
        // the application inside the pane via tmux's send-keys mechanism.
        let mut tmux_writer = TmuxPaneWriter::new(self.writer, pane_id);
        let mut performer = Performer::new(
            &mut self.state.ansi_processor.state,
            self.handler,
            &mut tmux_writer,
        );
        self.state
            .ansi_processor
            .parser
            .advance(&mut performer, byte);
    }

    fn finish(self) -> Vec<u8> {
        self.primary_pane_output
    }
}

impl<'a, H, W> TmuxControlModeHandler for TmuxPerformer<'a, H, W>
where
    H: Handler + 'a,
    W: io::Write + 'a,
{
    fn pane_output(&mut self, pane: u32, byte: u8) {
        let primary_pane_id = match &self.state.primary_pane {
            PrimaryPaneState::Ready { pane_id } => Some(*pane_id),
            PrimaryPaneState::Pending { .. } => None,
        };

        match primary_pane_id {
            None => {
                // Buffer output until we know which pane is the primary
                if let PrimaryPaneState::Pending { pane_output_map } = &mut self.state.primary_pane
                {
                    pane_output_map.entry(pane).or_default().push(byte);
                } else {
                    debug_assert!(false, "Expected Pending state while buffering pane output");
                }
            }
            Some(primary_pane) => {
                if pane == primary_pane {
                    self.primary_pane_output.push(byte);
                    self.process_primary_pane_output(byte, primary_pane);
                } else {
                    self.handler
                        .tmux_control_mode_event(ControlModeEvent::BackgroundPaneOutput {
                            pane,
                            byte,
                        });
                }
            }
        };
    }

    fn tmux_control_mode_message(&mut self, message: TmuxMessage) {
        match message {
            TmuxMessage::Exit => {
                self.exited = true;
            }
            TmuxMessage::ParseError {
                message: _,
                byte: _,
            } => {
                self.exited = true;
                self.parse_error = true;
            }
            TmuxMessage::CommandOutput { output_lines } => {
                if let Ok(output_lines) = output_lines {
                    for line in output_lines {
                        let Some(command) = parse_command(line) else {
                            continue;
                        };
                        match command {
                            TmuxCommandResponse::SetPrimaryWindowPane { window_id, pane_id } => {
                                self.state.pane_for_window.insert(window_id, pane_id);
                                self.init_primary_pane(window_id, pane_id);
                            }
                            TmuxCommandResponse::BackgroundWindow { window_id, pane_id } => {
                                self.state.pane_for_window.insert(window_id, pane_id);
                            }
                        }
                    }
                }
            }
            TmuxMessage::Unknown { tag: _, rest: _ } => {}
            TmuxMessage::WindowClose { window_id: _ } => {}
        }
    }
}

// Tests for parsing escape sequences.
//
// Byte sequences used in these tests are recording of pty stdout.
#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
