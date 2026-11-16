use std::sync::{Arc, Mutex};

use bounded_vec_deque::BoundedVecDeque;

/// Maximum number of log lines to retain in the tail buffer.
const LOG_TAIL_MAX_LINES: usize = 5;

/// Maximum number of characters to include when draining the log buffer
/// for telemetry payloads.
const LOG_TAIL_MAX_CHARS: usize = 2048;

/// A shared buffer that retains the last [`LOG_TAIL_MAX_LINES`] lines
/// from the remote server proxy. Used to attach server-side context to
/// telemetry when the connection fails.
#[derive(Clone, Debug)]
pub struct RemoteServerLog(Arc<Mutex<BoundedVecDeque<String>>>);

impl RemoteServerLog {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(BoundedVecDeque::new(
            LOG_TAIL_MAX_LINES,
        ))))
    }

    pub(crate) fn push(&self, line: String) {
        if let Ok(mut buf) = self.0.lock() {
            buf.push_back(line);
        }
    }

    /// Drains the buffer and returns the joined lines, or `None` if empty.
    /// Truncates to [`LOG_TAIL_MAX_CHARS`] chars (keeping the tail, which
    /// is the most useful context for diagnosing why the proxy died).
    pub fn drain(&self) -> Option<String> {
        let lines: Vec<String> = self.0.lock().ok()?.drain(..).collect();
        if lines.is_empty() {
            return None;
        }
        let joined = lines.join("\n");
        if joined.chars().count() > LOG_TAIL_MAX_CHARS {
            let tail: String = joined
                .chars()
                .rev()
                .take(LOG_TAIL_MAX_CHARS)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            Some(format!("…{tail}"))
        } else {
            Some(joined)
        }
    }
}
