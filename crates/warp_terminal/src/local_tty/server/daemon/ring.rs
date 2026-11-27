//! Per-session output history: a bounded ring buffer addressed by absolute
//! stream offsets, plus the hook bookkeeping needed to build a replay.

use std::collections::VecDeque;

use super::scanner::{DcsScanner, Hook, HookKind};

/// Inserted into a replay when the start of the current command was evicted.
pub(super) const TRUNCATION_MARKER: &[u8] = b"\r\n[output truncated]\r\n";

/// A byte ring buffer that remembers the absolute offset of every byte ever
/// pushed. Storage grows lazily up to `capacity`.
pub(super) struct Ring {
    buf: VecDeque<u8>,
    capacity: usize,
    total: u64,
}

impl Ring {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: VecDeque::new(),
            capacity,
            total: 0,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.total += bytes.len() as u64;
        if bytes.len() >= self.capacity {
            self.buf.clear();
            self.buf.extend(&bytes[bytes.len() - self.capacity..]);
            return;
        }
        let overflow = (self.buf.len() + bytes.len()).saturating_sub(self.capacity);
        self.buf.drain(..overflow);
        self.buf.extend(bytes);
    }

    /// The absolute offset one past the last byte pushed.
    #[cfg(test)]
    pub fn total(&self) -> u64 {
        self.total
    }

    /// The absolute offset of the oldest retained byte.
    pub fn start(&self) -> u64 {
        self.total - self.buf.len() as u64
    }

    /// Copies every retained byte from `offset` onwards, or `None` if `offset`
    /// is outside the retained range.
    pub fn copy_from(&self, offset: u64) -> Option<Vec<u8>> {
        if offset < self.start() || offset > self.total {
            return None;
        }
        let skip = (offset - self.start()) as usize;
        Some(self.buf.range(skip..).copied().collect())
    }
}

#[derive(Debug, Clone)]
struct RawHook {
    offset: u64,
    raw: Vec<u8>,
}

/// The bytes to send to a client on attach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Replay {
    pub bytes: Vec<u8>,
    pub has_bootstrap_preamble: bool,
}

/// A prompt boundary: the `CommandFinished` hook that immediately precedes a
/// `Precmd` (if any) and the `Precmd` itself.
#[derive(Debug, Clone)]
struct Prompt {
    /// Where a replay of this prompt starts: the `CommandFinished` offset, or
    /// the `Precmd` offset if there was none.
    start: u64,
    precmd_offset: u64,
    /// The raw `CommandFinished` (if any) followed by the raw `Precmd`.
    raw: Vec<u8>,
}

/// The output history of one session.
pub(super) struct History {
    ring: Ring,
    scanner: DcsScanner,
    hook_session_id: u64,
    hooks: Vec<Hook>,
    prompt_starts: VecDeque<u64>,
    pending_command_finished: Option<RawHook>,
    last_prompt: Option<Prompt>,
    last_preexec: Option<RawHook>,
    pending_init_shell: Option<RawHook>,
    preamble: Option<RawHook>,
    replay_from: u64,
}

impl History {
    pub fn new(capacity: usize, hook_session_id: u64) -> Self {
        Self {
            ring: Ring::new(capacity),
            scanner: DcsScanner::new(),
            hook_session_id,
            hooks: Vec::new(),
            prompt_starts: VecDeque::new(),
            pending_command_finished: None,
            last_prompt: None,
            last_preexec: None,
            pending_init_shell: None,
            preamble: None,
            replay_from: 0,
        }
    }

    /// Appends PTY output to the history.
    pub fn record(&mut self, bytes: &[u8]) {
        self.ring.push(bytes);
        self.scanner.feed(bytes, &mut self.hooks);
        for hook in std::mem::take(&mut self.hooks) {
            self.on_hook(hook);
        }
        let start = self.ring.start();
        while self.prompt_starts.front().is_some_and(|o| *o < start) {
            self.prompt_starts.pop_front();
        }
    }

