use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Default, Clone)]
pub(super) struct ChangedFiles {
    pub(super) deletions: HashSet<PathBuf>,
    pub(super) upsertions: HashSet<PathBuf>,
}

impl ChangedFiles {
    pub(super) fn is_empty(&self) -> bool {
        self.deletions.is_empty() && self.upsertions.is_empty()
    }

    pub(super) fn deletions(&self) -> &HashSet<PathBuf> {
        &self.deletions
    }

    /// Merges a subsequent set of file changes into the current set.
    pub(super) fn merge_subsequent(&mut self, mut subsequent_changes: Self) {
        for path in subsequent_changes.deletions.drain() {
            if self.upsertions.contains(&path) {
                self.upsertions.remove(&path);
            }
            self.deletions.insert(path);
        }

        for path in subsequent_changes.upsertions.drain() {
            if self.deletions.contains(&path) {
                self.deletions.remove(&path);
            }
            self.upsertions.insert(path);
        }
    }

    // Add paths to this changed files set based on whether they currently exist on the file system.
    pub(super) async fn add_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            if path.exists() {
                self.upsertions.insert(path);
            } else {
                self.deletions.insert(path);
            }
        }
    }
}

#[cfg(test)]
#[path = "changed_files_tests.rs"]
mod tests;
