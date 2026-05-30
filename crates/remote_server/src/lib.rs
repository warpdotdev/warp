pub mod auth;
pub mod client;
pub mod codebase_index_proto;
pub mod host_id;
pub mod manager;
pub mod protocol;
pub mod repo_metadata_proto;
pub mod setup;
#[cfg(not(target_family = "wasm"))]
pub mod ssh;
pub mod transport;

pub use host_id::HostId;

#[allow(clippy::large_enum_variant)]
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/remote_server.rs"));
}
