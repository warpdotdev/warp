use std::fmt::{self, Debug, Formatter};
use std::num::ParseIntError;
use std::string::FromUtf8Error;
use std::sync::Arc;

use hex::FromHexError;
use itertools::Itertools as _;
use warp_util::AsciiDebug;

use crate::{ClipboardType, ImageProtocol};
/// Emitted upon completion of an executor command that goes through the pty, such as the
/// InBandCommandExecutor.
#[derive(Clone)]
pub struct ExecutedExecutorCommandEvent {
    pub command_id: String,
    pub exit_code: usize,
    pub output: Vec<u8>,
}

impl ExecutedExecutorCommandEvent {
    /// Parses the given `payload` (expected to be the payload of a generator output OSC) into a
    /// `ExecutedGeneratorCommandValue`.
    ///
    /// The given `string` is expected to follow the following format:
    ///     <commmand_id>;<output>;<exit_code>
    ///
    /// Returns a `ParseGeneratorCommandValueError` if payload cannot be successfully parsed.
    ///
    pub fn parse_generator_payload(payload: Vec<u8>) -> Result<Self, ParseGeneratorOutputError> {
        // Break the payload apart at the first and last semicolons.
        let mut payload_initial_split = payload.splitn(2, |&byte| byte == b';');

        let Some(before_first_semicolon) = payload_initial_split.next() else {
            return Err(ParseGeneratorOutputError::Corrupted);
        };

        let Some(after_first_semicolon) = payload_initial_split.next() else {
            return Err(ParseGeneratorOutputError::Corrupted);
        };

        let mut payload_final_split = after_first_semicolon.rsplitn(2, |&byte| byte == b';');
        let Some(after_final_semicolon) = payload_final_split.next() else {
            return Err(ParseGeneratorOutputError::Corrupted);
        };

        let Some(payload_middle) = payload_final_split.next() else {
            return Err(ParseGeneratorOutputError::Corrupted);
        };

        let command_id = String::from_utf8(before_first_semicolon.to_vec())
            .map_err(ParseGeneratorOutputError::Utf8DecodingFailure)?;

        let exit_code = String::from_utf8(after_final_semicolon.to_vec())
            .map_err(ParseGeneratorOutputError::Utf8DecodingFailure)?
            .parse::<usize>()
            .map_err(ParseGeneratorOutputError::ExitCodeParseFailure)?;

        // The output of the command remains as bytes. This is so we can operate on the bytes higher in
        // the stack if we need to, such as in the case of parsing out the zsh history file where we want to
        // transform the byte array before converting to a string.
        let output = payload_middle.to_vec();

        Ok(Self {
            command_id,
            exit_code,
            output,
        })
    }
}

impl Debug for ExecutedExecutorCommandEvent {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutedExecutorCommandEvent")
            .field("command_id", &self.command_id)
            .field("exit_code", &self.exit_code)
            .field("output", &AsciiDebug(&self.output))
            .finish()
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ParseGeneratorOutputError {
    #[error("Failed to parse exit code: {0:?}")]
    ExitCodeParseFailure(ParseIntError),
    #[error("Corrupted DCS. Should be of the format <command_id>;<exit_code>;<output>. ")]
    Corrupted,
    #[error("Failed to convert to Utf8: {0:?}")]
    Utf8DecodingFailure(FromUtf8Error),
}

#[derive(Debug, Copy, Clone)]
pub enum ExitReason {
    /// The shell process exited naturally
    ShellProcessExited,
    /// PTY spawn failed
    PtySpawnFailed,
    /// PTY connection was lost/disconnected
    PtyDisconnected,
    /// Process was killed/terminated
    ProcessKilled,
    /// Shell could not be found/determined
    ShellNotFound,
}
/// Validates and decodes in-band command output sent via `warp_send_generator_output_osc_message`.
/// Upon success, returns the string content of the generator output. The OSC payload is expected
/// to conform to the following format:
///
///   <content_length>;<content>
///
/// where `content_length` is the length (number of bytes) in `content`.  If the
/// payload does not conform to this format or if expected content length does not
/// match the actual content length, returns an error.
pub fn validate_and_decode_in_band_command_output_to_bytes(
    raw_payload: &str,
) -> Result<Vec<u8>, InBandCommandOutputDecodingError> {
    let components = raw_payload.splitn(2, ';').collect_vec();
    if components.len() != 2 {
        return Err(InBandCommandOutputDecodingError::NoContentLengthHeader);
    }

    let expected_content_length = components[0]
        .parse::<usize>()
        .map_err(InBandCommandOutputDecodingError::ContentLengthHeaderCorrupted)?;
    let payload: &str = components[1].trim();
    let actual_content_length = payload.len();
    if actual_content_length != expected_content_length {
        return Err(InBandCommandOutputDecodingError::ContentLengthMismatch {
            actual_length: actual_content_length,
            expected_length: expected_content_length,
        });
    }

    hex::decode(payload).map_err(InBandCommandOutputDecodingError::HexDecodingFailure)
}

#[derive(thiserror::Error, Debug)]
pub enum InBandCommandOutputDecodingError {
    #[error("Missing content length header.")]
    NoContentLengthHeader,
    #[error("DCS content length header is corrupted: {0:?}")]
    ContentLengthHeaderCorrupted(ParseIntError),
    #[error(
        "Content length header does not match length of received content. Actual: {actual_length}, expected: {expected_length}"
    )]
    ContentLengthMismatch {
        actual_length: usize,
        expected_length: usize,
    },
    #[error("Failed to hex-decode the DCS payload: {0:?}")]
    HexDecodingFailure(FromHexError),
}
#[derive(Clone)]
pub enum Event {
    MouseCursorDirty,
    ClipboardStore(ClipboardType, String),
    ClipboardLoad(
        ClipboardType,
        Arc<dyn Fn(&str) -> String + Sync + Send + 'static>,
    ),
    CursorBlinkingChange(bool),
    Bell,
    ImageReceived {
        image_id: u32,
        image_data: Vec<u8>,
        image_protocol: ImageProtocol,
    },
}
