//! Screenshot capture for Wayland using the XDG Desktop Portal.

use std::collections::HashMap;

use futures::StreamExt as _;
use zbus::zvariant;

use crate::{Screenshot, ScreenshotParams};

/// A D-Bus proxy for the Screenshot portal.
#[zbus::proxy(
    interface = "org.freedesktop.portal.Screenshot",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait ScreenshotPortal {
    /// Takes a screenshot.
    ///
    /// Returns an object path for a Request object that will receive the response.
    fn screenshot(
        &self,
        parent_window: &str,
        options: HashMap<&str, zvariant::Value<'_>>,
    ) -> zbus::fdo::Result<zvariant::OwnedObjectPath>;
}

/// A D-Bus proxy for portal Request objects.
#[zbus::proxy(
    interface = "org.freedesktop.portal.Request",
    default_service = "org.freedesktop.portal.Desktop"
)]
trait PortalRequest {
    /// Signal emitted when the request completes.
    #[zbus(signal)]
    fn response(
        &self,
        response: u32,
        results: HashMap<String, zvariant::OwnedValue>,
    ) -> zbus::fdo::Result<()>;
}

/// Takes a screenshot using the XDG Desktop Portal.
pub async fn take(params: ScreenshotParams) -> Result<Screenshot, String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|e| format!("Failed to connect to D-Bus session bus: {e}"))?;

    let screenshot_proxy = ScreenshotPortalProxy::new(&connection)
        .await
        .map_err(|e| format!("Failed to create screenshot portal proxy: {e}"))?;

    // Request a non-interactive screenshot.
    let mut options: HashMap<&str, zvariant::Value> = HashMap::new();
    options.insert("interactive", zvariant::Value::Bool(false));

    let request_path = screenshot_proxy
        .screenshot("", options)
        .await
        .map_err(|e| format!("Failed to request screenshot: {e}"))?;

    // Wait for the response signal.
    let request_proxy = PortalRequestProxy::builder(&connection)
        .path(request_path)
        .map_err(|e| format!("Failed to build request proxy: {e}"))?
        .build()
        .await
        .map_err(|e| format!("Failed to create request proxy: {e}"))?;

    let mut response_stream = request_proxy
        .receive_response()
        .await
        .map_err(|e| format!("Failed to subscribe to response signal: {e}"))?;

    // Wait for the response.
    let response = response_stream
        .next()
        .await
        .ok_or("Screenshot request was cancelled or timed out")?;

    let args = response
        .args()
        .map_err(|e| format!("Failed to get response arguments: {e}"))?;

    // Response code 0 means success, 1 means cancelled, 2 means other error.
    if args.response != 0 {
        return Err(format!(
            "Screenshot request failed with response code: {}",
            args.response
        ));
    }

    // Extract the URI from the results.
    let uri_value = args
        .results
        .get("uri")
        .ok_or("Screenshot response missing 'uri' field")?;

    let uri: &str = uri_value
        .downcast_ref()
        .map_err(|e| format!("Failed to get URI from response: {e}"))?;

    // Parse the file:// URI and read the file.
    let path = url::Url::parse(uri)
        .map_err(|e| format!("Failed to parse screenshot URI: {e}"))?
        .to_file_path()
        .map_err(|_| format!("Screenshot URI is not a valid file path: {uri}"))?;

    // Load the image from the temporary file.
    let img = image::ImageReader::open(&path)
        .map_err(|e| format!("Failed to open screenshot file: {e}"))?
        .decode()
        .map_err(|e| format!("Failed to decode screenshot: {e}"))?;

    // Clean up the temporary file created by the portal.
    let _ = std::fs::remove_file(&path);

    // If capturing a region, crop the full-display image.
    // The XDG Portal doesn't support region capture natively.
    let img = if let Some(region) = params.region {
        region.validate()?;
        crate::screenshot_utils::crop_to_region(img, region.top_left, region.bottom_right)
    } else {
        img
    };

    crate::screenshot_utils::process_screenshot(img, params)
}
