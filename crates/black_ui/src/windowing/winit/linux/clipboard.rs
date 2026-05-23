use std::ops::Not;

use arboard::{
    self, Clipboard as LinuxClipboardInner, GetExtLinux, LinuxClipboardKind, SetExtLinux,
};
use zbus::zvariant::NoneValue;

use crate::clipboard::ClipboardContent;
use crate::Clipboard;

pub struct LinuxClipboard {
    inner: LinuxClipboardInner,
}

impl LinuxClipboard {
    pub fn new() -> Result<Self, arboard::Error> {
        Ok(Self {
            inner: LinuxClipboardInner::new()?,
        })
    }
}

impl Clipboard for LinuxClipboard {
    fn write(&mut self, contents: ClipboardContent) {
        if let Err(err) = self.write_to_specific_clipboard(LinuxClipboardKind::Clipboard, &contents)
        {
            if contents.html.is_some() {
                log::warn!("Unable to set clipboard HTML: {err:?}");
            } else {
                log::warn!("Unable to set clipboard text: {err:?}");
            }
        }
    }

    fn read(&mut self) -> ClipboardContent {
        match self.read_from_specific_clipboard(LinuxClipboardKind::Clipboard) {
            Ok(content) => content,
            Err(err) => {
                log::warn!("Failed to read from Linux clipboard: {err:?}");
                ClipboardContent::null_value()
            }
        }
    }

    fn write_to_primary_clipboard(&mut self, contents: ClipboardContent) {
        match self.write_to_specific_clipboard(LinuxClipboardKind::Primary, &contents) {
            Ok(_) => (),
            Err(arboard::Error::ClipboardNotSupported) => {
                log::info!(
                    "Primary clipboard is not supported, falling back to default clipboard."
                );
                // Try the default clipboard.
                self.write(contents);
            }
            Err(err) => {
                if contents.html.is_some() {
                    log::warn!("Unable to set primary clipboard HTML: {err:?}");
                } else {
                    log::warn!("Unable to set primary clipboard text: {err:?}");
                }
            }
        }
    }

    fn read_from_primary_clipboard(&mut self) -> ClipboardContent {
        match self.read_from_specific_clipboard(LinuxClipboardKind::Primary) {
            Ok(content) => content,
            Err(arboard::Error::ClipboardNotSupported) => {
                log::info!(
                    "Primary clipboard is not supported, falling back to default clipboard."
                );
                // Try the default clipboard.
                match self.read_from_specific_clipboard(LinuxClipboardKind::Clipboard) {
                    Ok(content) => content,
                    Err(err) => {
                        log::warn!("Unable to read from primary clipboard fallback: {err:?}");
                        ClipboardContent::null_value()
                    }
                }
            }
            Err(err) => {
                log::warn!("Unable to read from primary clipboard: {err:?}");
                ClipboardContent::null_value()
            }
        }
    }
}

impl LinuxClipboard {
    /// Parses Linux clipboard text for absolute file paths.
    ///
    /// When copying files, Linux file managers typically place the paths as text content onto
    /// the clipboard. We parse this text to extract absolute paths, but if ANY line is not an
    /// absolute path, we assume this is regular text content and return None (no paths).
    fn parse_valid_filepaths_from_text(&mut self, text_content: &str) -> Option<Vec<String>> {
        let mut file_paths = Vec::new();

        // Check for absolute filepaths
        for line in text_content.trim().lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let candidate_path_str = if let Some(uri_path) = line.strip_prefix("file://") {
                match urlencoding::decode(uri_path) {
                    Ok(decoded_path) => decoded_path.into_owned(),
                    Err(_) => uri_path.to_string(),
                }
            } else {
                line.to_string()
            };

            let candidate_path = std::path::Path::new(&candidate_path_str);
            if candidate_path.is_absolute() && candidate_path.exists() {
                file_paths.push(candidate_path_str);
            } else {
                // Not an absolute-path indicates the text was not from copying files, so return
                return None;
            }
        }

        if file_paths.is_empty() {
            None
        } else {
            Some(file_paths)
        }
    }

    /// Reads clipboard content from a specific clipboard buffer.
    fn read_from_specific_clipboard(
        &mut self,
        clipboard_kind: LinuxClipboardKind,
    ) -> Result<ClipboardContent, arboard::Error> {
        let text_result = self.inner.get().text();
        let mut content = ClipboardContent {
            plain_text: text_result.as_ref().map(|s| s.clone()).unwrap_or_default(),
            ..Default::default()
        };

        // Get file paths from clipboard (Linux-specific)
        content.paths = self.parse_valid_filepaths_from_text(&content.plain_text);

        // Attempt to use HTML data first.
        match self.inner.get().clipboard(clipboard_kind).html() {
            Ok(html) => {
                content.html = html.is_empty().not().then_some(html);

                // Try to get image content from clipboard
                content.images = crate::clipboard_utils::read_images_from_clipboard(
                    &mut self.inner,
                    &content.html,
                    &content.plain_text,
                );

                return Ok(content);
            }
            Err(err) => {
                log::info!(
                    "Unable to read HTML from clipboard: {err:?}, falling back to plaintext."
                );
            }
        }

        // Fallback to using plaintext
        content.images = crate::clipboard_utils::read_images_from_clipboard(
            &mut self.inner,
            &None, // No HTML in fallback case
            &content.plain_text,
        );

        // Return success if we have ANY content (text, paths, OR images)
        // Only error if ALL content types failed
        if text_result.is_ok()
            || content
                .paths
                .as_ref()
                .is_some_and(|paths| !paths.is_empty())
            || content.images.as_ref().is_some_and(|imgs| !imgs.is_empty())
        {
            Ok(content)
        } else {
            // All content types failed - return the text error
            text_result.map(|_| content)
        }
    }

    fn write_to_specific_clipboard(
        &mut self,
        clipboard_kind: LinuxClipboardKind,
        contents: &ClipboardContent,
    ) -> Result<(), arboard::Error> {
        if let Some(html) = &contents.html {
            self.inner
                .set()
                .clipboard(clipboard_kind)
                .html(html, Some(&contents.plain_text))
        } else {
            self.inner
                .set()
                .clipboard(clipboard_kind)
                .text(&contents.plain_text)
        }
    }
}

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;
