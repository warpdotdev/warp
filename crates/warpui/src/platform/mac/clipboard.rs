use std::ffi::CStr;
use std::os::raw::{c_uchar, c_void};
use std::slice;

use anyhow::{anyhow, Result};
use cocoa::appkit::{NSPasteboard, NSPasteboardTypeHTML, NSPasteboardTypeString};
use cocoa::base::{id, nil};
use cocoa::foundation::{NSArray, NSData, NSString};
use objc::{class, msg_send, sel, sel_impl};
use warpui_core::clipboard::{ClipboardContent, ImageData};

use super::make_nsstring;

extern "C" {
    fn getFilePathsFromPasteboard() -> id;
}

pub struct Clipboard(id);

unsafe impl Send for Clipboard {}

impl Clipboard {
    pub fn new() -> Result<Self> {
        let pboard = unsafe { NSPasteboard::generalPasteboard(nil) };
        if pboard.is_null() {
            Err(anyhow!("NSPasteboard::generalPasteboard returned nil"))
        } else {
            Ok(Clipboard(pboard))
        }
    }
}

unsafe fn pasteboard_type_for_image_mime_type(mime_type: &str) -> Option<id> {
    let pasteboard_type = match mime_type {
        "image/png" => "public.png",
        "image/jpeg" => "public.jpeg",
        "image/gif" => "public.gif",
        "image/webp" => "public.webp",
        "image/svg+xml" => "public.svg-image",
        _ => return None,
    };
    Some(make_nsstring(pasteboard_type))
}

impl crate::Clipboard for Clipboard {
    fn write(&mut self, contents: ClipboardContent) {
        unsafe {
            let nsstr = make_nsstring(&contents.plain_text);
            self.0
                .declareTypes_owner(NSArray::arrayWithObject(nil, NSPasteboardTypeString), nil);
            NSPasteboard::setString_forType(self.0, nsstr, NSPasteboardTypeString);

            if let Some(html) = contents.html {
                let nsstr = make_nsstring(&html);
                self.0
                    .addTypes_owner(NSArray::arrayWithObject(nil, NSPasteboardTypeHTML), nil);
                NSPasteboard::setString_forType(self.0, nsstr, NSPasteboardTypeHTML);
            }

            if let Some(images) = contents.images {
                for image in images {
                    let Some(pasteboard_type) =
                        pasteboard_type_for_image_mime_type(&image.mime_type)
                    else {
                        continue;
                    };
                    let data: id = msg_send![class!(NSData), alloc];
                    let data: id = data.initWithBytes_length_(
                        image.data.as_ptr() as *const c_void,
                        image.data.len() as u64,
                    );
                    self.0
                        .addTypes_owner(NSArray::arrayWithObject(nil, pasteboard_type), nil);
                    let _: () = msg_send![self.0, setData: data forType: pasteboard_type];
                    // Balance the +1 retain from `[NSData alloc]`. The pasteboard retains
                    // `data` in `setData:forType:`, so the object stays alive as needed.
                    let _: () = msg_send![data, release];
                }
            }
        }
    }

    fn read(&mut self) -> ClipboardContent {
        unsafe {
            // Try getting file paths from the clipboard. If we end up with an empty
            // array of file paths, fallback to getting the string from the pasteboard.
            let file_paths = getFilePathsFromPasteboard();
            let available_paths = file_paths.count();

            let text = NSPasteboard::stringForType(self.0, NSPasteboardTypeString);
            let mut content = ClipboardContent::plain_text(if text != nil {
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

            let html = NSPasteboard::stringForType(self.0, NSPasteboardTypeHTML);
            if html != nil {
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
                make_nsstring("public.png"),
                make_nsstring("public.jpeg"),
                make_nsstring("public.gif"),
                make_nsstring("public.webp"),
                make_nsstring("public.svg-image"),
                make_nsstring("com.compuserve.gif"),
            ];

            let mut images = Vec::new();

            for &pasteboard_type in &supported_pasteboard_types {
                let data = NSPasteboard::dataForType(self.0, pasteboard_type);
                if data != nil {
                    let length = NSData::length(data);
                    if length > 0 {
                        let bytes_ptr = NSData::bytes(data) as *const u8;
                        let bytes = slice::from_raw_parts(bytes_ptr, length as usize);

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
                            let html = NSPasteboard::stringForType(self.0, NSPasteboardTypeHTML);
                            if html != nil {
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
                            data: bytes.to_vec(),
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
