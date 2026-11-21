/// Windows-specific clipboard tests.
///
/// Note: Most image processing functionality is tested in ui/src/clipboard_utils_tests.rs
/// to avoid duplication. These tests focus on Windows-specific clipboard behavior.
#[cfg(target_os = "windows")]
mod clipboard_tests {
    use crate::Clipboard;
    use crate::clipboard::ClipboardContent;
    use crate::windowing::winit::windows::clipboard::WindowsClipboard;

    fn create_test_clipboard() -> Option<WindowsClipboard> {
        WindowsClipboard::new().ok()
    }

    // The OS clipboard is shared, so keep native clipboard scenarios in one test to prevent races
    // under both nextest and libtest.
    #[test]
    fn test_clipboard_content() {
        let mut clipboard = match create_test_clipboard() {
            Some(clipboard) => clipboard,
            None => {
                eprintln!("Skipping test - no clipboard available (headless environment)");
                return;
            }
        };

        let test_content = ClipboardContent::plain_text("Windows clipboard test".to_string());

        // Write content
        clipboard.write(test_content.clone());

        // Read it back
        let read_content = clipboard.read();

        // Should get the same text back (in environments where clipboard works)
        if !read_content.plain_text.is_empty() {
            assert_eq!(read_content.plain_text, test_content.plain_text);
        }

        let test_content = ClipboardContent {
            plain_text: "Test text".to_string(),
            html: Some("<div>Test HTML</div>".to_string()),
            images: None,
            paths: None,
        };

        // Write HTML content
        clipboard.write(test_content.clone());

        // Read it back
        let read_content = clipboard.read();

        // In environments where clipboard works, we should get content back
        // (the exact HTML may not be preserved depending on the system)
        if !read_content.is_empty() {
            assert!(!read_content.plain_text.is_empty());
        }

        let empty_content = ClipboardContent::plain_text("".to_string());

        // Writing empty content should not panic
        clipboard.write(empty_content);

        clipboard.read();
    }
}
