//! Test helpers for capturing URL-open requests during integration tests.
//!
//! In production, `AppContext::open_url` calls a `before_open_url`
//! callback (used to rewrite Warp web URLs to local intents) and then
//! delegates to the platform. The integration test platform delegate's
//! `open_url` is a no-op, so installing a capture callback gives us a
//! reliable way to assert that a click actually triggered a URL open
//! without launching a real browser.

mod assertion;
mod step;

pub use assertion::*;
pub use step::*;
