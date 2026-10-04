use super::*;

#[test]
fn recovery_backends_never_add_backends() {
    assert!(wgpu_backend_options().contains(wgpu_recovery_backend_options()));
}

#[cfg(windows)]
#[test]
fn recovery_backends_skip_gl_on_windows() {
    assert!(!wgpu_recovery_backend_options().contains(wgpu::Backends::GL));
}

#[cfg(not(windows))]
#[test]
fn recovery_backends_match_default_off_windows() {
    assert_eq!(wgpu_recovery_backend_options(), wgpu_backend_options());
}
