use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;
use std::rc::Rc;

use futures::stream::AbortHandle;
use warpui::{App, Entity};

use super::{substitute_env_vars, FileMCPWatcher, FileMCPWatcherEvent, InFlightParse, PendingScan};
use crate::ai::mcp::MCPProvider;

struct WatcherEventCollector;

impl Entity for WatcherEventCollector {
    type Event = ();
}

fn inert_watcher(
    pending_scans: HashMap<PendingScan, HashSet<(PathBuf, MCPProvider)>>,
) -> FileMCPWatcher {
    FileMCPWatcher {
        file_mcp_tx: async_channel::unbounded().0,
        in_flight_parses: HashMap::new(),
        next_parse_generation: 0,
        home_provider_watchers: HashMap::new(),
        project_repo_watchers: HashSet::new(),
        pending_scans,
    }
}

#[test]
fn pending_scan_completes_once_after_every_source_settles() {
    let repo = PathBuf::from("/work/repository");
    let claude = (repo.join(".mcp.json"), MCPProvider::Claude);
    let codex = (repo.join(".codex/config.toml"), MCPProvider::Codex);
    let scan = PendingScan::InitialGlobal;

    App::test((), |mut app| async move {
        let watcher = app.add_singleton_model(|_| {
            inert_watcher(HashMap::from([(
                scan.clone(),
                HashSet::from([claude.clone(), codex.clone()]),
            )]))
        });
        let completed = Rc::new(RefCell::new(Vec::new()));
        let collector = app.add_model(|_| WatcherEventCollector);
        collector.update(&mut app, {
            let completed = completed.clone();
            let watcher = watcher.clone();
            move |_, ctx| {
                ctx.subscribe_to_model(&watcher, move |_, event, _| {
                    if let FileMCPWatcherEvent::ScanComplete(scan) = event {
                        completed.borrow_mut().push(scan.clone());
                    }
                });
            }
        });

        for _ in 0..2 {
            watcher.update(&mut app, |watcher, ctx| {
                watcher.settle_pending_source(&claude.0, claude.1, ctx);
            });
            assert!(completed.borrow().is_empty());
        }
        watcher.update(&mut app, |watcher, ctx| {
            watcher.settle_pending_source(&codex.0, codex.1, ctx);
        });

        assert_eq!(*completed.borrow(), vec![scan]);
        watcher.read(&app, |watcher, _| {
            assert!(watcher.pending_scans.is_empty());
        });
    });
}

#[test]
fn only_the_current_parse_generation_can_settle_a_source() {
    let config_path = PathBuf::from("/tmp/.mcp.json");
    let key = (config_path, MCPProvider::Warp);
    let (abort_handle, _) = AbortHandle::new_pair();
    let mut watcher = inert_watcher(HashMap::new());
    watcher.in_flight_parses.insert(
        key.clone(),
        InFlightParse {
            generation: 2,
            abort_handle,
        },
    );

    assert!(!watcher.take_current_in_flight_parse(&key, 1));
    assert!(watcher.in_flight_parses.contains_key(&key));
    assert!(watcher.take_current_in_flight_parse(&key, 2));
    assert!(!watcher.in_flight_parses.contains_key(&key));
}

fn cleanup_env_vars(vars: &[&str]) {
    for var in vars {
        env::remove_var(var);
    }
}

#[test]
fn test_substitute_env_vars_success() {
    let test_vars = ["FOO", "BAZ", "REPEATED"];

    // Setup environment variables
    env::set_var("FOO", "bar");
    env::set_var("BAZ", "qux");
    env::set_var("REPEATED", "value");

    // Test 1: Single variable substitution
    let input = r#"{"key": "${FOO}"}"#;
    let result = substitute_env_vars(input).expect("Single variable substitution should succeed");
    assert_eq!(
        result, r#"{"key": "bar"}"#,
        "Single variable FOO should be replaced with 'bar'"
    );

    // Test 2: Multiple different variables
    let input = r#"{"key": "${FOO}", "other": "${BAZ}"}"#;
    let result = substitute_env_vars(input).expect("Multiple variable substitution should succeed");
    assert_eq!(
        result, r#"{"key": "bar", "other": "qux"}"#,
        "Multiple variables FOO and BAZ should be replaced"
    );

    // Test 3: Multiple occurrences of same variable
    let input = r#"{"a": "${REPEATED}", "b": "${REPEATED}", "c": "prefix_${REPEATED}_suffix"}"#;
    let result = substitute_env_vars(input).expect("Repeated variable substitution should succeed");
    assert_eq!(
        result, r#"{"a": "value", "b": "value", "c": "prefix_value_suffix"}"#,
        "All occurrences of REPEATED should be replaced with 'value', including within context"
    );

    // Cleanup
    cleanup_env_vars(&test_vars);
}

#[test]
fn test_substitute_env_vars_missing_or_empty() {
    // Test 1: Missing variable
    // Ensure MISSING_VAR is not set
    env::remove_var("MISSING_VAR");

    let input = r#"{"key": "${MISSING_VAR}"}"#;
    let result = substitute_env_vars(input);
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Missing or empty environment variable: MISSING_VAR"),
        "Error message should mention MISSING_VAR, got: {err_msg}"
    );

    // Test 2: Empty variable
    env::set_var("EMPTY_VAR", "");

    let input = r#"{"key": "${EMPTY_VAR}"}"#;
    let result = substitute_env_vars(input);
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Missing or empty environment variable: EMPTY_VAR"),
        "Error message should mention EMPTY_VAR, got: {err_msg}"
    );

    // Cleanup
    cleanup_env_vars(&["EMPTY_VAR"]);
}
