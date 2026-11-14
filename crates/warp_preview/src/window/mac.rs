//! Window capture through ScreenCaptureKit. Each frame is a screenshot of one window taken with
//! `SCScreenshotManager`, which works while the window is covered and never activates its app.

// Captures run on their own native threads, which block on channels by design.
#![allow(clippy::disallowed_methods)]

use std::sync::mpsc;
use std::time::Duration;

use instant::Instant;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{AnyThread, available};
use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_graphics::{
    CGDataProvider, CGImage, CGImageByteOrderInfo, CGPreflightScreenCaptureAccess,
    CGRequestScreenCaptureAccess, CGWindowListCopyWindowInfo, CGWindowListOption, kCGNullWindowID,
    kCGWindowNumber,
};
use objc2_foundation::{NSDictionary, NSError, NSNumber, NSString};
use objc2_screen_capture_kit::{
    SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration, SCWindow,
};
use pathfinder_geometry::vector::{Vector2F, vec2f};

use super::WindowEntry;
use super::input_mac::{self, InputWorker};
use crate::geometry::scale_within;
use crate::stream::Control;
use crate::{Error, Rate, Stream, StreamEvent, WindowSource, jpeg};

/// How long ScreenCaptureKit gets to answer one request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Time between frames for the preview in front, and the longest side of its frames in pixels.
const FULL_INTERVAL: Duration = Duration::from_millis(50);
const FULL_MAX_SIDE: u32 = 1600;
/// The same for cards behind it.
const THUMBNAIL_INTERVAL: Duration = Duration::from_millis(500);
const THUMBNAIL_MAX_SIDE: u32 = 400;
/// How often a window that gave no picture is looked up again.
const RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// ScreenCaptureKit objects are immutable once delivered and safe to use from any thread.
struct Delivered<T>(T);

// SAFETY: see `Delivered`.
unsafe impl<T> Send for Delivered<T> {}

pub fn has_permission() -> bool {
    CGPreflightScreenCaptureAccess()
}

pub fn request_permission() {
    CGRequestScreenCaptureAccess();
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> bool;
}

pub fn is_on_current_desktop(window_id: u32) -> bool {
    let Some(info) =
        CGWindowListCopyWindowInfo(CGWindowListOption::OptionOnScreenOnly, kCGNullWindowID)
    else {
        return false;
    };
    // SAFETY: Core Graphics documents the result as an array of dictionaries with CFString keys
    // and CFType values.
    let info: CFRetained<CFArray<CFDictionary<CFString, CFType>>> =
        unsafe { CFRetained::cast_unchecked(info) };
    // SAFETY: reading a constant key.
    let number_key = unsafe { kCGWindowNumber };
    info.iter().any(|window| {
        window
            .get(number_key)
            .and_then(|number| number.downcast::<CFNumber>().ok())
            .and_then(|number| number.as_i64())
            == Some(i64::from(window_id))
    })
}

pub fn has_input_permission() -> bool {
    // SAFETY: takes no arguments and only reads the process's trust state.
    unsafe { AXIsProcessTrusted() }
}

pub fn request_input_permission() {
    let key = NSString::from_str("AXTrustedCheckOptionPrompt");
    let prompt = NSNumber::new_bool(true);
    let options = NSDictionary::from_slices(&[&*key], &[&*prompt]);
    // SAFETY: the options dictionary is toll-free bridged to the CFDictionary the call expects and
    // outlives it.
    unsafe {
        AXIsProcessTrustedWithOptions(Retained::as_ptr(&options).cast());
    }
}

/// Every window ScreenCaptureKit can capture, including minimized ones.
fn shareable_content() -> Result<Retained<SCShareableContent>, Error> {
    if !has_permission() {
        return Err(Error::ScreenRecordingDenied);
    }
    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            // SAFETY: ScreenCaptureKit passes a valid object or null.
            let content = unsafe { Retained::retain(content) };
            let failed = !error.is_null();
            let _ = tx.send(Delivered((content, failed)));
        },
    );
    // SAFETY: the handler matches the documented signature and outlives the call, which copies it.
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true, false, &handler,
        );
    }
    match rx.recv_timeout(REQUEST_TIMEOUT) {
        Ok(Delivered((Some(content), false))) => Ok(content),
        Ok(Delivered((_, true))) | Ok(Delivered((None, _))) => Err(Error::ScreenRecordingDenied),
        Err(_) => Err(Error::Other("Screen capture didn't respond".to_owned())),
    }
}

