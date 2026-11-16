use anyhow::Result;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::integration_testing::agent_mode::util::get_base_server_url;

#[derive(Debug, Serialize)]
pub struct LLMGenerateRequest {
    pub prompt: String,
    pub user_messages: Vec<String>,
    /// These are model IDs internal to warp-server.
    /// See warp-server/logic/ai/llm/llm.go
    pub model_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ExactTokenUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_reads: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_writes: Option<i32>,
    pub total_input: i32,
    pub output: i32,
}

#[derive(Debug, Deserialize)]
pub struct LLMGenerateResponse {
    pub content: String,
    pub token_usage: ExactTokenUsage,
}

pub fn generate_llm_response(
    client: &Client,
    request: LLMGenerateRequest,
) -> Result<LLMGenerateResponse> {
    let url = format!("{}/agent-mode-evals/llm_generate", get_base_server_url());

    let response = client.post(&url).json(&request).send()?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Failed to generate LLM response: {}",
            response.status()
        ));
    }

    let response = response.json::<LLMGenerateResponse>()?;
    Ok(response)
}
