//! macOS process-level setup that must happen before any AppKit work.

use anyhow::{Result, bail};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

/// Tells macOS to treat this process as a background-only application, so it
/// never gets a Dock tile (and therefore never shows the perpetual Dock bounce
/// of an app that is "still launching").
///
/// This must run before anything touches AppKit / Launch Services, because the
/// Dock presence is established the first time the process registers itself as
/// an application.
///
/// # Why this is needed
///
/// The bundled macOS CLI wrapper (`Contents/Resources/bin/oz`,
/// `oz-<channel>`, written by `script/macos/bundle`) is a shell script that
/// `exec`s the **GUI** executable inside `Warp.app` with `argv[0]` rewritten.
/// The CLI process is therefore the GUI binary running inside the GUI bundle,
/// whose `Info.plist` declares `LSBackgroundOnly=false` and an
/// `NSDockTilePlugIn`, so Launch Services considers it a dockable foreground
/// app. The standalone CLI artifact avoids this by embedding
/// `app/assets/resources/mac/CLI-Info.plist`, which sets `LSBackgroundOnly` —
/// the bundled wrapper has no `Info.plist` of its own to opt into that, so the
/// equivalent has to be applied at runtime instead.
///
/// # Why `setActivationPolicy` and not `TransformProcessType`
///
/// [`NSApplicationActivationPolicy::Prohibited`] is the documented AppKit way
/// to say "this process is not a UI app": it gets no Dock tile, no menu bar,
/// and cannot be activated. The deprecated `TransformProcessType` Carbon call
/// reaches the same end state, but has no binding we can use — the
/// `objc2-application-services` crate keeps `ProcessSerialNumber` private, so
/// using it means hand-writing an `extern "C"` declaration and mirroring an
/// Apple struct by hand. `setActivationPolicy` needs no new dependency
/// (`objc2-app-kit` is already a dependency of this crate) and no hand-written
/// binding.
///
/// The obvious objection is that reaching `setActivationPolicy` requires
/// `NSApplication::sharedApplication`, which Apple documents as connecting to
/// the window server — seemingly the very AppKit initialization the headless
/// path exists to avoid. That was measured on macOS 26.3.1 (arm64) rather than
/// assumed, by A/B-ing the mechanisms against the bundled CLI inside an
/// installed `Warp.app`, and the objection does not hold:
///
/// - Doing nothing leaves the CLI registered as `Foreground`, and a Warp icon
///   appears in the Dock.
/// - `sharedApplication` on its own does register the process with Launch
///   Services — but so does the untouched control roughly 0.15 s later, of its
///   own accord. `sharedApplication` only makes that registration happen
///   sooner; it does not introduce one that would not otherwise occur.
/// - Setting the policy to `Prohibited` immediately afterwards (~1 ms later)
///   lands the process on the same `BackgroundOnly` Launch Services type that
///   `TransformProcessType` produces, and no Dock tile is ever created.
///
/// See APP-2946.
pub(crate) fn mark_process_as_background_only() -> Result<()> {
    // `run_internal` calls this from the process's main thread, before the
    // platform event loop starts.
    let Some(mtm) = MainThreadMarker::new() else {
        bail!("must be called on the main thread");
    };

    let app = NSApplication::sharedApplication(mtm);
    if !app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited) {
        bail!("NSApplication::setActivationPolicy(.prohibited) returned false");
    }

    Ok(())
}
