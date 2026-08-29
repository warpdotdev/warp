use std::collections::{HashMap, HashSet};

use super::parser::PaneId;

/// Tracks tmux pane identities independently of how the `-CC` byte stream is transported.
#[derive(Debug, Default, Clone)]
pub struct PaneRegistry {
    panes: HashSet<PaneId>,
    focused: Option<PaneId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosePlan {
    KillPane,
    TearDownSession,
    DetachClient,
    UnknownPane,
}

/// Network or PTY loss detaches the control client only. Explicit user close of the last
/// pane may tear down the tmux session; transport EOF must not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlClientLoss {
    TransportEof,
    ExplicitClose,
}

impl ControlClientLoss {
    pub fn close_plan(self, last_pane: bool) -> ClosePlan {
        match self {
            Self::TransportEof => ClosePlan::DetachClient,
            // Product same-PTY sessions must leave tmux alive; harness teardown is separate.
            Self::ExplicitClose if last_pane => ClosePlan::DetachClient,
            Self::ExplicitClose => ClosePlan::KillPane,
        }
    }
}

/// Independent output slots for tmux panes that share one PTY owner.
#[derive(Debug, Default, Clone)]
pub struct TmuxViewSlots {
    outputs: HashMap<PaneId, Vec<u8>>,
}

impl TmuxViewSlots {
    pub fn deliver(&mut self, pane_id: PaneId, bytes: &[u8]) {
        self.outputs
            .entry(pane_id)
            .or_default()
            .extend_from_slice(bytes);
    }

    pub fn view_count(&self) -> usize {
        self.outputs.len()
    }

    pub fn output(&self, pane_id: &PaneId) -> Option<&[u8]> {
        self.outputs.get(pane_id).map(Vec::as_slice)
    }
}

impl PaneRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, pane_id: PaneId) {
        if self.focused.is_none() {
            self.focused = Some(pane_id.clone());
        }
        self.panes.insert(pane_id);
    }

    pub fn contains(&self, pane_id: &PaneId) -> bool {
        self.panes.contains(pane_id)
    }

    pub fn len(&self) -> usize {
        self.panes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.panes.is_empty()
    }

    pub fn focused(&self) -> Option<&PaneId> {
        self.focused.as_ref()
    }

    pub fn focus(&mut self, pane_id: &PaneId) -> bool {
        if !self.panes.contains(pane_id) {
            return false;
        }
        self.focused = Some(pane_id.clone());
        true
    }

    pub fn should_deliver_output(&self, pane_id: &PaneId) -> bool {
        self.panes.contains(pane_id)
    }

    pub fn close_plan(&self, pane_id: &PaneId) -> ClosePlan {
        if !self.panes.contains(pane_id) {
            return ClosePlan::UnknownPane;
        }
        if self.panes.len() <= 1 {
            ClosePlan::DetachClient
        } else {
            ClosePlan::KillPane
        }
    }

    pub fn unregister(&mut self, pane_id: &PaneId) -> ClosePlan {
        let plan = self.close_plan(pane_id);
        if matches!(plan, ClosePlan::UnknownPane) {
            return plan;
        }
        self.panes.remove(pane_id);
        if self.focused.as_ref() == Some(pane_id) {
            self.focused = self.panes.iter().next().cloned();
        }
        plan
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