fn entry(window: &SCWindow) -> Option<WindowEntry> {
    // SAFETY: plain property reads on a delivered object.
    unsafe {
        let app = window.owningApplication()?;
        let app_name = app.applicationName().to_string();
        let bundle_id = app.bundleIdentifier().to_string();
        let frame = window.frame();
        Some(WindowEntry {
            source: WindowSource {
                window_id: window.windowID(),
                pid: app.processID(),
                app_id: if bundle_id.is_empty() {
                    app_name.clone()
                } else {
                    bundle_id
                },
                app_name,
                title: window
                    .title()
                    .map(|title| title.to_string())
                    .unwrap_or_default(),
            },
            width: frame.size.width.max(0.) as u32,
            height: frame.size.height.max(0.) as u32,
            layer: window.windowLayer() as i64,
            on_screen: window.isOnScreen(),
        })
    }
}

pub fn list_windows() -> Result<Vec<WindowEntry>, Error> {
    let content = shareable_content()?;
    // SAFETY: a property read on a delivered object.
    let windows = unsafe { content.windows() };
    Ok(windows.iter().filter_map(|window| entry(&window)).collect())
}

fn find_window(window_id: u32) -> Result<Option<Retained<SCWindow>>, Error> {
    let content = shareable_content()?;
    // SAFETY: property reads on a delivered object.
    let windows = unsafe { content.windows() };
    Ok(windows
        .iter()
        .find(|window: &Retained<SCWindow>| unsafe { window.windowID() } == window_id))
}

/// One picture of the window, at most `max_side` pixels on its longest side.
fn capture(filter: &SCContentFilter, max_side: u32) -> Option<crate::Frame> {
    // SAFETY: property reads and setters on objects this function owns.
    let config = unsafe {
        let rect = filter.contentRect();
        let scale = filter.pointPixelScale() as f64;
        let size = (
            (rect.size.width * scale).max(1.) as u32,
            (rect.size.height * scale).max(1.) as u32,
        );
        let (width, height) = scale_within(size, max_side);
        let config = SCStreamConfiguration::new();
        config.setWidth(width as usize);
        config.setHeight(height as usize);
        config.setShowsCursor(false);
        config.setIgnoreShadowsSingleWindow(true);
        config
    };

    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(move |image: *mut CGImage, _error: *mut NSError| {
        // SAFETY: ScreenCaptureKit passes a valid image or null.
        let image = unsafe { image.as_ref() }.and_then(image_to_frame);
        let _ = tx.send(image);
    });
    // SAFETY: the handler matches the documented signature and outlives the call, which copies it.
    unsafe {
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            filter,
            &config,
            Some(&handler),
        );
    }
    rx.recv_timeout(REQUEST_TIMEOUT).ok().flatten()
}

/// Converts a 32-bit screenshot to a JPEG frame.
fn image_to_frame(image: &CGImage) -> Option<crate::Frame> {
    let width = CGImage::width(Some(image));
    let height = CGImage::height(Some(image));
    let row_bytes = CGImage::bytes_per_row(Some(image));
    if CGImage::bits_per_pixel(Some(image)) != 32 || width == 0 || height == 0 {
        return None;
    }
    let data = CGDataProvider::data(CGImage::data_provider(Some(image)).as_deref())?;
    let pixels = data.to_vec();
    let bgra = CGImage::byte_order_info(Some(image)) == CGImageByteOrderInfo::Order32Little;

    let mut rgb = Vec::with_capacity(width * height * 3);
    for row in pixels.chunks(row_bytes).take(height) {
        for pixel in row[..width * 4].chunks_exact(4) {
            if bgra {
                rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
            } else {
                rgb.extend_from_slice(&pixel[..3]);
            }
        }
    }
    jpeg::frame_from_rgb(&rgb, width as u32, height as u32)
}

