use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetRelevantFiles {
    pub query: String,
    pub files: Vec<FileContext>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetRelevantFilesResponse {
    pub relevant_file_paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileContext {
    pub path: String,
    pub symbols: String,
}
