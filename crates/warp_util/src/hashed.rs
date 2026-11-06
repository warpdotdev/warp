use std::borrow::Borrow;
use std::fmt;
use std::hash::{BuildHasher, Hash, Hasher};
use std::ops::Deref;

/// A key stored alongside its precomputed hash.
///
/// Useful for keys that are looked up repeatedly in the same hash container:
/// the hash is computed once and can be reused with APIs that accept a raw hash
/// (for example `hashbrown`'s `raw_entry().from_hash(..)`), avoiding a rehash
/// on every lookup.
///
/// The hash is only meaningful for the [`BuildHasher`] it was computed with, so
/// a `Hashed` must only be used against the container whose hasher produced it.
/// Use [`Hashed::rehash`] when the key moves to a container with a different
/// hasher.
#[derive(Clone, Copy)]
pub struct Hashed<K> {
    key: K,
    hash: u64,
}

impl<K: Hash> Hashed<K> {
    pub fn new(key: K, build_hasher: &impl BuildHasher) -> Self {
        let hash = build_hasher.hash_one(&key);
        Self { key, hash }
    }

    /// Recomputes the hash for a different [`BuildHasher`].
    pub fn rehash(&mut self, build_hasher: &impl BuildHasher) {
        self.hash = build_hasher.hash_one(&self.key);
    }
}

impl<K> Hashed<K> {
    pub fn key(&self) -> &K {
        &self.key
    }

    pub fn hash(&self) -> u64 {
        self.hash
    }

    pub fn into_key(self) -> K {
        self.key
    }
}

impl<K> Deref for Hashed<K> {
    type Target = K;

    fn deref(&self) -> &K {
        &self.key
    }
}

impl<K> Borrow<K> for Hashed<K> {
    fn borrow(&self) -> &K {
        &self.key
    }
}

impl<K> AsRef<K> for Hashed<K> {
    fn as_ref(&self) -> &K {
        &self.key
    }
}

// Equality and hashing intentionally ignore the cached hash so that a `Hashed`
// behaves exactly like the key it wraps, regardless of which hasher produced
// the cached value.
impl<K: PartialEq> PartialEq for Hashed<K> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl<K: Eq> Eq for Hashed<K> {}

impl<K: PartialEq> PartialEq<K> for Hashed<K> {
    fn eq(&self, other: &K) -> bool {
        self.key == *other
    }
}

impl<K: Hash> Hash for Hashed<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

impl<K: fmt::Debug> fmt::Debug for Hashed<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hashed")
            .field("key", &self.key)
            .field("hash", &self.hash)
            .finish()
    }
}

#[cfg(test)]
#[path = "hashed_tests.rs"]
mod tests;
