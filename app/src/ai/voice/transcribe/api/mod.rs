//! This module contains Rust types for the Transcribe endpoint in warp-server for voice transcription.
//!
//! These types are manually transposed from the API schema defined in go
//! (warp-server/model/types/transcribe/(request.go|response.go|common.go)).
//!
//! Documentation on the types here is directly borrowed from the documentation on the go schema;
//! see the go schema for the source-of-truth.
mod request;
mod response;

pub use request::*;
pub use response::*;
