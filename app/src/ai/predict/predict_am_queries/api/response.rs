use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct PredictAMQueriesResponse {
    pub suggestion: String,
}
