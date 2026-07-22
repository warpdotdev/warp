use std::io;

use base64::Engine;
use tink_proto::KeysetInfo;
use tink_proto::keyset_info::KeyInfo;

use super::UploadKey;
use crate::ManagedSecretValue;

#[test]
fn test_import_public_keyset() {
    super::init();

    // A base64-encoded public keyset as returned by warp-server.
    let public_key = "COvInMIBEnAKZAo0dHlwZS5nb29nbGVhcGlzLmNvbS9nb29nbGUuY3J5cHRvLnRpbmsuSHBrZVB1YmxpY0tleRIqEgYIARABGAIaIHRaibhtYbpEfh2CSpdDPhh/6lCBnfoO3nqBmZ3VQGJyGAMQARjryJzCASAB";
    let upload_key =
        UploadKey::import_public_keyset(public_key).expect("unable to import public keyset");

    let keyset_info = upload_key.public_key.keyset_info();
    assert_eq!(
        keyset_info,
        KeysetInfo {
            primary_key_id: 407315563,
            key_info: vec![KeyInfo {
                key_id: 407315563,
                status: tink_proto::KeyStatusType::Enabled.into(),
                type_url: "type.googleapis.com/google.crypto.tink.HpkePublicKey".to_string(),
                output_prefix_type: tink_proto::OutputPrefixType::Tink.into(),
            }],
        }
    );

    let encrypted = upload_key
        .encrypt
        .encrypt(b"hello from rust", b"rust context")
        .expect("unable to encrypt");
    assert!(!encrypted.is_empty());
}

/// An HPKE private key for use in tests.
///
/// Created with:
/// ```sh
/// $ java -jar /opt/homebrew/Cellar/tinkey/1.12.0/bin/tinkey_deploy.jar create-keyset --key-template DHKEM_X25519_HKDF_SHA256_HKDF_SHA256_AES_256_GCM --out-format json | jq .
/// ```
const TEST_PRIVATE_KEY: &str = r#"
{
  "primaryKeyId": 625520774,
  "key": [
    {
      "keyData": {
        "typeUrl": "type.googleapis.com/google.crypto.tink.HpkePrivateKey",
        "value": "EioSBggBEAEYAhogGHVh0Tju/DHOWEgpuUJ+9P/pXa5tK16udRWoJJwHbnIaIHdq5FthS7H4Q6xSLzCEnbf1z/F1+PTQHev/5PJ+pc+m",
        "keyMaterialType": "ASYMMETRIC_PRIVATE"
      },
      "status": "ENABLED",
      "keyId": 625520774,
      "outputPrefixType": "TINK"
    }
  ]
}
"#;

/// An HPKE public key for use in tests, corresponding to [`TEST_PRIVATE_KEY`].
///
/// Created with:
/// ```sh
/// $ java -jar /opt/homebrew/Cellar/tinkey/1.12.0/bin/tinkey_deploy.jar create-public-keyset | jq .
/// < private key JSON on stdin >
/// ```
const TEST_PUBLIC_KEY: &str = r#"
{
  "primaryKeyId": 625520774,
  "key": [
    {
      "keyData": {
        "typeUrl": "type.googleapis.com/google.crypto.tink.HpkePublicKey",
        "value": "EgYIARABGAIaIBh1YdE47vwxzlhIKblCfvT/6V2ubSternUVqCScB25y",
        "keyMaterialType": "ASYMMETRIC_PUBLIC"
      },
      "status": "ENABLED",
      "keyId": 625520774,
      "outputPrefixType": "TINK"
    }
  ]
}
"#;

/// Test encrypting a managed secret value.
#[test]
fn test_encrypt_managed_secret() {
    super::init();

    let keyset = read_keyset_json(TEST_PUBLIC_KEY);
    let upload_key = UploadKey {
        encrypt: tink_hybrid::new_encrypt(&keyset).expect("failed to create encrypt primitive"),
        public_key: keyset,
    };

    let encrypted = upload_key
        .encrypt_secret(
            "user123",
            "MY_SECRET",
            &ManagedSecretValue::RawValue {
                value: "secret".to_string(),
            },
        )
        .expect("failed to encrypt secret");
    assert!(!encrypted.is_empty());
}

/// Test our HPKE encryption and decryption primitives. At the very least, they should be able to roundtrip a plaintext value.
#[test]
fn test_encrypt_decrypt() {
    super::init();

    let private_key = read_keyset_json(TEST_PRIVATE_KEY);
    let public_key = read_keyset_json(TEST_PUBLIC_KEY);

    let encrypt =
        tink_hybrid::new_encrypt(&public_key).expect("failed to create encrypt primitive");
    let decrypt =
        tink_hybrid::new_decrypt(&private_key).expect("failed to create decrypt primitive");

    let context = b"I am context";
    let plaintext = b"hello from rust";

    let encrypted = encrypt
        .encrypt(plaintext, context)
        .expect("failed to encrypt");
    let decrypted = decrypt
        .decrypt(&encrypted, context)
        .expect("failed to decrypt");

    assert_eq!(decrypted, plaintext);
}

fn read_keyset_json(json: &str) -> tink_core::keyset::Handle {
    let mut reader = tink_core::keyset::JsonReader::new(io::Cursor::new(json.as_bytes()));
    tink_core::keyset::insecure::read(&mut reader).expect("failed to read keyset")
}

/// BYO credential sealing round-trip via [`UploadKey::seal_with_context`].
///
/// Pins the exact context format shared by the WASM producer
/// (`managed_secrets_wasm::encrypt_byo_first_party`) and the Go consumer
/// (`byo.UnsealCredential`): `byo:1:TEAM:<owner_uid>:first_party:provider=openai`.
/// Proves the new seam reuses the same Tink/HPKE primitive (the sealed blob
/// round-trips) and that a mismatched context fails to unseal.
#[test]
fn test_seal_with_context_byo_roundtrip() {
    super::init();

    let public_key = read_keyset_json(TEST_PUBLIC_KEY);
    let private_key = read_keyset_json(TEST_PRIVATE_KEY);

    let upload_key = UploadKey {
        encrypt: tink_hybrid::new_encrypt(&public_key).expect("failed to create encrypt primitive"),
        public_key,
    };

    let owner_uid = "t_abc";
    let context = format!("byo:1:TEAM:{owner_uid}:first_party:provider=openai");
    assert_eq!(context, "byo:1:TEAM:t_abc:first_party:provider=openai");

    let plaintext = br#"{"api_key":"sk-test"}"#;
    let sealed = upload_key
        .seal_with_context(context.as_bytes(), plaintext)
        .expect("failed to seal BYO credential");

    let ciphertext = base64::prelude::BASE64_STANDARD
        .decode(&sealed)
        .expect("sealed blob is not valid base64");

    let decrypt =
        tink_hybrid::new_decrypt(&private_key).expect("failed to create decrypt primitive");
    let decrypted = decrypt
        .decrypt(&ciphertext, context.as_bytes())
        .expect("failed to unseal BYO credential");
    assert_eq!(decrypted, plaintext);

    // A mismatched context (e.g. wrong provider) must fail to unseal.
    let wrong_context = format!("byo:1:TEAM:{owner_uid}:first_party:provider=anthropic");
    assert!(
        decrypt
            .decrypt(&ciphertext, wrong_context.as_bytes())
            .is_err(),
        "unseal must fail when the context does not match"
    );
}
