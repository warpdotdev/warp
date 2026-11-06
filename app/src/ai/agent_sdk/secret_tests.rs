use super::*;

#[test]
fn validate_registry_host_accepts_bare_host() {
    assert!(validate_registry_host("ghcr.io").is_ok());
    assert!(validate_registry_host("registry.example.com").is_ok());
    assert!(validate_registry_host("localhost:5000").is_ok());
}

#[test]
fn validate_registry_host_rejects_scheme() {
    let err = validate_registry_host("https://ghcr.io").unwrap_err();
    assert!(err.to_string().contains("scheme or path"));
}

#[test]
fn validate_registry_host_rejects_path() {
    let err = validate_registry_host("ghcr.io/my-org").unwrap_err();
    assert!(err.to_string().contains("scheme or path"));
}

#[test]
fn read_docker_registry_secret_value_reads_password_from_file() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let password_file = dir.path().join("password.txt");
    fs::write(&password_file, "file-token\n").expect("write password file");

    let value = read_docker_registry_secret_value(
        Some("ghcr.io".to_string()),
        Some("octocat".to_string()),
        None,
        Some(password_file),
    )
    .expect("read succeeds")
    .expect("value present");

    let json = serde_json::to_value(&value).expect("serialize");
    assert_eq!(json["registry_host"], "ghcr.io");
    assert_eq!(json["username"], "octocat");
    assert_eq!(json["password"], "file-token");
}
