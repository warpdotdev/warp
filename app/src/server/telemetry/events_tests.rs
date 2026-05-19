use warp_core::telemetry::TelemetryEventDesc;

#[derive(Debug)]
enum TelemetryEventPropertyError {
    // The variant data is never directly read, but it's used for error formatting if the test
    // below fails.
    EmptyName(#[expect(dead_code)] Box<dyn TelemetryEventDesc>),
    EmptyDescription(#[expect(dead_code)] Box<dyn TelemetryEventDesc>),
}

/// Checks that all telemetry events have a non-empty name and description.
///
/// The name and description are intended to be user-facing and are used to populate
/// our [exhaustive telemetry table](https://docs.warp.dev/support-and-community/privacy-and-security/privacy#exhaustive-telemetry-table).
#[test]
#[cfg(not(target_family = "wasm"))]
fn telemetry_events_have_nonempty_name_and_description() -> Result<(), TelemetryEventPropertyError>
{
    for event in warp_core::telemetry::all_events() {
        if event.name().is_empty() {
            return Err(TelemetryEventPropertyError::EmptyName(event));
        }
        if event.description().is_empty() {
            return Err(TelemetryEventPropertyError::EmptyDescription(event));
        }
    }
    Ok(())
}

/// GH9729 §517 acceptance criterion: a click on an image file in the
/// project explorer surfaces as a `CodePanelsFileOpened` event whose
/// `target` field serializes to the stable string `"image_preview"`.
/// Dashboards filter on this string to distinguish image opens from
/// markdown / code-editor / system-generic opens.
#[test]
#[cfg(feature = "local_fs")]
#[cfg(not(target_family = "wasm"))]
fn code_panels_file_opened_serializes_image_preview_target() {
    use crate::util::openable_file_type::FileTarget;
    // `payload()` is an inherent method on `TelemetryEvent` (events.rs:2862
    // routes through it), so no trait import is required at the call site.

    let event = super::TelemetryEvent::CodePanelsFileOpened {
        entrypoint: super::CodePanelsFileOpenEntrypoint::ProjectExplorer,
        target: FileTarget::ImagePreview,
    };
    let payload = event.payload().expect("event has a payload");
    let target = payload
        .get("target")
        .and_then(|v| v.as_str())
        .expect("payload has a string `target` field");
    assert_eq!(target, "image_preview");
}
