// Re-export types from warp_server_client.
pub use warp_server_client::ids::{
    parse_sqlite_id_to_uid, ApiKeyUid, ClientId, HashableId, HashedSqliteId, ObjectUid, ServerId,
    ServerIdAndType, SyncId, ToServerId,
};

/// server_id_traits is a macro used for generating implementations for the type aliases on
/// ServerId. It implements different To/From and Display, and HashableId traits.
/// Takes type and desired prefix for HashableId.
///
/// Note: This macro uses `$crate::server::ids::*` paths, so it only works within the warp crate.
/// For types defined in warp_server_client, use `warp_server_client::server_id_traits!` instead.
#[macro_export]
macro_rules! server_id_traits {
    ($t:ty, $prefix:literal) => {
        #[cfg(test)]
        impl From<i64> for $t {
            fn from(id: i64) -> Self {
                Self(id.into())
            }
        }

        impl From<String> for $t {
            fn from(id: String) -> Self {
                Self($crate::server::ids::ServerId::from_string_lossy(id))
            }
        }

        impl From<$t> for String {
            fn from(id: $t) -> String {
                id.0.into()
            }
        }

        impl std::fmt::Display for $t {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
                write!(f, "{}", self.0)
            }
        }

        impl From<$t> for $crate::server::ids::ServerId {
            fn from(id: $t) -> Self {
                id.0
            }
        }

        impl $crate::server::ids::HashableId for $t {
            fn to_hash(&self) -> String {
                format!("{}-{}", $prefix, self)
            }

            fn from_hash(hash: &str) -> Option<$t> {
                hash.strip_prefix(&format!("{}-", $prefix))
                    .map(|s| s.to_string().into())
            }
        }

        impl From<$crate::server::ids::ServerId> for $t {
            fn from(id: $crate::server::ids::ServerId) -> Self {
                Self(id)
            }
        }

        impl $crate::server::ids::ToServerId for $t {
            fn to_server_id(&self) -> $crate::server::ids::ServerId {
                self.0
            }
        }
    };
}

#[cfg(test)]
#[path = "ids_tests.rs"]
mod tests;
