use super::*;

#[test]
fn validate_os_config_rejects_macos_version_with_linux() {
    let err = validate_os_config(
        RunnerOsArg::Linux,
        None,
        Some(RunnerMacosVersionArg::Macos14),
    )
    .expect_err("macos-version with linux is rejected");
    assert!(err.contains("--macos-version"), "got: {err}");
}

#[test]
fn validate_os_config_rejects_docker_image_with_macos() {
    let err = validate_os_config(RunnerOsArg::Macos, Some("ubuntu:latest"), None)
        .expect_err("docker-image with macos is rejected");
    assert!(err.contains("--docker-image"), "got: {err}");
}

#[test]
fn validate_os_config_accepts_matching_linux() {
    validate_os_config(RunnerOsArg::Linux, Some("ubuntu:latest"), None)
        .expect("docker-image with linux is valid");
}

#[test]
fn validate_os_config_accepts_matching_macos() {
    validate_os_config(
        RunnerOsArg::Macos,
        None,
        Some(RunnerMacosVersionArg::Macos15),
    )
    .expect("macos-version with macos is valid");
}
