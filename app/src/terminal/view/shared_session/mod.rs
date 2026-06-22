//! Session-sharing logic related to the terminal view.

pub(in crate::terminal::view) mod adapter;
pub(in crate::terminal::view) mod cloud_conversation_continuation;
mod conversation_ended_tombstone_view;
pub(in crate::terminal::view) mod sharer;
#[cfg(test)]
pub mod test_utils;
mod view_impl;
mod viewer;

pub(in crate::terminal::view) use adapter::Adapter as SharedSessionAdapter;
pub(in crate::terminal::view) use conversation_ended_tombstone_view::{
    ConversationEndedTombstoneEvent, ConversationEndedTombstoneView,
};
pub(in crate::terminal::view) use viewer::Viewer;
