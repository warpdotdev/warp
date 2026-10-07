use js_sys::{Array, Object};
use wasm_bindgen::prelude::*;
use wasm_bindgen::{self, JsCast};
use web_sys::{Blob, BlobPropertyBag};

use crate::Clipboard;
use crate::clipboard::ClipboardContent;

pub struct WebClipboard {
    inner: web_sys::Clipboard,
    saved_content: ClipboardContent,
}

impl WebClipboard {
    pub fn new() -> Self {
        Self {
            inner: gloo::utils::window().navigator().clipboard(),
            saved_content: Default::default(),
        }
    }
}

impl Default for WebClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Clipboard for WebClipboard {
    fn write(&mut self, contents: ClipboardContent) {
        match create_item_list(&contents) {
            Ok(item_list) => {
                // This returns a Promise, which succeeds iff the copy succeeds. There's nothing we can do
                // if the copy fails, though, and this API doesn't support async, so we just ignore the
                // promise. It's not necessary to hold a reference to the promise for the copy to succeed.
                let _ = self.inner.write(&item_list);
            }
            Err(error) => {
                // Fall back to just writing plain text.
                // ClipboardItems are not supported in Firefox yet.
                log::warn!("Failed to construct clipboard data: {error:?}");
                let _ = self.inner.write_text(&contents.plain_text);
            }
        }
    }

    fn read(&mut self) -> ClipboardContent {
        std::mem::take(&mut self.saved_content)
    }

    fn save(&mut self, content: ClipboardContent) {
        self.saved_content = content;
    }
}

fn create_item_list(contents: &ClipboardContent) -> Result<Array, JsValue> {
    // The Clipboard.write method
    // (https://developer.mozilla.org/en-US/docs/Web/API/Clipboard/write)
    // requires an array of ClipboardItem objects
    // (https://developer.mozilla.org/en-US/docs/Web/API/ClipboardItem).
    // This function constructs a single element array containing a ClipboardItem, which contains
    // both plain text and html data that's being copied.

    let items = Object::new();

    // We always have plain text data.
    let text_blob = create_blob(&contents.plain_text, "text/plain")?;
    js_sys::Reflect::set(&items, &JsValue::from_str("text/plain"), &text_blob)?;

    // We sometimes have html data.
    if let Some(html) = &contents.html {
        let html_blob = create_blob(html, "text/html")?;
        js_sys::Reflect::set(&items, &JsValue::from_str("text/html"), &html_blob)?;
    }

    // web_sys doesn't have this constructor, so we have to do things the hard way.
    let clipboard_item_constructor: js_sys::Function = js_sys::Reflect::get(
        &JsValue::from(gloo::utils::window()),
        &JsValue::from_str("ClipboardItem"),
    )?
    .dyn_into()?;
    let clipboard_item =
        js_sys::Reflect::construct(&clipboard_item_constructor, &Array::of1(&items))?;

    // Write the ClipboardItem to the clipboard
    let item_list = Array::new();
    item_list.push(&clipboard_item);

    Ok(item_list)
}

fn create_blob(contents: &str, type_: &str) -> Result<Blob, JsValue> {
    // See the JS Blob constructor docs for more info:
    // https://developer.mozilla.org/en-US/docs/Web/API/Blob/Blob
    let blob_parts = Array::new();
    blob_parts.push(&JsValue::from_str(contents));
    let blob_opts = BlobPropertyBag::new();
    blob_opts.set_type(type_);
    Blob::new_with_str_sequence_and_options(&blob_parts, &blob_opts)
}
