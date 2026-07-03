//! This crate provides an ipmlementation of a basic IPC request/response protocol.
//!
//! Users may instantiate a server that implements any number of [`Service`]s as well as
//! corresponding typed "clients" ([`ServiceCaller`]s) which provide a typed interface to call the
//! services across process boundaries.
//!
//! This is intended to initially be used to support communication between the Warp app and
//! third-party plugins running in a separate "plugin host" process, but is designed generically to
//! be extended to other use cases such as the terminal server. Where possible,
//! transport-specific details are abstracted out.
//!
//! On native platforms, this is implemented on top of the `interprocess` crate, which uses
//! Unix Domain Sockets on Unix platforms and named pipes on Windows as the underlying transport.
//!
//! Basic usage is like so:
//!
//! ```ignore
//! // In the server's process...
//! let background_executor = ctx.background_executor();
//!
//! // `MyServiceImpl` implements `ServiceImpl<Service = MyService>`.
//! let my_service_impl = MyServiceImpl::new();
//! let (server, connection_address) = ServerBuilder::default()
//!     .with_service(my_service_impl)
//!     .build_and_run(background_executor)
//!     .expect("Failed to instantiate server");
//!
//! // In the client process, passing the same connection address returned from the server
//! // instantiation (possibly as an environment variable set in the client process).
//! let client = Arc::new(
//!     Client::connect(connection_address, background_executor)
//!         .await
//!         .expect("Failed to connect client"),
//! );
//! let my_service_stub = service_caller::<MyService>(client);
//! let response = my_service_stub.call(MyServiceRequest { .. }).await;
//! ```
mod client;
mod protocol;
mod server;
mod service;

// Native transport implementation for both server and client.
#[path = "native.rs"]
mod platform;

pub use client::{Client, ClientError};
pub use protocol::ConnectionAddress;
pub use server::{Server, ServerBuilder};
pub use service::{service_caller, Service, ServiceCaller, ServiceImpl};

#[cfg(test)]
pub mod testing;
