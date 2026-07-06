use serde::Serialize;
use warp_managed_secrets::{ManagedSecretValue, UploadKey, init_envelope};
use wasm_bindgen::prelude::*;

/// Called once when the WASM module is instantiated.
#[wasm_bindgen(start)]
pub fn start() {
    init_envelope();
}

/// Helper: import keyset and encrypt a secret value.
fn do_encrypt(
    public_key_base64: &str,
    actor_uid: &str,
    secret_name: &str,
    secret: &ManagedSecretValue,
) -> Result<String, JsValue> {
    let upload_key = UploadKey::import_public_keyset(public_key_base64)
        .map_err(|e| JsValue::from_str(&format!("failed to import public key: {e}")))?;

    upload_key
        .encrypt_secret(actor_uid, secret_name, secret)
        .map_err(|e| JsValue::from_str(&format!("encryption failed: {e}")))
}

/// Encrypt a raw secret value.
#[wasm_bindgen]
pub fn encrypt_raw_secret(
    public_key_base64: &str,
    actor_uid: &str,
    secret_name: &str,
    secret_value: &str,
) -> Result<String, JsValue> {
    do_encrypt(
        public_key_base64,
        actor_uid,
        secret_name,
        &ManagedSecretValue::raw_value(secret_value),
    )
}

/// Encrypt an Anthropic API key secret.
#[wasm_bindgen]
pub fn encrypt_anthropic_api_key_secret(
    public_key_base64: &str,
    actor_uid: &str,
    secret_name: &str,
    api_key: &str,
) -> Result<String, JsValue> {
    do_encrypt(
        public_key_base64,
        actor_uid,
        secret_name,
        &ManagedSecretValue::anthropic_api_key(api_key),
    )
}

/// Encrypt an Anthropic Bedrock API key secret.
#[wasm_bindgen]
pub fn encrypt_anthropic_bedrock_api_key_secret(
    public_key_base64: &str,
    actor_uid: &str,
    secret_name: &str,
    aws_bearer_token_bedrock: &str,
    aws_region: &str,
) -> Result<String, JsValue> {
    do_encrypt(
        public_key_base64,
        actor_uid,
        secret_name,
        &ManagedSecretValue::anthropic_bedrock_api_key(aws_bearer_token_bedrock, aws_region),
    )
}

/// Encrypt an Anthropic Bedrock access key secret.
///
/// `aws_session_token` is optional and may be `None` for persistent IAM credentials
/// that do not require a session token.
#[wasm_bindgen]
pub fn encrypt_anthropic_bedrock_access_key_secret(
    public_key_base64: &str,
    actor_uid: &str,
    secret_name: &str,
    aws_access_key_id: &str,
    aws_secret_access_key: &str,
    aws_session_token: Option<String>,
    aws_region: &str,
) -> Result<String, JsValue> {
    do_encrypt(
        public_key_base64,
        actor_uid,
        secret_name,
        &ManagedSecretValue::anthropic_bedrock_access_key(
            aws_access_key_id,
            aws_secret_access_key,
            aws_session_token,
            aws_region,
        ),
    )
}

/// Encrypt an OpenAI API key secret.
///
/// `base_url` is optional; when `None`, the harness uses the provider's default endpoint.
#[wasm_bindgen]
pub fn encrypt_openai_api_key_secret(
    public_key_base64: &str,
    actor_uid: &str,
    secret_name: &str,
    api_key: &str,
    base_url: Option<String>,
) -> Result<String, JsValue> {
    do_encrypt(
        public_key_base64,
        actor_uid,
        secret_name,
        &ManagedSecretValue::openai_api_key(api_key, base_url),
    )
}

