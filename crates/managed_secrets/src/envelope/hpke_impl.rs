//! HPKE key manager implementations using the pure-Rust [`hpke`] crate.
//!
//! Supports both encryption (public key manager) and decryption (private key manager).

use hpke::aead::{AesGcm128, AesGcm256, ChaCha20Poly1305};
use hpke::kdf::HkdfSha256;
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable, OpModeR, OpModeS, Serializable};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use tink_core::TinkError;
use tink_proto::prost::Message;
use tink_proto::{HpkeAead, HpkeKdf, HpkeKem};

pub const HPKE_PUBLIC_KEY_TYPE_URL: &str = "type.googleapis.com/google.crypto.tink.HpkePublicKey";
pub const HPKE_PRIVATE_KEY_TYPE_URL: &str = "type.googleapis.com/google.crypto.tink.HpkePrivateKey";
const HPKE_PUBLIC_KEY_KEY_VERSION: u32 = 0;
const HPKE_PRIVATE_KEY_KEY_VERSION: u32 = 0;

// ── Cipher suite ────────────────────────────────────────────────────────────

/// Validated HPKE cipher suite. Adding a new combination requires adding a
/// variant here and handling it in both [`HpkeSuite::seal`] and
/// [`HpkeSuite::open`], so the compiler enforces exhaustive handling.
#[derive(Clone, Copy)]
enum HpkeSuite {
    X25519Sha256Aes256Gcm,
    X25519Sha256Aes128Gcm,
    X25519Sha256Chacha20Poly1305,
}