pub fn start_capture(
    source: WindowSource,
    rate: Rate,
    events: async_channel::Sender<StreamEvent>,
) -> Result<Stream, Error> {
    if !available!(macos = 14.0) {
        return Err(Error::Other(
            "Previewing windows needs macOS 14 or later".to_owned(),
        ));
    }
    let window = find_window(source.window_id)?.ok_or(Error::WindowGone)?;
    let filter = Delivered(unsafe {
        SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window)
    });

    let (control_tx, control_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("preview-window".to_owned())
        .spawn(move || {
            let filter = filter;
            run(&source, filter.0, rate, &control_rx, &events);
        })
        .map_err(|err| Error::Other(err.to_string()))?;
    Ok(Stream::new(control_tx))
}

/// The window's size in pixels, which input positions are scaled to.
fn pixel_size(filter: &SCContentFilter) -> Vector2F {
    // SAFETY: property reads on a delivered filter.
    unsafe {
        let rect = filter.contentRect();
        let scale = filter.pointPixelScale() as f64;
        vec2f(
            (rect.size.width * scale) as f32,
            (rect.size.height * scale) as f32,
        )
    }
}

fn run(
    source: &WindowSource,
    mut filter: Retained<SCContentFilter>,
    mut rate: Rate,
    control: &mpsc::Receiver<Control>,
    events: &async_channel::Sender<StreamEvent>,
) {
    let window_id = source.window_id;
    let mut input: Option<InputWorker> = None;
    let mut minimized = false;
    let mut next = Instant::now();
    loop {
        let wait = match rate {
            Rate::Paused => None,
            Rate::Full | Rate::Thumbnail => Some(next.saturating_duration_since(Instant::now())),
        };
        let message = match wait {
            Some(wait) => control.recv_timeout(wait).map_err(|err| match err {
                mpsc::RecvTimeoutError::Timeout => None,
                mpsc::RecvTimeoutError::Disconnected => Some(()),
            }),
            None => control.recv().map_err(|_| Some(())),
        };
        match message {
            Ok(Control::SetRate(new_rate)) => {
                rate = new_rate;
                next = Instant::now();
                continue;
            }
            Ok(Control::Navigate(_) | Control::Resize(..) | Control::Rotate) => continue,
            Ok(Control::Input(event)) => {
                if input.is_none() {
                    input = InputWorker::start(window_id, source.pid);
                }
                if let Some(input) = &input {
                    input.send(input_mac::actions(&event, pixel_size(&filter)));
                }
                continue;
            }
            Ok(Control::Stop) | Err(Some(())) => return,
            Err(None) => {}
        }

        let (interval, max_side) = match rate {
            Rate::Full => (FULL_INTERVAL, FULL_MAX_SIDE),
            Rate::Thumbnail => (THUMBNAIL_INTERVAL, THUMBNAIL_MAX_SIDE),
            Rate::Paused => continue,
        };
        next = Instant::now() + interval;
        if let Some(frame) = capture(&filter, max_side) {
            if minimized {
                minimized = false;
                let _ = events.send_blocking(StreamEvent::Minimized(false));
            }
            if events.send_blocking(StreamEvent::Frame(frame)).is_err() {
                return;
            }
            continue;
        }

        next = Instant::now() + RETRY_INTERVAL;
        match find_window(window_id) {
            Ok(Some(window)) => {
                // SAFETY: property read and filter creation on a delivered window.
                let on_screen = unsafe { window.isOnScreen() };
                if !on_screen && !minimized {
                    minimized = true;
                    let _ = events.send_blocking(StreamEvent::Minimized(true));
                }
                filter = unsafe {
                    SCContentFilter::initWithDesktopIndependentWindow(
                        SCContentFilter::alloc(),
                        &window,
                    )
                };
            }
            Ok(None) => {
                let _ = events.send_blocking(StreamEvent::Ended(Error::WindowGone.to_string()));
                return;
            }
            Err(err) => {
                let _ = events.send_blocking(StreamEvent::Ended(err.to_string()));
                return;
            }
        }
    }
}
