use super::*;

#[test]
fn recovery_backends_never_add_backends() {
    let backends = wgpu::Backends::all();
    assert!(backends.contains(recovery_backends(backends)));
}

#[test]
fn recovery_backends_keep_gl_when_it_is_the_only_backend() {
    assert_eq!(recovery_backends(wgpu::Backends::GL), wgpu::Backends::GL);
}

#[cfg(windows)]
#[test]
fn recovery_backends_skip_gl_on_windows() {
    let recovery = recovery_backends(wgpu::Backends::all());
    assert!(!recovery.contains(wgpu::Backends::GL));
    assert!(recovery.contains(wgpu::Backends::DX12 | wgpu::Backends::VULKAN));
}

#[cfg(not(windows))]
#[test]
fn recovery_backends_match_default_off_windows() {
    assert_eq!(
        recovery_backends(wgpu::Backends::all()),
        wgpu::Backends::all()
    );
}
