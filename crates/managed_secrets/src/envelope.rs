use std::io;
use std::sync::Once;

use base64::Engine;
use warp_graphql::managed_secrets::ManagedSecretType;

use crate::secret_value::ManagedSecretValue;

mod hpke_impl;

static INIT: Once = Once::new();

/// Initialize cryptography providers for secret enveloping. This is safe to call multiple times.
pub fn init() {
    INIT.call_once(|| {
        tink_hybrid::init();

        use std::sync::Arc;

        use hpke_impl::{HpkePrivateKeyManager, HpkePublicKeyManager};
        tink_core::registry::register_key_manager(Arc::new(HpkePublicKeyManager::new()))
            .expect("unable to register HPKE public key manager");
        tink_core::registry::register_key_manager(Arc::new(HpkePrivateKeyManager::new()))
            .expect("unable to register HPKE private key manager");
    });
}

#[derive(thiserror::Error, Debug)]
pub enum EnvelopeError {
    #[error("invalid public key")]
    InvalidPublicKey(#[source] anyhow::Error),
    #[error("cryptography operation failed")]
    Tink(#[from] tink_core::TinkError),
    #[error("failed to marshal secret value")]
    MarshalSecretValue(#[source] serde_json::Error),
}

/// UploadKey is the client-side (public) representation of the tink keysets used to encrypt
/// secrets before uploading to the server.
pub struct UploadKey {
    #[allow(dead_code)]
    public_key: tink_core::keyset::Handle,
    encrypt: Box<dyn tink_core::HybridEncrypt>,
}

impl UploadKey {
    pub fn import_public_keyset(public_key: &str) -> Result<Self, EnvelopeError> {
        let key_bytes = base64::prelude::BASE64_STANDARD
            .decode(public_key)
            .map_err(|e| EnvelopeError::InvalidPublicKey(anyhow::anyhow!(e)))?;
        let mut key_reader = tink_core::keyset::BinaryReader::new(io::Cursor::new(key_bytes));

        let keyset = tink_core::keyset::Handle::read_with_no_secrets(&mut key_reader)?;

        let primitive = tink_hybrid::new_encrypt(&keyset)?;
        Ok(UploadKey {
            public_key: keyset,
            encrypt: primitive,
        })
    }

    /// Encrypt a secret value for uploading to the server.
    pub fn encrypt_secret(
        &self,
        actor_uid: &str,
        secret_name: &str,
        secret: &ManagedSecretValue,
    ) -> Result<String, EnvelopeError> {
        let context = UploadContext {
            actor_uid,
            secret_name,
            secret_type: secret.secret_type(),
        };

        let secret_plaintext =
            serde_json::to_vec(secret).map_err(EnvelopeError::MarshalSecretValue)?;

        let encrypted = self
            .encrypt
            .encrypt(&secret_plaintext, context.encode().as_bytes())?;
        Ok(base64::prelude::BASE64_STANDARD.encode(encrypted))
    }
}

struct UploadContext<'a> {
    actor_uid: &'a str,
    secret_name: &'a str,
    secret_type: ManagedSecretType,
}

impl<'a> UploadContext<'a> {
    fn encode(&self) -> String {
        // This must match the context encoding format used by the server.
        format!(
            "1:{}:{}:{}",
            self.actor_uid,
            self.secret_name,
            self.secret_type.envelope_name()
        )
    }
}

#[cfg(test)]
#[path = "envelope_tests.rs"]
mod tests;
