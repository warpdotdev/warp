//! Image path parsing and clipboard/file processing for TUI attachments.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose;
use warp::tui_export::{
    ImageContext, MAX_IMAGE_SIZE_BYTES, MIME_SNIFF_BYTES, ProcessImageResult, infer_mime_type,
    is_supported_image_mime_type, process_image_for_agent,
};
use warpui_core::clipboard::{ClipboardContent, ImageData};
use warpui_core::clipboard_utils::CLIPBOARD_IMAGE_MIME_TYPES;

pub(super) enum ClipboardPasteContent {
    Image(ClipboardContent),
    ImagePaths {
        paths: Vec<PathBuf>,
        original_text: String,
    },
    Text(String),
    Empty,
}

pub(super) fn parse_image_paths(text: &str, cwd: &Path) -> Option<Vec<PathBuf>> {
    let tokens = shell_words::split(text.trim()).ok()?;
    if tokens.is_empty() {
        return None;
    }
    tokens
        .into_iter()
        .map(|token| resolve_image_path(&token, cwd))
        .collect()
}

pub(super) fn classify_clipboard_content(
    content: ClipboardContent,
    cwd: &Path,
) -> ClipboardPasteContent {
    if content.has_image_data() {
        return ClipboardPasteContent::Image(content);
    }

    let original_text = if content.plain_text.is_empty() {
        content
            .paths
            .as_ref()
            .map(|paths| paths.join("\n"))
            .unwrap_or_default()
    } else {
        content.plain_text.clone()
    };
    if let Some(paths) = content.paths.as_ref()
        && !paths.is_empty()
        && let Some(paths) = paths
            .iter()
            .map(|path| resolve_image_path(path, cwd))
            .collect()
    {
        return ClipboardPasteContent::ImagePaths {
            paths,
            original_text,
        };
    }
    if let Some(paths) = parse_image_paths(&content.plain_text, cwd) {
        return ClipboardPasteContent::ImagePaths {
            paths,
            original_text,
        };
    }
    if original_text.is_empty() {
        ClipboardPasteContent::Empty
    } else {
        ClipboardPasteContent::Text(original_text)
    }
}

fn resolve_image_path(token: &str, cwd: &Path) -> Option<PathBuf> {
    let token = token.strip_prefix("file://").unwrap_or(token);
    let path = if token == "~" {
        dirs::home_dir()?
    } else if let Some(rest) = token.strip_prefix("~/") {
        dirs::home_dir()?.join(rest)
    } else {
        PathBuf::from(token)
    };
    let path = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp").then_some(path)
}

pub(super) async fn process_paths(paths: Vec<PathBuf>) -> Result<Vec<ImageContext>, String> {
    let mut images = Vec::with_capacity(paths.len());
    for path in paths {
        let metadata = async_fs::metadata(&path)
            .await
            .map_err(|_| format!("Could not read image {}.", path.display()))?;
        if !metadata.is_file() {
            return Err(format!("Image path is not a file: {}.", path.display()));
        }
        if metadata.len() > u64::try_from(MAX_IMAGE_SIZE_BYTES).unwrap_or(u64::MAX) {
            return Err(format!("Image is too large: {}.", path.display()));
        }
        let bytes = async_fs::read(&path)
            .await
            .map_err(|_| format!("Could not read image {}.", path.display()))?;
        let mime_type = infer_mime_type(&path, &bytes[..bytes.len().min(MIME_SNIFF_BYTES)]);
        if !is_supported_image_mime_type(&mime_type) {
            return Err(format!(
                "Unsupported image type for {}. Use PNG, JPG, GIF, or WebP.",
                path.display()
            ));
        }
        let data = match process_image_for_agent(&bytes) {
            ProcessImageResult::Success { data } => data,
            ProcessImageResult::TooLarge => {
                return Err(format!("Image is too large: {}.", path.display()));
            }
            ProcessImageResult::Error(_) => {
                return Err(format!("Could not process image {}.", path.display()));
            }
        };
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return Err(format!("Image has no valid filename: {}.", path.display()));
        };
        images.push(ImageContext {
            data: general_purpose::STANDARD.encode(data),
            mime_type,
            file_name: file_name.to_owned(),
            is_figma: false,
        });
    }
    Ok(images)
}

pub(super) async fn read_clipboard_content() -> Result<ClipboardContent, String> {
    blocking::unblock(|| -> Result<ClipboardContent, String> {
        let mut clipboard = warpui::platform::create_system_clipboard()
            .map_err(|_| "The system clipboard is unavailable.".to_owned())?;
        Ok(clipboard.read())
    })
    .await
}

pub(super) fn process_clipboard_content(content: ClipboardContent) -> Result<ImageContext, String> {
    let images = content
        .images
        .ok_or_else(|| "Clipboard image data is unavailable.".to_owned())?;
    let image = CLIPBOARD_IMAGE_MIME_TYPES
        .iter()
        .find_map(|mime_type| {
            images
                .iter()
                .find(|image| image.mime_type == *mime_type)
                .cloned()
        })
        .ok_or_else(|| "The clipboard does not contain a supported image.".to_owned())?;
    process_clipboard_image_data(image)
}

fn process_clipboard_image_data(image: ImageData) -> Result<ImageContext, String> {
    let data = match process_image_for_agent(&image.data) {
        ProcessImageResult::Success { data } => data,
        ProcessImageResult::TooLarge => return Err("The clipboard image is too large.".to_owned()),
        ProcessImageResult::Error(_) => {
            return Err("The clipboard image could not be processed.".to_owned());
        }
    };
    Ok(ImageContext {
        data: general_purpose::STANDARD.encode(data),
        mime_type: image.mime_type,
        file_name: image
            .filename
            .unwrap_or_else(|| "clipboard-image.png".to_owned()),
        is_figma: false,
    })
}

#[cfg(test)]
#[path = "image_processing_tests.rs"]
mod tests;
