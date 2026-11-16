use parking_lot::Mutex;

/// A Clipboard that can read and write strings. Each platform must implement this trait to support
/// writing to a clipboard.
pub trait Clipboard: 'static {
    fn write(&mut self, contents: ClipboardContent);

    fn read(&mut self) -> ClipboardContent;

    /// Writes to the primary clipboard, used to support the Primary Selection Protocol (middle-click paste).
    ///
    /// NOTE: For platforms that don't support the primary clipboard, it writes to the default clipboard instead.
    fn write_to_primary_clipboard(&mut self, contents: ClipboardContent) {
        self.write(contents)
    }

    /// Reads the primary clipboard, used to support the Primary Selection Protocol (middle-click paste).
    /// This reads from the default clipboard on platforms other than Linux.
    fn read_from_primary_clipboard(&mut self) -> ClipboardContent {
        self.read()
    }

    #[cfg(target_family = "wasm")]
    fn save(&mut self, content: ClipboardContent);
}

// Clipboard could contain content with multiple data types at the same type.
#[derive(Debug, Clone, Default)]
pub struct ClipboardContent {
    // Clipboard contains plain string.
    pub plain_text: String,
    // Clipboard contains a list of file paths.
    // Parsed direct from OS clipboard on Mac/Windows, from plain_text on Linux.
    // On Mac/Linux, plain_text may also be populated.
    pub paths: Option<Vec<String>>,
    // Clipboard contains HTML content.
    pub html: Option<String>,
    // Clipboard contains image data (can be multiple images).
    pub images: Option<Vec<ImageData>>,
}

/// Represents image data from the clipboard.
///
/// Contains the raw image bytes and associated MIME type information.
#[derive(Debug, Clone)]
pub struct ImageData {
    /// Raw image data as bytes.
    pub data: Vec<u8>,
    /// MIME type of the image (e.g., "image/png", "image/jpeg").
    pub mime_type: String,
    /// Original filename if available (e.g., "photo.jpg").
    pub filename: Option<String>,
}

impl ClipboardContent {
    pub fn plain_text(text: String) -> Self {
        Self {
            plain_text: text,
            paths: Default::default(),
            html: Default::default(),
            images: Default::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        let Self {
            plain_text,
            paths,
            html,
            images,
        } = self;
        plain_text.is_empty() && paths.is_none() && html.is_none() && images.is_none()
    }

    pub fn has_image_data(&self) -> bool {
        self.images
            .as_ref()
            .map(|images| !images.is_empty())
            .unwrap_or(false)
    }

    pub fn num_paths(&self) -> usize {
        self.paths.as_ref().map(|paths| paths.len()).unwrap_or(0)
    }

    /// Check if clipboard contains file paths that are not images
    pub fn has_non_image_filepaths(&self) -> bool {
        self.paths
            .as_ref()
            .map(|paths| {
                paths
                    .iter()
                    .any(|path| !crate::clipboard_utils::has_image_extension(path))
            })
            .unwrap_or(false)
    }
}

pub fn should_insert_text_on_paste(content: &ClipboardContent) -> bool {
    // Insert any text content present when:
    // 1. No images at all (neither data nor paths)
    // 2. Has non-image files (mixed content)
    // 3. Has image data but no file paths (direct image paste)
    if !content.has_image_data() && content.num_paths() == 0 {
        return true; // No images at all
    }
    if content.has_non_image_filepaths() {
        return true; // Mixed content - user likely wants text paths
    }
    // Direct image paste - user would still want any text content present (unless paths)
    content.has_image_data() && content.num_paths() == 0
}

/// Stores clipboard content in the heap of this process. Therefore, it is scoped to this process.
/// This is not a proper implementation for a "real" platform. It's useful in tests or as a
/// temporary substitute.
pub struct InMemoryClipboard {
    clipboard_content: Mutex<ClipboardContent>,
}

impl Default for InMemoryClipboard {
    fn default() -> Self {
        Self {
            clipboard_content: Mutex::new(ClipboardContent::plain_text(String::new())),
        }
    }
}

impl Clipboard for InMemoryClipboard {
    fn write(&mut self, contents: ClipboardContent) {
        *self.clipboard_content.lock() = contents;
    }

    fn read(&mut self) -> ClipboardContent {
        self.clipboard_content.lock().clone()
    }

    #[cfg(target_family = "wasm")]
    fn save(&mut self, _content: ClipboardContent) {}
}
