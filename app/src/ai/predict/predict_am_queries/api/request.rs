use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct PredictAMQueriesRequest {
    pub context_messages: Vec<String>,
    pub partial_query: String,
    pub system_context: Option<String>,
}