    fn on_hook(&mut self, hook: Hook) {
        let raw = RawHook {
            offset: hook.offset,
            raw: hook.raw,
        };
        let own_session = hook.session_id == Some(self.hook_session_id);
        match hook.kind {
            HookKind::CommandFinished | HookKind::Precmd | HookKind::Preexec if !own_session => {}
            HookKind::CommandFinished => self.pending_command_finished = Some(raw),
            HookKind::Precmd => {
                let prompt = match self.pending_command_finished.take() {
                    Some(mut command_finished) => {
                        command_finished.raw.extend_from_slice(&raw.raw);
                        Prompt {
                            start: command_finished.offset,
                            precmd_offset: raw.offset,
                            raw: command_finished.raw,
                        }
                    }
                    None => Prompt {
                        start: raw.offset,
                        precmd_offset: raw.offset,
                        raw: raw.raw,
                    },
                };
                self.prompt_starts.push_back(prompt.start);
                self.last_prompt = Some(prompt);
            }
            HookKind::Preexec => {
                self.pending_command_finished = None;
                self.last_preexec = Some(raw);
            }
            HookKind::InitShell { is_subshell: false } if own_session => {
                self.preamble = None;
                self.pending_init_shell = Some(raw);
            }
            HookKind::InitShell { .. } => {}
            HookKind::Bootstrapped => {
                if hook.session_id.is_some_and(|id| id != self.hook_session_id) {
                    return;
                }
                if let Some(mut init_shell) = self.pending_init_shell.take() {
                    init_shell.raw.extend_from_slice(&raw.raw);
                    self.preamble = Some(init_shell);
                }
            }
        }
    }

    /// Records that the client detached: the next replay starts at the last
    /// prompt seen so far, including the `CommandFinished` right before it.
    pub fn mark_detached(&mut self) {
        self.replay_from = self.last_prompt.as_ref().map_or(0, |prompt| prompt.start);
    }

    pub fn has_bootstrap_preamble(&self) -> bool {
        self.preamble.is_some()
    }

    /// Builds the replay: the bootstrap preamble followed by the body, which is
    /// the first of:
    /// 1. everything since `replay_from`, if still retained;
    /// 2. everything since the oldest retained prompt;
    /// 3. the last `CommandFinished`, `Precmd` and `Preexec` hooks, a
    ///    truncation marker, and every retained byte.
    ///
    /// A prompt starts at the `CommandFinished` hook right before its `Precmd`
    /// so the restored block list leaves the bootstrap block.
    pub fn replay(&self) -> Replay {
        let ring_start = self.ring.start();
        let mut prefix = Vec::new();
        let body_start = if self.replay_from >= ring_start {
            self.replay_from
        } else if let Some(oldest) = self.prompt_starts.front() {
            *oldest
        } else {
            let precmd_offset = self.last_prompt.as_ref().map(|prompt| prompt.precmd_offset);
            if let Some(prompt) = &self.last_prompt {
                prefix.extend_from_slice(&prompt.raw);
            }
            if let Some(preexec) = &self.last_preexec
                && precmd_offset.is_none_or(|offset| preexec.offset > offset)
            {
                prefix.extend_from_slice(&preexec.raw);
            }
            prefix.extend_from_slice(TRUNCATION_MARKER);
            ring_start
        };
        let body = self.ring.copy_from(body_start).unwrap_or_default();

        let mut bytes = Vec::new();
        if let Some(preamble) = &self.preamble
            && !(prefix.is_empty() && body_start <= preamble.offset)
        {
            bytes.extend_from_slice(&preamble.raw);
        }
        bytes.extend_from_slice(&prefix);
        bytes.extend_from_slice(&body);
        Replay {
            bytes,
            has_bootstrap_preamble: self.preamble.is_some(),
        }
    }
}

#[cfg(test)]
#[path = "ring_tests.rs"]
mod tests;
