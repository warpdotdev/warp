use std::collections::HashSet;
use std::path::PathBuf;

use uuid::Uuid;

use super::{
    InitialGlobalMcpReadiness, InitialGlobalScanCohort, home_subdir_to_watch,
    plan_initial_global_scan,
};
use crate::ai::mcp::MCPProvider;

#[test]
fn complete_is_idempotent_and_late_safe() {
    let latch = InitialGlobalMcpReadiness::pending();
    assert!(!latch.is_complete());
    assert_eq!(latch.result(), None);

    let first = vec![Uuid::nil()];
    latch.complete(first.clone());
    latch.complete(vec![Uuid::new_v4()]);

    assert!(latch.is_complete());
    assert_eq!(latch.result(), Some(first.clone()));
    assert_eq!(
        futures::executor::block_on(latch.wait()),
        first,
        "a waiter attached after completion must still see the frozen set"
    );
}

#[test]
fn cohort_emits_once_when_the_last_source_settles() {
    let source = (PathBuf::from("/tmp/.mcp.json"), MCPProvider::Warp);
    let mut cohort = InitialGlobalScanCohort::from_pending(HashSet::from([source.clone()]));
    assert!(!cohort.try_complete());
    assert!(cohort.remove(&source));
    assert!(cohort.try_complete());
    assert!(cohort.has_emitted());
    assert!(!cohort.try_complete());
}

#[test]
fn existing_subdir_provider_joins_scan_cohort_without_direct_parse() {
    let home = PathBuf::from("/home/test");
    let plan = plan_initial_global_scan(Some(home.clone()), None, |path| {
        path == home.join(".codex") || path == home.join(".agents")
    });
    let codex = (home.join(".codex/config.toml"), MCPProvider::Codex);
    assert!(
        plan.pending.contains(&codex),
        "an existing Codex subdir must be owed by the initial scan"
    );
    assert!(
        plan.watch_subdirs
            .iter()
            .any(|(path, _)| path == &home.join(".codex")),
        "an existing Codex subdir should be watched rather than parsed directly"
    );
    assert!(
        !plan
            .direct_parses
            .iter()
            .any(|(_, _, provider)| *provider == MCPProvider::Codex),
        "watching an existing Codex subdir must not also schedule a racing direct parse"
    );
}

#[test]
fn missing_subdir_provider_joins_scan_cohort_and_direct_parses() {
    let home = PathBuf::from("/home/test");
    let plan = plan_initial_global_scan(Some(home.clone()), None, |_| false);
    let codex = (home.join(".codex/config.toml"), MCPProvider::Codex);
    assert!(plan.pending.contains(&codex));
    assert!(
        plan.direct_parses
            .iter()
            .any(|(path, _, provider)| *provider == MCPProvider::Codex && path == &codex.0)
    );
    assert!(plan.watch_subdirs.is_empty());
}

#[test]
fn claude_home_config_is_always_a_direct_parse() {
    let home = PathBuf::from("/home/test");
    let plan = plan_initial_global_scan(Some(home.clone()), None, |_| true);
    assert_eq!(home_subdir_to_watch(MCPProvider::Claude), None);
    assert!(
        plan.direct_parses
            .iter()
            .any(|(path, _, provider)| *provider == MCPProvider::Claude
                && path == &home.join(".claude.json"))
    );
}

#[test]
fn warp_config_is_pending_and_directly_parsed_without_home() {
    let config = PathBuf::from("/tmp/.warp/.mcp.json");
    let root = PathBuf::from("/tmp/.warp");
    let plan = plan_initial_global_scan(None, Some((config.clone(), root.clone())), |_| true);
    assert_eq!(
        plan.pending,
        HashSet::from([(config.clone(), MCPProvider::Warp)])
    );
    assert_eq!(plan.direct_parses, vec![(config, root, MCPProvider::Warp)]);
    assert!(plan.watch_subdirs.is_empty());
}
