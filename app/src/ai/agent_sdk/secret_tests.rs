use super::*;

#[test]
fn validate_registry_host_accepts_bare_host() {
    for host in ["ghcr.io", "registry.example.com", "localhost:5000"] {
        assert!(validate_registry_host(host).is_ok(), "host: {host}");
    }
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
