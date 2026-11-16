use warpui::clipboard::ClipboardContent;
use warpui::integration::TestStep;

use super::assert_clipboard_contains_string;

pub fn write_to_clipboard(text: String) -> TestStep {
    let expected = text.clone();
    TestStep::new("Write text to the clipboard")
        .with_action(move |app, _, _| {
            app.update(|app| {
                app.clipboard()
                    .write(ClipboardContent::plain_text(text.clone()))
            })
        })
        .add_named_assertion(
            "Ensure the clipboard contains the text",
            assert_clipboard_contains_string(expected),
        )
}
