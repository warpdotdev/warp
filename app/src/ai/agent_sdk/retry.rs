//! Shared retry primitives for HTTP-backed operations in the agent SDK.
//!
//! The canonical implementation of `with_bounded_retry` and `is_transient_http_error`
//! lives in [`crate::server::retry_strategies`] (available on all targets, including WASM).
//! This module re-exports those symbols so existing agent-SDK call sites keep compiling
//! without a path change.

// Re-export for tests only; the canonical definitions live in retry_strategies.
#[cfg(test)]
pub(crate) use crate::server::retry_strategies::{MAX_ATTEMPTS, is_transient_http_error};
pub(crate) use crate::server::retry_strategies::{
    is_transient_graphql_or_http_error, with_bounded_retry, with_bounded_retry_using,
};

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;
