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
