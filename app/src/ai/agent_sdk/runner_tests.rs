use warp_cli::runner::UpdateRunnerArgs;

use super::{
    RunnerArch, RunnerArchArg, RunnerOsArg, confirm_delete, merge_instance_shape, resolve_arch,
    resolve_updated_name, update_selector,
};

fn update_args(identifier: Option<&str>, name: Option<&str>) -> UpdateRunnerArgs {
    UpdateRunnerArgs {
        identifier: identifier.map(str::to_string),
        name: name.map(str::to_string),
        description: None,
        setup_command: Vec::new(),
        os: None,
        arch: None,
        docker_image: None,
        macos_version: None,
        vcpus: None,
        memory_gb: None,
    }
}

#[test]
fn confirm_delete_refuses_non_interactive_without_force() {
    // In non-interactive mode, refusal must surface as an error so the caller
    // exits non-zero instead of treating a skipped delete as a success.
    let err = confirm_delete("runner-123", false).expect_err("non-interactive refusal is an error");
    let msg = err.to_string();
    assert!(msg.contains("non-interactive"), "got: {msg}");
    assert!(msg.contains("runner-123"), "got: {msg}");
}

#[test]
fn resolve_arch_auto_maps_to_os_default() {
    assert!(matches!(
        resolve_arch(RunnerArchArg::Auto, RunnerOsArg::Linux),
        RunnerArch::X8664
    ));
    assert!(matches!(
        resolve_arch(RunnerArchArg::Auto, RunnerOsArg::Macos),
        RunnerArch::Aarch64
    ));
}

#[test]
fn resolve_arch_explicit_is_preserved_regardless_of_os() {
    assert!(matches!(
        resolve_arch(RunnerArchArg::X8664, RunnerOsArg::Macos),
        RunnerArch::X8664
    ));
    assert!(matches!(
        resolve_arch(RunnerArchArg::Aarch64, RunnerOsArg::Linux),
        RunnerArch::Aarch64
    ));
}

#[test]
fn merge_instance_shape_updates_dimensions_independently() {
    // Neither specified: preserve the existing shape.
    assert_eq!(
        merge_instance_shape(None, None, Some((2, 4))).unwrap(),
        Some((2, 4))
    );
    // Only vCPUs: keep existing memory.
    assert_eq!(
        merge_instance_shape(Some(8), None, Some((2, 4))).unwrap(),
        Some((8, 4))
    );
    // Only memory: keep existing vCPUs.
    assert_eq!(
        merge_instance_shape(None, Some(16), Some((2, 4))).unwrap(),
        Some((2, 16))
    );
    // Both specified: use both.
    assert_eq!(
        merge_instance_shape(Some(8), Some(16), Some((2, 4))).unwrap(),
        Some((8, 16))
    );
    // No existing shape and nothing set: no shape.
    assert_eq!(merge_instance_shape(None, None, None).unwrap(), None);
}

#[test]
fn merge_instance_shape_errors_on_partial_shape_without_existing() {
    assert!(merge_instance_shape(Some(8), None, None).is_err());
    assert!(merge_instance_shape(None, Some(16), None).is_err());
}

#[test]
fn resolve_updated_name_renames_only_with_uid() {
    // UID + --name renames the runner.
    assert_eq!(resolve_updated_name(true, Some("new"), "old"), "new");
    // UID without --name keeps the existing name.
    assert_eq!(resolve_updated_name(true, None, "old"), "old");
    // No UID: --name is the selector, so the name is unchanged.
    assert_eq!(resolve_updated_name(false, Some("old"), "old"), "old");
}

#[test]
fn update_selector_uses_identifier_for_both_uid_and_name() {
    // identifier is ambiguous (uid or name), so it feeds both fields; the
    // server tries uid first and falls back to name.
    let args = update_args(Some("uid-or-name"), None);
    let selector = update_selector(&args);
    assert_eq!(
        selector.uid.map(|id| id.inner().to_string()),
        Some("uid-or-name".to_string())
    );
    assert_eq!(selector.name.as_deref(), Some("uid-or-name"));
}

#[test]
fn update_selector_ignores_rename_value_when_identifier_is_given() {
    // --name alongside a positional identifier is a rename instruction, not a
    // lookup key, so it must not leak into the selector's name field.
    let args = update_args(Some("uid-1"), Some("renamed"));
    let selector = update_selector(&args);
    assert_eq!(
        selector.uid.map(|id| id.inner().to_string()),
        Some("uid-1".to_string())
    );
    assert_eq!(selector.name.as_deref(), Some("uid-1"));
}

#[test]
fn update_selector_uses_name_flag_as_sole_selector_when_identifier_is_absent() {
    let args = update_args(None, Some("by-name"));
    let selector = update_selector(&args);
    assert_eq!(selector.uid, None);
    assert_eq!(selector.name.as_deref(), Some("by-name"));
}
