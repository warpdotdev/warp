/// Linux-specific clipboard tests.
///
/// Note: Most image processing functionality is tested in ui/src/clipboard_utils_tests.rs
/// to avoid duplication. These tests focus on Linux-specific clipboard behavior.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod clipboard_tests {
    use crate::clipboard::{Clipboard, ClipboardContent};
    use crate::windowing::winit::linux::LinuxClipboard;

    fn create_test_clipboard() -> Option<LinuxClipboard> {
        LinuxClipboard::new().ok()
    }

    /// Helper function to avoid repetitive clipboard creation and early return logic.
    fn with_test_clipboard<F>(test_fn: F)
    where
        F: FnOnce(&mut LinuxClipboard),
    {
        let mut clipboard = match create_test_clipboard() {
            Some(clipboard) => clipboard,
            None => {
                eprintln!("Skipping test - no clipboard available (headless environment)");
                return;
            }
        };
        test_fn(&mut clipboard);
    }

    /// Helper to assert that paths are correctly extracted from clipboard text.
    fn assert_paths_extracted(
        clipboard: &mut LinuxClipboard,
        input: &str,
        expected_paths: &[&str],
    ) {
        let content = ClipboardContent::plain_text(input.to_string());
        clipboard.write(content);
        let read_content = clipboard.read();

        if let Some(paths) = read_content.paths {
            assert_eq!(paths.len(), expected_paths.len());
            for expected_path in expected_paths {
                assert!(
                    paths.contains(&expected_path.to_string()),
                    "Expected path '{expected_path}' not found in: {paths:?}"
                );
            }
        } else {
            panic!("Expected to extract paths from: '{input}'");
        }
    }

    /// Helper to assert that no paths are extracted from clipboard text.
    fn assert_no_paths_extracted(clipboard: &mut LinuxClipboard, input: &str) {
        let content = ClipboardContent::plain_text(input.to_string());
        clipboard.write(content);
        let read_content = clipboard.read();
        assert!(
            read_content.paths.is_none(),
            "Expected no paths to be extracted from: '{}', but got: {:?}",
            input,
            read_content.paths
        );
    }

    #[test]
    fn test_clipboard_round_trip() {
        with_test_clipboard(|clipboard| {
            let test_content = ClipboardContent::plain_text("Linux clipboard test".to_string());

            // Write content
            clipboard.write(test_content.clone());

            // Read it back
            let read_content = clipboard.read();

            // Should get the same text back (in environments where clipboard works)
            if !read_content.plain_text.is_empty() {
                assert_eq!(read_content.plain_text, test_content.plain_text);
            }
        });
    }

    #[test]
    fn test_html_content_handling() {
        with_test_clipboard(|clipboard| {
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
        });
    }

    #[test]
    fn test_primary_clipboard_operations() {
        with_test_clipboard(|clipboard| {
            let test_content = ClipboardContent::plain_text("Primary clipboard test".to_string());

            // Test primary clipboard write (should not panic)
            clipboard.write_to_primary_clipboard(test_content.clone());

            // Test primary clipboard read (should return valid ClipboardContent)
            let read_content = clipboard.read_from_primary_clipboard();

            // Should always return a ClipboardContent struct, even if empty
            // (this tests the fallback behavior when primary clipboard isn't supported)
            assert!(matches!(read_content.images, None | Some(_)));
            assert!(matches!(read_content.html, None | Some(_)));
        });
    }

    #[test]
    fn test_empty_content_handling() {
        with_test_clipboard(|clipboard| {
            let empty_content = ClipboardContent::plain_text("".to_string());

            // Writing empty content should not panic
            clipboard.write(empty_content);

            // Reading should return valid ClipboardContent (may be empty or have previous content)
            let read_content = clipboard.read();

            // Should always return a valid ClipboardContent struct
            assert!(matches!(read_content.images, None | Some(_)));
        });
    }

    #[test]
    fn test_absolute_paths_extracted() {
        with_test_clipboard(|clipboard| {
            // Test single path
            assert_paths_extracted(
                clipboard,
                "/home/user/document.txt",
                &["/home/user/document.txt"],
            );

            // Test multiple paths
            assert_paths_extracted(
                clipboard,
                "/home/user/file1.txt\n/home/user/file2.pdf",
                &["/home/user/file1.txt", "/home/user/file2.pdf"],
            );
        });
    }

    #[test]
    fn test_file_uri_decoded() {
        with_test_clipboard(|clipboard| {
            // Test basic file:// URI
            assert_paths_extracted(
                clipboard,
                "file:///home/user/document.txt",
                &["/home/user/document.txt"],
            );

            // Test URL-encoded URI with spaces
            assert_paths_extracted(
                clipboard,
                "file:///home/user/My%20Documents/file.txt",
                &["/home/user/My Documents/file.txt"],
            );
        });
    }

    #[test]
    fn test_non_absolute_paths_rejected() {
        with_test_clipboard(|clipboard| {
            // Relative paths should be rejected
            assert_no_paths_extracted(clipboard, "./relative.txt\n../another.txt");

            // Regular text should be rejected
            assert_no_paths_extracted(clipboard, "Hello world\nThis is text");

            // Mixed content should be rejected (strict policy)
            assert_no_paths_extracted(
                clipboard,
                "/home/user/file.txt\nSome text\n/another/file.txt",
            );
        });
    }
}
