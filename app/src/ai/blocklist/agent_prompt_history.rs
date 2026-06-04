//! In-memory store of prompts the user has submitted to the agent, used by NLD
//! input classification for prompt-history matching.
//!
//! This mirrors the capped, FIFO `commands` history but for agent prompts. It is
//! intentionally kept separate from [`crate::terminal::History`] because prompts
//! are global to the user rather than shell-host/session scoped.

use std::collections::{HashSet, VecDeque};

use chrono::{DateTime, Local};
use warpui::{Entity, SingletonEntity};

/// The maximum number of prompts retained in memory. Mirrors the
/// `AGENT_PROMPTS_COUNT_LIMIT` write cap on the `agent_prompts` table so the
/// per-keystroke matching cost stays bounded over a long session.
const MAX_IN_MEMORY_AGENT_PROMPTS: usize = 2_000;

/// A single prompt the user submitted to the agent.
#[derive(Clone, Debug)]
pub struct AgentPrompt {
    pub text: String,
    pub start_ts: DateTime<Local>,
}

/// Singleton model holding a recency-ordered store of recent agent prompts,
/// deduplicated to the most-recent occurrence of each distinct prompt text
/// (mirroring the in-memory `commands` dedup).
///
/// Entries are stored newest-first (most-recent at the front), matching the
/// `id DESC` startup read. Iterate via [`Self::iter_recent`] to walk
/// newest-first.
#[derive(Default, Debug)]
pub struct AgentPromptHistory {
    prompts: VecDeque<AgentPrompt>,
}

impl AgentPromptHistory {
    /// Builds the store from prompts persisted across restarts in newest first order
    pub fn new(prompts: Vec<(String, DateTime<Local>)>) -> Self {
        let mut seen: HashSet<&str> = HashSet::with_capacity(prompts.len());
        let keep_entry: Vec<bool> = prompts
            .iter()
            .map(|(text, _)| seen.insert(text.as_str()))
            .collect();
        drop(seen);

        let mut history = Self {
            prompts: VecDeque::with_capacity(prompts.len().min(MAX_IN_MEMORY_AGENT_PROMPTS)),
        };
        for ((text, start_ts), keep) in prompts.into_iter().zip(keep_entry) {
            if keep {
                history.prompts.push_back(AgentPrompt { text, start_ts });
            }
        }
        history
    }

    /// Appends a newly-submitted prompt as the most-recent entry (at the front)
    pub fn append(&mut self, text: String, start_ts: DateTime<Local>) {
        // Drop any earlier occurrence of the same text. Prompt submissions
        // happen at human speed, so this linear scan is not on a hot path.
        if let Some(existing) = self.prompts.iter().position(|prompt| prompt.text == text) {
            self.prompts.remove(existing);
        }
        // Evict the oldest entry (at the back) when at capacity.
        if self.prompts.len() == MAX_IN_MEMORY_AGENT_PROMPTS {
            self.prompts.pop_back();
        }
        // The newest prompt goes to the front.
        self.prompts.push_front(AgentPrompt { text, start_ts });
    }

    /// Iterates over prompts newest-first, so the first match encountered is the
    /// most-recent matching prompt. Entries are stored newest-first (most recent
    /// at the front), so this is a plain forward iteration.
    pub fn iter_recent(&self) -> impl Iterator<Item = &AgentPrompt> {
        self.prompts.iter()
    }

    /// Clears all in-memory prompt history. Used on logout, when the backing DB
    /// is removed, so in-memory state does not leak across accounts.
    pub fn reset(&mut self) {
        self.prompts.clear();
    }
}

impl Entity for AgentPromptHistory {
    type Event = ();
}

impl SingletonEntity for AgentPromptHistory {}
