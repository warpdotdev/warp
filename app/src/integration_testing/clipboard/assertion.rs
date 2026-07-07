use warpui::async_assert_eq;
use warpui::integration::AssertionCallback;

pub fn assert_clipboard_contains_string(string: String) -> AssertionCallback {
    Box::new(move |app, _window_id| {
        let clipboard = app.update(|ctx| ctx.clipboard().read());
        let content = match clipboard.paths {
            Some(paths) => paths.join(" "),
            None => clipboard.plain_text,
        };

        async_assert_eq!(content, string)
    })
}