impl HpkeSuite {
    fn seal(
        self,
        public_key: &<X25519HkdfSha256 as hpke::Kem>::PublicKey,
        context_info: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, TinkError> {
        let mut rng = StdRng::from_os_rng();

        macro_rules! do_seal {
            ($aead_ty:ty) => {{
                let (encapped_key, ciphertext) =
                    hpke::single_shot_seal::<$aead_ty, HkdfSha256, X25519HkdfSha256, _>(
                        &OpModeS::Base,
                        public_key,
                        context_info,
                        plaintext,
                        &[],
                        &mut rng,
                    )
                    .map_err(|e| TinkError::from(format!("HpkeSuite::seal failed: {e:?}")))?;
                let mut output = encapped_key.to_bytes().to_vec();
                output.extend_from_slice(&ciphertext);
                Ok(output)
            }};
        }

        match self {
            HpkeSuite::X25519Sha256Aes256Gcm => do_seal!(AesGcm256),
            HpkeSuite::X25519Sha256Aes128Gcm => do_seal!(AesGcm128),
            HpkeSuite::X25519Sha256Chacha20Poly1305 => do_seal!(ChaCha20Poly1305),
        }
    }

    fn open(
        self,
        private_key: &<X25519HkdfSha256 as hpke::Kem>::PrivateKey,
        context_info: &[u8],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, TinkError> {
        let (enc_bytes, encrypted_data) = ciphertext
            .split_at_checked(X25519_ENCAPPED_KEY_LEN)
            .ok_or_else(|| TinkError::from("HpkeSuite::open: ciphertext too short"))?;

        macro_rules! do_open {
            ($aead_ty:ty) => {{
                let encapped_key =
                    <X25519HkdfSha256 as hpke::Kem>::EncappedKey::from_bytes(enc_bytes)
                        .map_err(|_| TinkError::new("HpkeSuite::open: invalid encapped key"))?;
                hpke::single_shot_open::<$aead_ty, HkdfSha256, X25519HkdfSha256>(
                    &OpModeR::Base,
                    private_key,
                    &encapped_key,
                    context_info,
                    encrypted_data,
                    &[],
                )
                .map_err(|e| TinkError::from(format!("HpkeSuite::open failed: {e:?}")))
            }};
        }

        match self {
            HpkeSuite::X25519Sha256Aes256Gcm => do_open!(AesGcm256),
            HpkeSuite::X25519Sha256Aes128Gcm => do_open!(AesGcm128),
            HpkeSuite::X25519Sha256Chacha20Poly1305 => do_open!(ChaCha20Poly1305),
        }
    }
}

/// The X25519 encapsulated key is always 32 bytes.
const X25519_ENCAPPED_KEY_LEN: usize = 32;

// ── Public key manager (encryption) ─────────────────────────────────────────

pub(crate) struct HpkePublicKeyManager;

impl HpkePublicKeyManager {
    pub fn new() -> Self {
        Self
    }
}

impl tink_core::registry::KeyManager for HpkePublicKeyManager {
    fn primitive(&self, serialized_key: &[u8]) -> Result<tink_core::Primitive, TinkError> {
        if serialized_key.is_empty() {
            return Err(TinkError::new("HpkePublicKeyManager: invalid key"));
        }

        let key = tink_proto::HpkePublicKey::decode(serialized_key).map_err(|e| {
            TinkError::from(format!("HpkePublicKeyManager: invalid public key: {e:#}"))
        })?;
        let suite = validate_public_key(&key)?;

        let pk = <X25519HkdfSha256 as hpke::Kem>::PublicKey::from_bytes(&key.public_key).map_err(
            |_| TinkError::new("HpkePublicKeyManager: failed to deserialize public key"),
        )?;

        Ok(tink_core::Primitive::HybridEncrypt(Box::new(
            HpkeHybridEncrypt {
                public_key: pk,
                suite,
            },
        )))
    }

    fn new_key(&self, _serialized_key_format: &[u8]) -> Result<Vec<u8>, TinkError> {
        Err(TinkError::new(
            "HpkePublicKeyManager: new_key not implemented",
        ))
    }

    fn type_url(&self) -> &'static str {
        HPKE_PUBLIC_KEY_TYPE_URL
    }

    fn key_material_type(&self) -> tink_proto::key_data::KeyMaterialType {
        tink_proto::key_data::KeyMaterialType::AsymmetricPublic
    }
}

#[derive(Clone)]
struct HpkeHybridEncrypt {
    public_key: <X25519HkdfSha256 as hpke::Kem>::PublicKey,
    suite: HpkeSuite,
}

impl tink_core::HybridEncrypt for HpkeHybridEncrypt {
    fn encrypt(&self, plaintext: &[u8], context_info: &[u8]) -> Result<Vec<u8>, TinkError> {
        self.suite.seal(&self.public_key, context_info, plaintext)
    }
}

// ── Private key manager (decryption) ────────────────────────────────────────

pub(crate) struct HpkePrivateKeyManager;

impl HpkePrivateKeyManager {
    pub fn new() -> Self {
        Self
    }
}

impl tink_core::registry::KeyManager for HpkePrivateKeyManager {
    fn primitive(&self, serialized_key: &[u8]) -> Result<tink_core::Primitive, TinkError> {
        if serialized_key.is_empty() {
            return Err(TinkError::new("HpkePrivateKeyManager: invalid key"));
        }

        let key = tink_proto::HpkePrivateKey::decode(serialized_key).map_err(|e| {
            TinkError::from(format!("HpkePrivateKeyManager: invalid private key: {e:#}"))
        })?;
        let suite = validate_private_key(&key)?;

        let sk = <X25519HkdfSha256 as hpke::Kem>::PrivateKey::from_bytes(&key.private_key)
            .map_err(|_| {
                TinkError::new("HpkePrivateKeyManager: failed to deserialize private key")
            })?;

        Ok(tink_core::Primitive::HybridDecrypt(Box::new(
            HpkeHybridDecrypt {
                private_key: sk,
                suite,
            },
        )))
    }

    fn new_key(&self, _serialized_key_format: &[u8]) -> Result<Vec<u8>, TinkError> {
        Err(TinkError::new(
            "HpkePrivateKeyManager: new_key not implemented",
        ))
    }

    fn type_url(&self) -> &'static str {
        HPKE_PRIVATE_KEY_TYPE_URL
    }

    fn key_material_type(&self) -> tink_proto::key_data::KeyMaterialType {
        tink_proto::key_data::KeyMaterialType::AsymmetricPrivate
    }

    fn supports_private_keys(&self) -> bool {
        true
    }

    fn public_key_data(
        &self,
        serialized_priv_key: &[u8],
    ) -> Result<tink_proto::KeyData, TinkError> {
        let priv_key = tink_proto::HpkePrivateKey::decode(serialized_priv_key).map_err(|e| {
            TinkError::from(format!("HpkePrivateKeyManager: invalid private key: {e:#}"))
        })?;
        let mut serialized_pub_key = Vec::new();
        priv_key
            .public_key
            .ok_or_else(|| TinkError::new("HpkePrivateKeyManager: no public key"))?
            .encode(&mut serialized_pub_key)
            .map_err(|e| {
                TinkError::from(format!("HpkePrivateKeyManager: invalid public key: {e:#}"))
            })?;
        Ok(tink_proto::KeyData {
            type_url: HPKE_PUBLIC_KEY_TYPE_URL.to_string(),
            value: serialized_pub_key,
            key_material_type: tink_proto::key_data::KeyMaterialType::AsymmetricPublic.into(),
        })
    }
}

