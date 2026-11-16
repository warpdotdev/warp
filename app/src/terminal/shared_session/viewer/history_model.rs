use warpui::Entity;

use crate::terminal::HistoryEntry;

/// Responsible for managing the history of a shared session for a viewer.
#[derive(Default)]
pub struct SharedSessionHistoryModel {
    entries: Vec<HistoryEntry>,
}

impl SharedSessionHistoryModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn entries(&self) -> impl Iterator<Item = &HistoryEntry> {
        self.entries.iter()
    }

    pub fn push(&mut self, entry: HistoryEntry) {
        self.entries.push(entry);
    }
}

impl Entity for SharedSessionHistoryModel {
    type Event = ();
}
