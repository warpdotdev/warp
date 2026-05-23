//! This module contains all code relevant to Agent Predict within Black.
//!
//! Agent Predict attempts to predict the next action the user will take in Black.

pub(crate) mod generate_ai_input_suggestions;
pub(crate) mod generate_am_query_suggestions;
pub mod next_command_model;
pub(crate) mod predict_am_queries;
pub mod prompt_suggestions;
