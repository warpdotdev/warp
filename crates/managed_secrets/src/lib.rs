pub mod client;
mod envelope;
mod gcp;
mod manager;
mod secret_value;

pub use client::TaskIdentityToken;
pub use envelope::{UploadKey, init as init_envelope};
pub use gcp::{
    GcpCredentials, GcpFederationConfig, GcpWorkloadIdentityFederationError,
    GcpWorkloadIdentityFederationToken, PrepareGcpCredentialsError,
};
pub use manager::{ActorProvider, ManagedSecretManager, convert_task_secrets};
pub use secret_value::ManagedSecretValue;
