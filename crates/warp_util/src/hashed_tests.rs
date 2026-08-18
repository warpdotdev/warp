use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;

use super::Hashed;

#[test]
fn hash_matches_build_hasher() {
    let build_hasher = RandomState::new();
    let hashed = Hashed::new("key".to_string(), &build_hasher);
    assert_eq!(hashed.hash(), build_hasher.hash_one("key".to_string()));
}

#[test]
fn derefs_and_compares_as_the_key() {
    let build_hasher = RandomState::new();
    let hashed = Hashed::new("key".to_string(), &build_hasher);

    assert_eq!(hashed.len(), 3);
    assert_eq!(hashed, "key".to_string());
    assert_eq!(hashed.key(), "key");
    assert_eq!(hashed.into_key(), "key".to_string());
}

#[test]
fn equality_ignores_cached_hash() {
    let one = Hashed::new(7u32, &RandomState::new());
    let mut another = Hashed::new(7u32, &RandomState::new());
    another.rehash(&RandomState::new());

    assert_eq!(one, another);
}

#[test]
fn looks_up_key_in_map() {
    let mut map: HashMap<u32, &str> = HashMap::new();
    map.insert(7, "seven");

    let hashed = Hashed::new(7u32, map.hasher());
    assert_eq!(map.get(hashed.key()), Some(&"seven"));
}

#[test]
fn rehash_updates_cached_hash() {
    let build_hasher = RandomState::new();
    let other_build_hasher = RandomState::new();

    let mut hashed = Hashed::new(7u32, &build_hasher);
    hashed.rehash(&other_build_hasher);

    assert_eq!(hashed.hash(), other_build_hasher.hash_one(7u32));
}
