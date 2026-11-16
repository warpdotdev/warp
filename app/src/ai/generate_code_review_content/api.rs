use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputType {
    CommitMessage,
    PrTitle,
    PrDescription,
}

#[derive(Serialize, Deserialize)]
pub struct GenerateCodeReviewContentRequest {
    pub output_type: OutputType,
    pub diff: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub branch_name: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub commit_messages: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct GenerateCodeReviewContentResponse {
    pub content: String,
}
