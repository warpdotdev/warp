use std::collections::HashSet;

use super::time::ReplicaId;
use super::Operation;

/// An operation queue to defer buffer edits
/// that cannot yet be applied.
#[cfg_attr(test, derive(Clone))]
pub struct DeferredOperations {
    /// The set of replica IDs for which operations
    /// are being deferred.
    replica_ids: HashSet<ReplicaId>,

    /// The set of operations that are being deferred.
    ///
    /// This list must stay ordered by the lamport timestamp
    /// of the edits to avoid starvation. Specifically,
    /// if edit B is causally dependent on edit A, then
    /// lamport(B) > lamport(A). So if the operations are
    /// processed in order, then consumers can guarantee that causally
    /// dependent, deferred ops will not be starved (even if they
    /// are very backed up). On the other hand, if edit B is _not_
    /// causally dependent on edit A, then it doesn't matter whether
    /// we process edit A or edit B first.
    ///
    /// This ordering invariant allows consumers to [`Self::drain`] once
    /// rather than repeatedly drain and apply.
    operations: Vec<Operation>,
}

impl DeferredOperations {
    pub fn new() -> Self {
        Self {
            replica_ids: HashSet::new(),
            operations: vec![],
        }
    }

    /// Empties the operation queue, returning an ordered
    /// vector of operations that were previously deferred.
    pub fn drain(&mut self) -> Vec<Operation> {
        self.replica_ids = HashSet::new();
        std::mem::take(&mut self.operations)
    }

    /// Extends the set of operations that need to be deferred.
    ///
    /// There is intentionally no `push` API for a single element.
    /// Callers are encouraged to collect the operations that need to
    /// deferred and batch-push them because extending the operation queue
    /// is expensive.
    pub fn extend(&mut self, operations: Vec<Operation>) {
        for op in operations {
            self.replica_ids.insert(op.replica_id().clone());
            self.operations.push(op);
        }
        self.operations
            .sort_unstable_by_key(|op| op.lamport_timestamp().clone());
    }

    /// Returns true iff there are operations in the queue that originated from `replica_id`.
    pub fn replica_deferred(&self, replica_id: &ReplicaId) -> bool {
        self.replica_ids.contains(replica_id)
    }
}

#[cfg(test)]
#[path = "deferred_ops_tests.rs"]
mod tests;
