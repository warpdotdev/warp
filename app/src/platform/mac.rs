//! macOS process-level setup that must happen before any AppKit work.

use anyhow::{Result, bail};

/// `kCurrentProcess` from `<CoreServices/MacTypes.h>` — the
/// `ProcessSerialNumber` low word that refers to the calling process. Using it
/// directly avoids the deprecated `GetCurrentProcess`.
const K_CURRENT_PROCESS: u32 = 2;

/// `kProcessTransformToBackgroundApplication` from `ApplicationServices`.
/// Converts the process into a background-only application: no Dock tile, no
/// menu bar, and it can never be activated. This is the runtime equivalent of
/// `LSBackgroundOnly` in an `Info.plist`.
const K_PROCESS_TRANSFORM_TO_BACKGROUND_APPLICATION: u32 = 2;

/// Mirrors `ProcessSerialNumber` from `<CoreServices/MacTypes.h>`.
#[repr(C)]
struct ProcessSerialNumber {
    high_long_of_psn: u32,
    low_long_of_psn: u32,
}

unsafe extern "C" {
    /// `TransformProcessType` from `ApplicationServices` (HIServices).
    fn TransformProcessType(psn: *const ProcessSerialNumber, transform_state: u32) -> i32;
}

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
/// See APP-2946.
pub(crate) fn mark_process_as_background_only() -> Result<()> {
    let psn = ProcessSerialNumber {
        high_long_of_psn: 0,
        low_long_of_psn: K_CURRENT_PROCESS,
    };

    // SAFETY: `psn` is a correctly-shaped `ProcessSerialNumber` that outlives
    // the call, and `TransformProcessType` only reads through the pointer.
    let status =
        unsafe { TransformProcessType(&psn, K_PROCESS_TRANSFORM_TO_BACKGROUND_APPLICATION) };

    if status != 0 {
        bail!("TransformProcessType returned OSStatus {status}");
    }
    Ok(())
}