// ── BYO (bring-your-own) credential sealing ─────────────────────────────────
//
// These exports seal enterprise BYOK / BYOE credentials for team upload. They
// do NOT reuse the typed managed-secret encryptors above: the managed-secret
// AEAD context is bound to the secret type (`1:<actor>:<name>:<type>`), whereas
// BYO credentials are bound to `byo:1:TEAM:<owner_uid>:<cred_type>:<slot>` and
// must round-trip with the server's `byo.UnsealCredential`. The context string
// and JSON payload are constructed here (single source of truth) and sealed via
// the crate's existing Tink/HPKE hybrid primitive for exact wire-format parity.

/// Plaintext payload for a first-party (BYOK) credential. Serialized as snake_case
/// JSON (`{"api_key":"…"}`) to match the server unseal contract.
#[derive(Serialize)]
struct ByoFirstPartyPayload<'a> {
    api_key: &'a str,
}

/// Plaintext payload for a custom-endpoint (BYOE) credential. Serialized as
/// snake_case JSON (`{"base_url":"…","api_key":"…"}`).
#[derive(Serialize)]
struct ByoEndpointPayload<'a> {
    base_url: &'a str,
    api_key: &'a str,
}

/// Build the pinned BYO first-party seal context. Must byte-for-byte match the
/// server `byo.UnsealCredential` contract:
/// `byo:1:TEAM:<owner_uid>:first_party:provider=<provider>`.
fn byo_first_party_context(owner_uid: &str, provider: &str) -> String {
    format!("byo:1:TEAM:{owner_uid}:first_party:provider={provider}")
}

/// Build the pinned BYO endpoint seal context:
/// `byo:1:TEAM:<owner_uid>:endpoint:endpoint=<endpoint_id>`.
fn byo_endpoint_context(owner_uid: &str, endpoint_id: &str) -> String {
    format!("byo:1:TEAM:{owner_uid}:endpoint:endpoint={endpoint_id}")
}

/// Import the public keyset and HPKE-seal `payload` (as JSON) under `context`.
fn seal_byo(
    public_key_base64: &str,
    context: &str,
    payload: &impl Serialize,
) -> Result<String, JsValue> {
    let upload_key = UploadKey::import_public_keyset(public_key_base64)
        .map_err(|e| JsValue::from_str(&format!("failed to import public key: {e}")))?;
    let plaintext = serde_json::to_vec(payload)
        .map_err(|e| JsValue::from_str(&format!("failed to serialize payload: {e}")))?;
    upload_key
        .seal_with_context(context.as_bytes(), &plaintext)
        .map_err(|e| JsValue::from_str(&format!("encryption failed: {e}")))
}

/// Encrypt (HPKE-seal) a team BYO first-party API key.
///
/// Seals `{"api_key":"<api_key>"}` bound to the context
/// `byo:1:TEAM:<owner_uid>:first_party:provider=<provider>`. `owner_uid` is the
/// team's string UID; `provider` is one of `openai` / `anthropic` / `google`.
#[wasm_bindgen]
pub fn encrypt_byo_first_party(
    public_key_base64: &str,
    owner_uid: &str,
    provider: &str,
    api_key: &str,
) -> Result<String, JsValue> {
    let context = byo_first_party_context(owner_uid, provider);
    seal_byo(
        public_key_base64,
        &context,
        &ByoFirstPartyPayload { api_key },
    )
}

/// Encrypt (HPKE-seal) a team BYO custom-endpoint credential.
///
/// Seals `{"base_url":"<base_url>","api_key":"<api_key>"}` bound to the context
/// `byo:1:TEAM:<owner_uid>:endpoint:endpoint=<endpoint_id>`. `endpoint_id` is the
/// client-minted endpoint UUID.
#[wasm_bindgen]
pub fn encrypt_byo_endpoint(
    public_key_base64: &str,
    owner_uid: &str,
    endpoint_id: &str,
    base_url: &str,
    api_key: &str,
) -> Result<String, JsValue> {
    let context = byo_endpoint_context(owner_uid, endpoint_id);
    seal_byo(
        public_key_base64,
        &context,
        &ByoEndpointPayload { base_url, api_key },
    )
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
