use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The type of input the user has provided.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputType {
    /// The user input is a shell command.
    #[default]
    Shell,
    /// The user input is a natural language query to AI.
    AI,
}

impl InputType {
    pub fn is_ai(&self) -> bool {
        matches!(self, InputType::AI)
    }
}

impl FromStr for InputType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "shell" => Ok(InputType::Shell),
            "ai" => Ok(InputType::AI),
            _ => Err(format!("Invalid input type: {s}. Must be 'shell' or 'ai'")),
        }
    }
}

impl std::fmt::Display for InputType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InputType::Shell => write!(f, "Shell"),
            InputType::AI => write!(f, "AI"),
        }
    }
}
