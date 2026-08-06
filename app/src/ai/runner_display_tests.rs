use std::collections::HashMap;

use warp_graphql::queries::get_runners::{RunnerArch, RunnerMacOsVersion, RunnerOs};

use super::{RunPlatform, RunnerPlatform, resolve_run_platform};
use crate::ui_components::icons::Icon;

fn linux_runner() -> RunnerPlatform {
    RunnerPlatform {
        os: RunnerOs::Linux,
        arch: RunnerArch::X8664,
        macos_version: None,
        name: "linux-standard".to_string(),
        vcpus: Some(4),
        memory_gb: Some(16),
    }
}

fn macos_runner() -> RunnerPlatform {
    RunnerPlatform {
        os: RunnerOs::Macos,
        arch: RunnerArch::Aarch64,
        macos_version: Some(RunnerMacOsVersion::Macos26),
        name: "macos-arm64".to_string(),
        vcpus: Some(8),
        memory_gb: Some(14),
    }
}

fn platforms() -> HashMap<String, RunnerPlatform> {
    HashMap::from([
        ("runner-linux".to_string(), linux_runner()),
        ("runner-macos".to_string(), macos_runner()),
    ])
}

#[test]
fn resolves_the_runner_the_run_names() {
    let resolved = resolve_run_platform(Some("runner-macos"), Some("runner-linux"), &platforms());

    assert_eq!(resolved, Some(RunPlatform::Runner(macos_runner())));
}

#[test]
fn falls_back_to_the_environment_default_runner() {
    let resolved = resolve_run_platform(None, Some("runner-linux"), &platforms());

    assert_eq!(resolved, Some(RunPlatform::Runner(linux_runner())));
}

#[test]
fn treats_a_blank_runner_reference_as_absent() {
    let resolved = resolve_run_platform(Some("  "), Some("runner-macos"), &platforms());

    assert_eq!(resolved, Some(RunPlatform::Runner(macos_runner())));
}

#[test]
fn reports_default_compute_when_nothing_references_a_runner() {
    let resolved = resolve_run_platform(None, None, &platforms());

    assert_eq!(resolved, Some(RunPlatform::Default));
    assert_eq!(resolved.unwrap().summary(), "Linux · x86-64");
}

// A referenced runner the client cannot see (deleted, inaccessible, or not
// fetched yet) must not be reported as some other platform.
#[test]
fn reports_nothing_when_a_referenced_runner_is_unresolvable() {
    assert_eq!(
        resolve_run_platform(Some("runner-missing"), None, &platforms()),
        None
    );
    assert_eq!(
        resolve_run_platform(None, Some("runner-missing"), &platforms()),
        None
    );
    assert_eq!(
        resolve_run_platform(Some("runner-linux"), None, &HashMap::new()),
        None
    );
}

#[test]
fn summarizes_a_macos_runner_with_its_version_and_shape() {
    let summary = RunPlatform::Runner(macos_runner()).summary();

    assert_eq!(summary, "macOS 26 · aarch64 · macos-arm64 · 8 vCPU · 14 GB");
}

#[test]
fn summarizes_a_linux_runner_without_an_os_version() {
    let summary = RunPlatform::Runner(linux_runner()).summary();

    assert_eq!(summary, "Linux · x86-64 · linux-standard · 4 vCPU · 16 GB");
}

#[test]
fn omits_absent_runner_metadata_from_the_summary() {
    let sparse = RunnerPlatform {
        os: RunnerOs::Linux,
        arch: RunnerArch::Aarch64,
        macos_version: None,
        name: String::new(),
        vcpus: None,
        memory_gb: None,
    };

    assert_eq!(RunPlatform::Runner(sparse).summary(), "Linux · aarch64");
}

#[test]
fn uses_a_distinct_icon_per_operating_system() {
    assert_eq!(RunPlatform::Runner(macos_runner()).icon(), Icon::Apple);
    assert_eq!(RunPlatform::Runner(linux_runner()).icon(), Icon::Linux);
    assert_eq!(RunPlatform::Default.icon(), Icon::Linux);
}

// The two shape fields are independent, so a runner that somehow reports only
// one of them still renders the half it has.
#[test]
fn summarizes_a_partially_known_instance_shape() {
    let only_memory = RunnerPlatform {
        os: RunnerOs::Linux,
        arch: RunnerArch::X8664,
        macos_version: None,
        name: "half-known".to_string(),
        vcpus: None,
        memory_gb: Some(16),
    };
    assert_eq!(
        RunPlatform::Runner(only_memory).summary(),
        "Linux · x86-64 · half-known · 16 GB"
    );

    let only_vcpus = RunnerPlatform {
        os: RunnerOs::Linux,
        arch: RunnerArch::X8664,
        macos_version: None,
        name: "half-known".to_string(),
        vcpus: Some(4),
        memory_gb: None,
    };
    assert_eq!(
        RunPlatform::Runner(only_vcpus).summary(),
        "Linux · x86-64 · half-known · 4 vCPU"
    );
}
