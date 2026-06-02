//! This crate defines shared SQLite persistence infrastructure for Warp cloud objects.
//!
//! It owns model-agnostic persistence helpers for object metadata, permissions, refresh
//! scheduling, guest and link-sharing encoding, callback-based object upsert and delete
//! operations, and generic string object table access.
//!
//! It should not depend on `cloud_object_models`; model-specific read and write adapters
//! should live with the corresponding model modules.

mod encoded_permissions;
mod objects;
mod refresh;

pub use encoded_permissions::{
    decode_guests, decode_link_sharing, encode_guests, encode_link_sharing,
};
pub use objects::{
    CloudObjectId, CloudObjectReadContext, CreateCloudObjectFn, DeleteCloudObjectFn,
    GenericStringObjectPersistenceData, GenericStringObjectRow, UpdateCloudObjectFn,
    delete_cloud_object, delete_generic_string_object, id_from_metadata, increment_retry_count,
    load_cloud_object_read_context, mark_object_as_synced, metadata_object_type_key,
    read_generic_string_object_rows, to_cloud_object_metadata, to_cloud_object_permissions,
    update_object_after_server_creation, update_object_metadata, upsert_cloud_object,
    upsert_generic_string_objects,
};
pub use refresh::{read_time_of_next_force_object_refresh, record_time_of_next_refresh};
