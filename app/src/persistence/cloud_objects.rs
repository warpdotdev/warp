//! Supporting types for persisting cloud objects to SQLite.

#[cfg(test)]
pub use warp_server_client::persistence::encode_guests;
pub use warp_server_client::persistence::{decode_guests, decode_link_sharing};

#[cfg(test)]
#[path = "cloud_object_tests.rs"]
mod tests;
