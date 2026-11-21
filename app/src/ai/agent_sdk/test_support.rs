//! Test-only helpers shared across `agent_sdk` unit tests.

/// Build a reqwest-backed `http_client::Client` with TLS/proxy disabled and the connection
/// pool disabled (`pool_max_idle_per_host(0)`).
///
/// The pool setting is the key bit: reqwest's default 90-second idle timeout holds sockets
/// open past a test's return, which nextest's leak detector flags on retry-exhaustion tests
/// that spin up multiple connections.
pub(super) fn build_test_http_client() -> http_client::Client {
    let builder = reqwest::ClientBuilder::new()
        .tls_certs_only([])
        .no_proxy()
        .pool_max_idle_per_host(0);
    http_client::Client::from_client_builder(builder)
        .expect("should not fail to build test http client")
}

/// Sets or clears one process environment variable for the guard's lifetime and restores the
/// previous value when dropped.
///
/// The process environment is shared, so every test that holds one of these must be
/// `#[serial_test::serial]`.
pub(super) struct EnvVarGuard {
    key: &'static str,
    original: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    pub(super) fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let original = std::env::var_os(key);
        // SAFETY: callers are `#[serial_test::serial]`, so no other thread touches the process
        // environment concurrently.
        unsafe { std::env::set_var(key, value) };
        Self { key, original }
    }

    pub(super) fn remove(key: &'static str) -> Self {
        let original = std::env::var_os(key);
        // SAFETY: see `EnvVarGuard::set`.
        unsafe { std::env::remove_var(key) };
        Self { key, original }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.original.take() {
            // SAFETY: see `EnvVarGuard::set`.
            Some(original) => unsafe { std::env::set_var(self.key, original) },
            // SAFETY: see `EnvVarGuard::set`.
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}
