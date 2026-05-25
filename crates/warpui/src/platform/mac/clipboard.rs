use std::ffi::CStr;
use std::os::raw::c_uchar;
use std::slice;

use anyhow::Result;
use cocoa::base::id;
use objc2::rc::Retained;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeHTML, NSPasteboardTypeString};
use objc2_foundation::{NSArray, NSData, NSString};
use warpui_core::clipboard::{ClipboardContent, ImageData};

extern "C" {
    fn getFilePathsFromPasteboard() -> id;
}

pub struct Clipboard(Retained<NSPasteboard>);

unsafe impl Send for Clipboard {}

impl Clipboard {
    pub fn new() -> Result<Self> {
        // `generalPasteboard` is documented to always return the shared
        // pasteboard, so objc2 models it as a non-null `Retained`.
        Ok(Clipboard(NSPasteboard::generalPasteboard()))
    }
}

fn pasteboard_type_for_image_mime_type(mime_type: &str) -> Option<Retained<NSString>> {
    let pasteboard_type = match mime_type {
        "image/png" => "public.png",
        "image/jpeg" => "public.jpeg",
        "image/gif" => "public.gif",
        "image/webp" => "public.webp",
        "image/svg+xml" => "public.svg-image",
        _ => return None,
    };
    Some(NSString::from_str(pasteboard_type))
}

impl crate::Clipboard for Clipboard {
    fn write(&mut self, contents: ClipboardContent) {
        unsafe {
            let nsstr = NSString::from_str(&contents.plain_text);
            self.0
                .declareTypes_owner(&NSArray::from_slice(&[NSPasteboardTypeString]), None);
            self.0.setString_forType(&nsstr, NSPasteboardTypeString);

            if let Some(html) = contents.html {
                let nsstr = NSString::from_str(&html);
                self.0
                    .addTypes_owner(&NSArray::from_slice(&[NSPasteboardTypeHTML]), None);
                self.0.setString_forType(&nsstr, NSPasteboardTypeHTML);
            }

            if let Some(images) = contents.images {
                for image in images {
                    let Some(pasteboard_type) =
                        pasteboard_type_for_image_mime_type(&image.mime_type)
                    else {
                        continue;
                    };
                    // `NSData::with_bytes` copies the image bytes into a +1-retained
                    // NSData. The pasteboard retains it in `setData:forType:`, and the
                    // `Retained` releases our reference when it drops at the loop end.
                    let data = NSData::with_bytes(&image.data);
                    self.0
                        .addTypes_owner(&NSArray::from_slice(&[&*pasteboard_type]), None);
                    self.0.setData_forType(Some(&data), &pasteboard_type);
                }
            }
        }
    }

    fn read(&mut self) -> ClipboardContent {
        unsafe {
            // Try getting file paths from the clipboard. If we end up with an empty
            // array of file paths, fallback to getting the string from the pasteboard.
            let file_paths = &*getFilePathsFromPasteboard().cast::<NSArray<NSString>>();
            let available_paths = file_paths.count();

            let text = self.0.stringForType(NSPasteboardTypeString);
            let mut content = ClipboardContent::plain_text(if let Some(text) = text.as_deref() {
                CStr::from_ptr(text.UTF8String())
                    .to_str()
                    .unwrap_or("")
                    .to_string()
            } else {
                String::from("")
            });

            if available_paths > 0 {
                content.paths = Some(
                    (0..available_paths)
                        .map(|i| {
                            let directory = file_paths.objectAtIndex(i);
                            let slice = slice::from_raw_parts(
                                directory.UTF8String() as *const c_uchar,
                                directory.len(),
                            );
                            std::str::from_utf8_unchecked(slice).to_string()
                        })
                        .collect::<Vec<String>>(),
                );
            }

            let html = self.0.stringForType(NSPasteboardTypeHTML);
            if let Some(html) = html.as_deref() {
                content.html = Some(
                    CStr::from_ptr(html.UTF8String())
                        .to_str()
                        .unwrap_or("")
                        .to_string(),
                )
            }

            // Try to read image data from clipboard
            content.images = self.read_image_data_from_pasteboard();

            content
        }
    }
}

impl Clipboard {
    /// Reads image data from the macOS pasteboard.
    ///
    /// Checks for supported image formats and returns the first available image
    /// data found, prioritizing common web-compatible formats.
    fn read_image_data_from_pasteboard(&self) -> Option<Vec<ImageData>> {
        unsafe {
            // Check for common image types on macOS pasteboard
            // macOS pasteboard type identifiers for supported image formats
            // Ordered by preference for web compatibility
            let supported_pasteboard_types = [
                NSString::from_str("public.png"),
                NSString::from_str("public.jpeg"),
                NSString::from_str("public.gif"),
                NSString::from_str("public.webp"),
                NSString::from_str("public.svg-image"),
                NSString::from_str("com.compuserve.gif"),
            ];

            let mut images = Vec::new();

            for pasteboard_type in &supported_pasteboard_types {
                if let Some(data) = self.0.dataForType(pasteboard_type) {
                    let length = data.len();
                    if length > 0 {
                        let mime_type = match CStr::from_ptr(pasteboard_type.UTF8String())
                            .to_str()
                            .unwrap_or("")
                        {
                            "public.png" => "image/png",
                            "public.jpeg" => "image/jpeg",
                            "public.gif" | "com.compuserve.gif" => "image/gif",
                            "public.webp" => "image/webp",
                            "public.svg-image" => "image/svg+xml",
                            _ => "image/unknown",
                        };

                        // Try to extract filename from HTML content if available
                        let filename = {
                            let html = self.0.stringForType(NSPasteboardTypeHTML);
                            if let Some(html) = html.as_deref() {
                                let html_str =
                                    CStr::from_ptr(html.UTF8String()).to_str().unwrap_or("");
                                if !html_str.is_empty() {
                                    crate::clipboard_utils::extract_filename_from_html(html_str)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        };

                        images.push(ImageData {
                            data: data.to_vec(),
                            mime_type: mime_type.to_string(),
                            filename,
                        });
                    }
                }
            }

            if images.is_empty() {
                None
            } else {
                Some(images)
            }
        }
    }
}

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;
