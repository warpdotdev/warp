use std::collections::HashSet;
use std::time::Duration;

use http::StatusCode;
use http::header::ACCEPT;
use serde::Deserialize;

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DiscoverModelsError {
    #[error("empty url")]
    EmptyUrl,
    #[error("empty key")]
    EmptyKey,
    #[error("unauthorized")]
    Unauthorized,
    #[error("not found")]
    NotFound,
    #[error("network error")]
    Network,
    #[error("unexpected response")]
    UnexpectedResponse,
    #[error("no models")]
    NoModels,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredModel {
    pub id: String,
    pub alias: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
}

pub fn models_url(base_url: &str) -> Result<String, DiscoverModelsError> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err(DiscoverModelsError::EmptyUrl);
    }
    let trimmed = trimmed.strip_suffix('/').unwrap_or(trimmed);
    Ok(format!("{trimmed}/models"))
}

pub async fn discover_models(
    client: &http_client::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<DiscoveredModel>, DiscoverModelsError> {
    discover_models_with_timeout(client, base_url, api_key, FETCH_TIMEOUT).await
}

pub(crate) async fn discover_models_with_timeout(
    client: &http_client::Client,
    base_url: &str,
    api_key: &str,
    timeout: Duration,
) -> Result<Vec<DiscoveredModel>, DiscoverModelsError> {
    if api_key.trim().is_empty() {
        return Err(DiscoverModelsError::EmptyKey);
    }
    let url = models_url(base_url)?;
    let response = client
        .get(&url)
        .bearer_auth(api_key)
        .header(ACCEPT, "application/json")
        .timeout(timeout)
        .send()
        .await
        .map_err(|_| DiscoverModelsError::Network)?;

    let status = response.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(DiscoverModelsError::Unauthorized);
    }
    if status == StatusCode::NOT_FOUND {
        return Err(DiscoverModelsError::NotFound);
    }
    if !status.is_success() {
        return Err(DiscoverModelsError::UnexpectedResponse);
    }

    if response
        .headers()
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|len| len > MAX_RESPONSE_BYTES)
    {
        return Err(DiscoverModelsError::UnexpectedResponse);
    }

    let body = response
        .text()
        .await
        .map_err(|_| DiscoverModelsError::Network)?;
    if body.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(DiscoverModelsError::UnexpectedResponse);
    }

    let parsed: ModelsResponse =
        serde_json::from_str(&body).map_err(|_| DiscoverModelsError::UnexpectedResponse)?;
    let models = unique_models(parsed.data);
    if models.is_empty() {
        return Err(DiscoverModelsError::NoModels);
    }
    Ok(models)
}

fn catalog_alias(id: &str, name: Option<&str>, display_name: Option<&str>) -> Option<String> {
    for candidate in [display_name, name].into_iter().flatten() {
        let trimmed = candidate.trim();
        if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case(id) {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn unique_models(entries: impl IntoIterator<Item = ModelEntry>) -> Vec<DiscoveredModel> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for entry in entries {
        let id = entry.id.trim();
        if id.is_empty() {
            continue;
        }
        if !seen.insert(id.to_ascii_lowercase()) {
            continue;
        }
        out.push(DiscoveredModel {
            alias: catalog_alias(id, entry.name.as_deref(), entry.display_name.as_deref()),
            id: id.to_string(),
        });
    }
    out
}

pub fn new_models<'a>(
    discovered: &'a [DiscoveredModel],
    existing: &[String],
) -> Vec<&'a DiscoveredModel> {
    let existing: HashSet<String> = existing
        .iter()
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for model in discovered {
        let key = model.id.trim().to_ascii_lowercase();
        if key.is_empty() || existing.contains(&key) || !seen.insert(key) {
            continue;
        }
        out.push(model);
    }
    out
}

#[cfg(test)]
#[path = "discover_models_tests.rs"]
mod tests;