/// The `x25519-dalek` `StaticSecret` type already implements `Zeroize`, so the
/// private key bytes are securely cleared when this struct is dropped.
#[derive(Clone)]
struct HpkeHybridDecrypt {
    private_key: <X25519HkdfSha256 as hpke::Kem>::PrivateKey,
    suite: HpkeSuite,
}

impl tink_core::HybridDecrypt for HpkeHybridDecrypt {
    fn decrypt(&self, ciphertext: &[u8], context_info: &[u8]) -> Result<Vec<u8>, TinkError> {
        self.suite.open(&self.private_key, context_info, ciphertext)
    }
}

// ── Validation helpers ──────────────────────────────────────────────────────

fn validate_public_key(key: &tink_proto::HpkePublicKey) -> Result<HpkeSuite, TinkError> {
    tink_core::keyset::validate_key_version(key.version, HPKE_PUBLIC_KEY_KEY_VERSION)?;
    let params = key
        .params
        .as_ref()
        .ok_or_else(|| TinkError::new("no params"))?;
    validate_key_params(params)
}

fn validate_private_key(key: &tink_proto::HpkePrivateKey) -> Result<HpkeSuite, TinkError> {
    tink_core::keyset::validate_key_version(key.version, HPKE_PRIVATE_KEY_KEY_VERSION)?;
    let pub_key = key
        .public_key
        .as_ref()
        .ok_or_else(|| TinkError::new("no public key"))?;
    tink_core::keyset::validate_key_version(pub_key.version, HPKE_PUBLIC_KEY_KEY_VERSION)?;
    let params = pub_key
        .params
        .as_ref()
        .ok_or_else(|| TinkError::new("no params"))?;
    validate_key_params(params)
}

/// Validate HPKE parameters and return the resolved [`HpkeSuite`].
///
/// Adding a new supported suite requires adding a variant to [`HpkeSuite`] and
/// handling it in both `seal` and `open`, so the compiler enforces completeness.
fn validate_key_params(params: &tink_proto::HpkeParams) -> Result<HpkeSuite, TinkError> {
    let kem = match HpkeKem::try_from(params.kem) {
        Ok(HpkeKem::DhkemX25519HkdfSha256) => HpkeKem::DhkemX25519HkdfSha256,
        Ok(HpkeKem::KemUnknown) => return Err(TinkError::new("unknown KEM")),
        Err(_) => return Err(TinkError::new("unrecognized KEM value")),
    };

    let kdf = match HpkeKdf::try_from(params.kdf) {
        Ok(HpkeKdf::HkdfSha256) => HpkeKdf::HkdfSha256,
        Ok(HpkeKdf::KdfUnknown) => return Err(TinkError::new("unknown KDF")),
        Err(_) => return Err(TinkError::new("unrecognized KDF value")),
    };

    let aead = match HpkeAead::try_from(params.aead) {
        Ok(HpkeAead::Aes256Gcm) => HpkeAead::Aes256Gcm,
        Ok(HpkeAead::Aes128Gcm) => HpkeAead::Aes128Gcm,
        Ok(HpkeAead::Chacha20Poly1305) => HpkeAead::Chacha20Poly1305,
        Ok(HpkeAead::AeadUnknown) => return Err(TinkError::new("unknown AEAD")),
        Err(_) => return Err(TinkError::new("unrecognized AEAD value")),
    };

    match (kem, kdf, aead) {
        (HpkeKem::DhkemX25519HkdfSha256, HpkeKdf::HkdfSha256, HpkeAead::Aes256Gcm) => {
            Ok(HpkeSuite::X25519Sha256Aes256Gcm)
        }
        (HpkeKem::DhkemX25519HkdfSha256, HpkeKdf::HkdfSha256, HpkeAead::Aes128Gcm) => {
            Ok(HpkeSuite::X25519Sha256Aes128Gcm)
        }
        (HpkeKem::DhkemX25519HkdfSha256, HpkeKdf::HkdfSha256, HpkeAead::Chacha20Poly1305) => {
            Ok(HpkeSuite::X25519Sha256Chacha20Poly1305)
        }
        _ => Err(TinkError::new("unsupported HPKE suite combination")),
    }
}
