use super::is_warp_bundle;

#[test]
fn is_warp_bundle_recognises_warp_channels() {
    assert!(is_warp_bundle("dev.warp.Warp"));
    assert!(is_warp_bundle("dev.warp.WarpDev"));
    assert!(is_warp_bundle("dev.warp.WarpPreview"));
    assert!(is_warp_bundle("dev.warp.WarpOss"));
}

#[test]
fn is_warp_bundle_rejects_other_apps() {
    assert!(!is_warp_bundle("com.microsoft.VSCode"));
    assert!(!is_warp_bundle("com.apple.TextEdit"));
    assert!(!is_warp_bundle("dev.zed.Zed"));
    assert!(!is_warp_bundle("invalid"));
    assert!(!is_warp_bundle(""));
}
