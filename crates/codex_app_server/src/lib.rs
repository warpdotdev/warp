#![cfg(not(target_family = "wasm"))]

mod client;
mod types;

pub use client::{Client, ClientOptions, deny_server_request};
pub use types::{
    Account, AccountStatus, AccountType, ApprovalPolicy, LoginChallenge, LoginMode, Notification,
    SandboxMode, ServerRequest, ServerRequestResponse, ThreadOptions, TurnEvent, TurnResult,
};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
