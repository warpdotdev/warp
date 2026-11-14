//! The input performed during a recording, logged so the finished video can be trimmed to it and
//! annotated with it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use instant::Instant;

use crate::{ActionLogEntry, PointerEvent, PointerSession, PointerSink, RecordingGeometry, Target};

/// The input batches performed during a recording.
pub struct RecordingActionLog {
    /// When capture went live; entry offsets are measured from here.
    started_at: Instant,
    target: Target,
    geometry: RecordingGeometry,
    /// Shared with every group's pointer sink, so a drag split across groups is annotated as one
    /// gesture.
    pointer_session: PointerSession,
    entries: Vec<ActionLogEntry>,
}

/// An input batch in progress, to commit to its log once the batch succeeds or discard if it
/// fails.
pub struct ActionGroup {
    start: Duration,
    labels: Vec<String>,
    /// Shared with the batch's pointer sink.
    pointer_events: Arc<Mutex<Vec<PointerEvent>>>,
}

impl RecordingActionLog {
    /// Creates an empty log for a capture of `target` that went live now.
    pub fn new(target: Target, geometry: RecordingGeometry) -> Self {
        Self {
            started_at: Instant::now(),
            target,
            geometry,
            pointer_session: PointerSession::new(),
            entries: Vec::new(),
        }
    }

    /// Starts an input batch, returning the sink the actor reports the batch's pointer events to.
    pub fn begin_group(&self, labels: Vec<String>) -> (ActionGroup, PointerSink) {
        let pointer_events = Arc::new(Mutex::new(Vec::new()));
        let sink = PointerSink {
            started_at: self.started_at,
            recording_target: self.target,
            recording_geometry: self.geometry,
            events: pointer_events.clone(),
            session: self.pointer_session.clone(),
        };
        let group = ActionGroup {
            start: self.started_at.elapsed(),
            labels,
            pointer_events,
        };
        (group, sink)
    }

    /// Commits `group` as finishing now.
    pub fn commit_group(&mut self, group: ActionGroup) {
        let finish_offset = self.started_at.elapsed();
        self.commit_group_at(group, finish_offset);
    }

    /// Commits `group` as finishing `finish_offset` after capture went live, clamped to no earlier
    /// than the group's start.
    pub fn commit_group_at(&mut self, group: ActionGroup, finish_offset: Duration) {
        let pointer_events = std::mem::take(
            &mut *group
                .pointer_events
                .lock()
                .expect("pointer event buffer poisoned"),
        );
        self.entries.push(ActionLogEntry {
            offset: group.start,
            finish_offset: finish_offset.max(group.start),
            labels: group.labels,
            pointer_events,
        });
    }

    /// Discards a failed batch, forgetting its pointer state so a later release can't complete its
    /// abandoned press.
    pub fn discard_group(&self, group: ActionGroup) {
        drop(group);
        self.pointer_session.clear();
    }

    /// The committed batches, in completion order.
    pub fn entries(&self) -> &[ActionLogEntry] {
        &self.entries
    }
}

#[cfg(test)]
#[path = "recording_action_log_tests.rs"]
mod tests;
