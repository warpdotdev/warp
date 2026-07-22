use std::collections::HashSet;

use chrono::{Local, TimeZone};

use super::*;

#[test]
fn round_trip_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let mut state = LocalAutomationsRunState::default();
    let entry = state.entry_mut("/tmp/a.toml");
    entry.last_scheduled_fire_at = Some(Local.with_ymd_and_hms(2026, 7, 22, 9, 0, 0).unwrap());
    entry.missed_count = 2;
    entry.last_missed_at = Some(Local.with_ymd_and_hms(2026, 7, 21, 9, 0, 0).unwrap());
    state.save_to_path(&path).unwrap();

    let loaded = LocalAutomationsRunState::load_from_path(&path);
    assert_eq!(loaded.version, 1);
    let e = loaded.entry("/tmp/a.toml").unwrap();
    assert_eq!(e.missed_count, 2);
    assert_eq!(
        e.last_scheduled_fire_at.unwrap(),
        Local.with_ymd_and_hms(2026, 7, 22, 9, 0, 0).unwrap()
    );
}

#[test]
fn corrupt_file_starts_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    std::fs::write(&path, "not-json{{{").unwrap();
    let loaded = LocalAutomationsRunState::load_from_path(&path);
    assert!(loaded.by_path.is_empty());
}

#[test]
fn missing_file_starts_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.json");
    let loaded = LocalAutomationsRunState::load_from_path(&path);
    assert!(loaded.by_path.is_empty());
}

#[test]
fn prune_to_paths() {
    let mut state = LocalAutomationsRunState::default();
    state.entry_mut("keep.toml");
    state.entry_mut("drop.toml");
    let mut keep = HashSet::new();
    keep.insert("keep.toml".to_string());
    state.prune_to_paths(&keep);
    assert!(state.by_path.contains_key("keep.toml"));
    assert!(!state.by_path.contains_key("drop.toml"));
}
