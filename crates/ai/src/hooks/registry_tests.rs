use std::sync::Arc;

use super::*;
use crate::hooks::model::{HookBoundary, HookSourceKind};
use crate::hooks::test_util::{
    RecordingAdapter, active_binding, test_binding, test_source_id, test_source_id_of_kind,
};

#[test]
fn a_new_registry_has_an_empty_snapshot_at_revision_zero() {
    let registry = HookRegistry::new();
    let snapshot = registry.snapshot();
    assert_eq!(snapshot.revision, 0);
    assert!(snapshot.is_empty());
    assert!(snapshot.diagnostics.is_empty());
    assert!(
        registry
            .bindings_for(HookBoundary::ToolPre, HookExecutionHost::Local)
            .is_empty()
    );
}

#[test]
fn installing_a_snapshot_bumps_the_revision_and_replaces_the_set() {
    let adapter: Arc<dyn HookFormatAdapter> = Arc::new(RecordingAdapter::new());
    let mut registry = HookRegistry::new();
    let first = registry.install_snapshot_for_test(
        vec![active_binding(
            test_binding(test_source_id("a"), HookBoundary::ToolPre, 0, None),
            adapter.clone(),
        )],
        Vec::new(),
    );
    assert_eq!(first, 1);
    assert_eq!(registry.snapshot().bindings.len(), 1);

    let second = registry.install_snapshot_for_test(Vec::new(), Vec::new());
    assert_eq!(second, 2);
    assert_eq!(registry.revision(), 2);
    assert!(registry.snapshot().is_empty());
}

#[test]
fn bindings_for_filters_by_boundary() {
    let adapter: Arc<dyn HookFormatAdapter> = Arc::new(RecordingAdapter::new());
    let mut registry = HookRegistry::new();
    registry.install_snapshot_for_test(
        vec![
            active_binding(
                test_binding(test_source_id("run"), HookBoundary::ToolPre, 0, None),
                adapter.clone(),
            ),
            active_binding(
                test_binding(test_source_id("run"), HookBoundary::ToolResult, 0, None),
                adapter.clone(),
            ),
        ],
        Vec::new(),
    );

    let names = |bindings: Vec<ActiveBinding>| {
        bindings
            .into_iter()
            .map(|active| active.binding.id.source.to_string())
            .collect::<Vec<_>>()
    };

    assert_eq!(
        names(registry.bindings_for(HookBoundary::ToolPre, HookExecutionHost::Local)),
        vec!["run"]
    );
    assert_eq!(
        names(registry.bindings_for(HookBoundary::ToolResult, HookExecutionHost::Local)),
        vec!["run"]
    );
    assert!(
        registry
            .snapshot()
            .has_bindings_for(HookBoundary::ToolResult)
    );
    assert!(!registry.snapshot().has_bindings_for(HookBoundary::TurnStop));
}

#[test]
fn bindings_for_sorts_by_source_priority_and_keeps_source_order_within_it() {
    let adapter: Arc<dyn HookFormatAdapter> = Arc::new(RecordingAdapter::new());
    let factory = test_source_id_of_kind("factory", HookSourceKind::FactoryWide);
    let agent = test_source_id_of_kind("agent", HookSourceKind::FactoryAgent);
    let mut registry = HookRegistry::new();
    registry.install_snapshot_for_test(
        vec![
            active_binding(
                test_binding(factory.clone(), HookBoundary::ToolPre, 0, None),
                adapter.clone(),
            ),
            active_binding(
                test_binding(factory.clone(), HookBoundary::ToolPre, 1, None),
                adapter.clone(),
            ),
            active_binding(
                test_binding(agent.clone(), HookBoundary::ToolPre, 0, None),
                adapter.clone(),
            ),
            active_binding(
                test_binding(agent.clone(), HookBoundary::ToolPre, 1, None),
                adapter.clone(),
            ),
        ],
        Vec::new(),
    );
    let ids = registry
        .bindings_for(HookBoundary::ToolPre, HookExecutionHost::Local)
        .into_iter()
        .map(|active| active.binding.id.to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            "agent#tool.pre[0]",
            "agent#tool.pre[1]",
            "factory#tool.pre[0]",
            "factory#tool.pre[1]",
        ]
    );
}
