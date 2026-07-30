use objc2_app_kit::{NSAlert, NSApplication, NSModalResponse};

/// Configures and runs an `NSAlert` modally, returning its response.
///
/// Exported via `#[no_mangle]` so the AppKit `showModal:modalId:` dispatch path
/// in `app.m` can call it by its C symbol; `app.m` invokes it inside a
/// main-queue block, so this runs on the main thread and `runModal` stays
/// synchronous.
#[unsafe(no_mangle)]
pub extern "C-unwind" fn configureAndRunModal(
    alert: &NSAlert,
    app: &NSApplication,
) -> NSModalResponse {
    alert.setShowsSuppressionButton(true);

    // It is generally frowned-upon to be overly assertive about putting our windows in
    // the user's face. However, it is reasonable to do this before showing our modal. If
    // we don't make ourselves the top active app, our modal might show up behind another
    // app's window.
    app.activateIgnoringOtherApps(true);

    alert.runModal()
}
