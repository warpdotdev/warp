use std::sync::Arc;

use crate::{ClipboardType, ImageProtocol};

#[derive(Clone)]
pub enum Event {
    MouseCursorDirty,
    ClipboardStore(ClipboardType, String),
    ClipboardLoad(
        ClipboardType,
        Arc<dyn Fn(&str) -> String + Sync + Send + 'static>,
    ),
    CursorBlinkingChange(bool),
    Bell,
    ImageReceived {
        image_id: u32,
        image_data: Vec<u8>,
        image_protocol: ImageProtocol,
    },
}
