use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct GenerateBlockTitleRequest {
    pub command: String,
    pub output: String,
}

#[derive(Serialize, Deserialize)]
pub struct GenerateBlockTitleResponse {
    pub title: String,
}
