//! An incremental scanner for Warp's DCS lifecycle hooks in raw PTY output.

use serde::Deserialize;

/// Hook payloads larger than this are ignored. The largest hook,
/// `Bootstrapped`, carries the shell's aliases, function names, builtins,
/// options and exported variable names; a typical zsh setup sends about 16 KiB
/// of JSON (32 KiB hex-encoded) and heavy plugin setups a few hundred KiB, so
/// this leaves a wide margin.
pub(super) const MAX_HOOK_PAYLOAD: usize = 8 * 1024 * 1024;

const ESC: u8 = 0x1b;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
const ST_8BIT: u8 = 0x9c;
const HEX_MARKER: u8 = b'd';
const JSON_MARKER: u8 = b'f';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HookKind {
    CommandFinished,
    Precmd,
    Preexec,
    InitShell { is_subshell: bool },
    Bootstrapped,
}

/// A recognized lifecycle hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Hook {
    pub kind: HookKind,
    pub session_id: Option<u64>,
    /// The absolute stream offset of the `ESC` that starts the hook.
    pub offset: u64,
    /// The full hook sequence, normalized to end with `ESC \`.
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    DcsEntry,
    DcsIntermediate,
    Payload,
    PayloadEscape,
    Ignore,
    IgnoreEscape,
}

/// Finds `ESC P $ d|f <payload> ESC \` sequences across arbitrarily split
/// reads. `CAN`/`SUB` abort a sequence, `0x9C` is accepted as a terminator, and
/// payloads above [`MAX_HOOK_PAYLOAD`] are dropped.
pub(super) struct DcsScanner {
    state: State,
    /// The absolute offset of the next byte to be fed.
    offset: u64,
    start: u64,
    marker: u8,
    payload: Vec<u8>,
}

impl Default for DcsScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl DcsScanner {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            offset: 0,
            start: 0,
            marker: 0,
            payload: Vec::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Hook>) {
        for &byte in bytes {
            self.step(byte, out);
            self.offset += 1;
        }
    }

    fn step(&mut self, byte: u8, out: &mut Vec<Hook>) {
        match self.state {
            State::Ground => {
                if byte == ESC {
                    self.begin_escape();
                }
            }
            State::Escape => match byte {
                b'P' => self.state = State::DcsEntry,
                ESC => self.begin_escape(),
                _ => self.state = State::Ground,
            },
            State::DcsEntry => match byte {
                b'$' => self.state = State::DcsIntermediate,
                ESC => self.begin_escape(),
                CAN | SUB | ST_8BIT => self.state = State::Ground,
                _ => self.state = State::Ignore,
            },
            State::DcsIntermediate => match byte {
                HEX_MARKER | JSON_MARKER => {
                    self.marker = byte;
                    self.payload.clear();
                    self.state = State::Payload;
                }
                ESC => self.begin_escape(),
                CAN | SUB | ST_8BIT => self.state = State::Ground,
                _ => self.state = State::Ignore,
            },
            State::Payload => match byte {
                ESC => self.state = State::PayloadEscape,
                ST_8BIT => self.finish(out),
                CAN | SUB => self.abort(),
                _ => {
                    if self.payload.len() >= MAX_HOOK_PAYLOAD {
                        log::warn!(
                            "Dropping a shell hook larger than {MAX_HOOK_PAYLOAD} bytes; \
                             it will not be part of a replay"
                        );
                        self.payload = Vec::new();
                        self.state = State::Ignore;
                    } else {
                        self.payload.push(byte);
                    }
                }
            },
            State::PayloadEscape => {
                if byte == b'\\' {
                    self.finish(out);
                } else {
                    self.abort();
                    self.begin_escape_at(self.offset - 1);
                    self.step(byte, out);
                }
            }
            State::Ignore => match byte {
                ESC => self.state = State::IgnoreEscape,
                CAN | SUB | ST_8BIT => self.state = State::Ground,
                _ => {}
            },
            State::IgnoreEscape => {
                if byte == b'\\' {
                    self.state = State::Ground;
                } else {
                    self.begin_escape_at(self.offset - 1);
                    self.step(byte, out);
                }
            }
        }
    }

    fn begin_escape(&mut self) {
        self.begin_escape_at(self.offset);
    }

    fn begin_escape_at(&mut self, offset: u64) {
        self.start = offset;
        self.state = State::Escape;
    }

    fn abort(&mut self) {
        self.payload.clear();
        self.state = State::Ground;
    }

    fn finish(&mut self, out: &mut Vec<Hook>) {
        self.state = State::Ground;
        let payload = std::mem::take(&mut self.payload);
        if let Some((kind, session_id)) = parse_payload(self.marker, &payload) {
            let mut raw = Vec::with_capacity(payload.len() + 6);
            raw.extend_from_slice(&[ESC, b'P', b'$', self.marker]);
            raw.extend_from_slice(&payload);
            raw.extend_from_slice(&[ESC, b'\\']);
            out.push(Hook {
                kind,
                session_id,
                offset: self.start,
                raw,
            });
        }
    }
}

#[derive(Deserialize)]
struct RawHook {
    hook: String,
    #[serde(default)]
    value: Option<RawHookValue>,
}

#[derive(Deserialize, Default)]
struct RawHookValue {
    #[serde(default)]
    session_id: Option<serde_json::Value>,
    #[serde(default)]
    is_subshell: Option<bool>,
}

fn parse_payload(marker: u8, payload: &[u8]) -> Option<(HookKind, Option<u64>)> {
    let decoded;
    let json = if marker == HEX_MARKER {
        decoded = hex::decode(payload).ok()?;
        decoded.as_slice()
    } else {
        payload
    };
    let hook: RawHook = serde_json::from_slice(json).ok()?;
    let value = hook.value.unwrap_or_default();
    let kind = match hook.hook.as_str() {
        "CommandFinished" => HookKind::CommandFinished,
        "Precmd" => HookKind::Precmd,
        "Preexec" => HookKind::Preexec,
        "InitShell" => HookKind::InitShell {
            is_subshell: value.is_subshell.unwrap_or(false),
        },
        "Bootstrapped" => HookKind::Bootstrapped,
        _ => return None,
    };
    let session_id = value.session_id.as_ref().and_then(|id| match id {
        serde_json::Value::Number(number) => number.as_u64(),
        serde_json::Value::String(string) => string.parse().ok(),
        _ => None,
    });
    Some((kind, session_id))
}

#[cfg(test)]
#[path = "scanner_tests.rs"]
mod tests;
