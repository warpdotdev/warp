//! Discover models available from an OpenAI-compatible custom inference
//! endpoint by calling its `/models` endpoint and parsing the response.
//!
//! Wired into the "Fetch from endpoint" button in
//! [`crate::settings_view::custom_inference_modal`] so a user does not have
//! to type out every model name when configuring a [`CustomEndpoint`].

use anyhow::{anyhow, Context};
use serde::Deserialize;

/// Maximum response body size we'll try to parse from `/models`. Reqwest has
/// already buffered the body by the time we check, so this only caps the work
/// the JSON parser does — not the memory the HTTP client allocates.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024; // 1 MiB

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
}

/// Hit `<base_url>/models` with bearer auth and return the list of model IDs.
///
/// `base_url` is the user-provided endpoint root (e.g. `https://openrouter.ai/api/v1`).
/// A trailing slash is tolerated. Empty model IDs are filtered out.
pub async fn discover_models(
    client: &http_client::Client,
    base_url: &str,
    api_key: &str,
) -> anyhow::Result<Vec<String>> {
    let base_trimmed = base_url.trim().trim_end_matches('/');
    if base_trimmed.is_empty() {
        return Err(anyhow!("Endpoint URL is empty"));
    }
    if api_key.trim().is_empty() {
        return Err(anyhow!("API key is empty"));
    }
    let url = format!("{base_trimmed}/models");

    let response = client
        .get(&url)
        .bearer_auth(api_key)
        .header("Accept", "application/json")
        .send()
        .await
        .with_context(|| format!("network error fetching {url}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        return Err(anyhow!(
            "endpoint returned HTTP {status}: {snippet}",
            status = status,
            snippet = snippet
        ));
    }

    let bytes = response.bytes().await.context("reading response body")?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(anyhow!(
            "response body too large: {} bytes (max {})",
            bytes.len(),
            MAX_RESPONSE_BYTES
        ));
    }

    let parsed: ModelsResponse =
        serde_json::from_slice(&bytes).context("response was not OpenAI-compatible JSON")?;

    let ids: Vec<String> = parsed
        .data
        .into_iter()
        .map(|entry| entry.id)
        .filter(|id| !id.trim().is_empty())
        .collect();

    if ids.is_empty() {
        return Err(anyhow!("endpoint returned no models"));
    }

    Ok(ids)
}

/// Merge `discovered` model IDs into `existing` rows. Returns the IDs that
/// were not already present (in the order they appeared in `discovered`).
///
/// "Already present" matches case-insensitively on the row name and skips
/// rows whose name field is blank, so a freshly-opened modal with one empty
/// row is treated as "no existing models" rather than blocking the merge.
pub fn new_model_ids<'a>(discovered: &'a [String], existing: &[String]) -> Vec<&'a str> {
    let existing_lower: std::collections::HashSet<String> = existing
        .iter()
        .map(|name| name.trim().to_lowercase())
        .filter(|name| !name.is_empty())
        .collect();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for id in discovered {
        let key = id.trim().to_lowercase();
        if key.is_empty() {
            continue;
        }
        if existing_lower.contains(&key) || !seen.insert(key) {
            continue;
        }
        out.push(id.as_str());
    }
    out
}

#[cfg(test)]
#[path = "discover_models_tests.rs"]
mod tests;
