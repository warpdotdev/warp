//! macOS process-level setup for headless launches.

use anyhow::{Result, bail};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

/// Tells macOS to treat this process as a background-only application, so it
/// never gets a Dock tile (and therefore never shows the perpetual Dock bounce
/// of an app that is "still launching").
///
/// The bundled CLI wrapper `exec`s the GUI executable from inside `Warp.app`,
/// so Launch Services binds the process to a bundle whose `Info.plist` makes
/// it a dockable foreground app. This applies at runtime what the standalone
/// CLI binary gets from the `LSBackgroundOnly` key in its own `Info.plist`.
///
/// Call this before any other AppKit work: `sharedApplication` here is what
/// registers the process with Launch Services, and the policy is applied on
/// that same registration, so nothing must reach AppKit ahead of it.
///
/// See APP-2946.
pub(crate) fn mark_process_as_background_only() -> Result<()> {
    let Some(mtm) = MainThreadMarker::new() else {
        bail!("must be called on the main thread");
    };

    let app = NSApplication::sharedApplication(mtm);
    if !app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited) {
        bail!("NSApplication::setActivationPolicy(.prohibited) returned false");
    }

    Ok(())
}
