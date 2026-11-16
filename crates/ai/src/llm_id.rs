use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LLMId(String);

impl LLMId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for LLMId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for LLMId {
    fn from(value: &str) -> Self {
        value.to_owned().into()
    }
}

impl From<LLMId> for String {
    fn from(value: LLMId) -> Self {
        value.0
    }
}

impl std::fmt::Display for LLMId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
