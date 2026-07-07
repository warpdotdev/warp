use std::cmp::{self, Ordering};
use std::collections::HashMap;
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const BASE_REPLICA_ID: &str = "0";

/// A unique ID assigned to every peer in the system.
#[derive(Clone, Hash, Eq, PartialEq, Debug, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ReplicaId(Rc<String>);

impl ReplicaId {
    pub fn new(id: impl ToString) -> Self {
        let id = Self(Rc::new(id.to_string()));
        debug_assert!(id.0.as_str() != BASE_REPLICA_ID);
        id
    }

    /// Creates a sufficiently random id.
    pub fn random() -> Self {
        Self::new(Uuid::new_v4())
    }

    /// A sentinel replica ID for the base text.
    /// The base text insertion is considered to be a replica-less edit.
    pub fn base_replica_id() -> Self {
        Self(Rc::new(BASE_REPLICA_ID.to_string()))
    }
}

impl std::fmt::Display for ReplicaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

/// A bare [lamport timestamp](https://en.wikipedia.org/wiki/Lamport_timestamp).
/// Prefer to use the full [`Lamport`] type unless the replica ID is known / fixed.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Ord, PartialOrd, Serialize, Deserialize,
)]
pub struct LamportValue(usize);

impl LamportValue {
    fn next(&self) -> Self {
        LamportValue(self.0 + 1)
    }
}

impl From<usize> for LamportValue {
    fn from(val: usize) -> Self {
        LamportValue(val)
    }
}

/// A [lamport timestamp](https://en.wikipedia.org/wiki/Lamport_timestamp).
///
/// Along with the bare [`LamportValue`], we also store the replica ID to identify
/// the origin of the event associated to this timestamp. This allows us to achieve
/// a total ordering of events (see https://en.wikipedia.org/wiki/Lamport_timestamp#Implications).
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct Lamport {
    pub replica_id: ReplicaId,
    pub value: LamportValue,
}

impl Lamport {
    pub fn new(replica_id: ReplicaId) -> Self {
        Self {
            value: LamportValue::default(),
            replica_id,
        }
    }

    pub fn tick(&mut self) -> Self {
        let timestamp = self.clone();
        self.value.0 += 1;
        timestamp
    }

    pub fn observe(&mut self, timestamp: &Self) {
        self.value = cmp::max(self.value, timestamp.value).next();
    }

    pub fn replica_id(&self) -> ReplicaId {
        self.replica_id.clone()
    }
}

impl Ord for Lamport {
    /// When comparing lamport timestamps, we break ties using the replica ID
    /// to get a total ordering.
    fn cmp(&self, other: &Self) -> Ordering {
        (self.value, &self.replica_id).cmp(&(other.value, &other.replica_id))
    }
}

impl PartialOrd for Lamport {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A [version vector](https://en.wikipedia.org/wiki/Version_vector) to track
/// the latest lamport timestamp seen for every peer in the system.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Global(Rc<HashMap<ReplicaId, LamportValue>>);

impl Default for Global {
    fn default() -> Self {
        Self::new()
    }
}

impl Global {
    pub fn new() -> Self {
        Global(Rc::new(HashMap::new()))
    }

    pub fn get(&self, replica_id: &ReplicaId) -> LamportValue {
        self.0.get(replica_id).copied().unwrap_or_default()
    }

    pub fn observe(&mut self, timestamp: &Lamport) {
        let map = Rc::make_mut(&mut self.0);
        let value = map.entry(timestamp.replica_id.clone()).or_default();
        *value = cmp::max(*value, timestamp.value);
    }

    pub fn observe_all(&mut self, other: &Self) {
        for (replica_id, value) in other.0.as_ref() {
            self.observe(&Lamport {
                replica_id: replica_id.clone(),
                value: *value,
            });
        }
    }

    pub fn observed(&self, timestamp: &Lamport) -> bool {
        self.get(&timestamp.replica_id) >= timestamp.value
    }

    pub fn changed_since(&self, other: &Self) -> bool {
        self.0
            .iter()
            .any(|(replica_id, value)| *value > other.get(replica_id))
    }
}

impl PartialOrd for Global {
    /// Returns Some(Ordering) iff the ordering is conclusive.
    /// Otherwise, the version vectors being compared are
    /// concurrent and so the ordering is undefined ([`None`]).
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let mut global_ordering = Ordering::Equal;

        for replica_id in self.0.keys().chain(other.0.keys()) {
            let ordering = self.get(replica_id).cmp(&other.get(replica_id));
            if ordering != Ordering::Equal {
                if global_ordering == Ordering::Equal {
                    global_ordering = ordering;
                } else if ordering != global_ordering {
                    return None;
                }
            }
        }

        Some(global_ordering)
    }
}
