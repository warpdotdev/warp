//! Interface for working with processes that ensures that commands are spawned with the correct set
//! of arguments.
//!
//! This is needed for Windows: Any attempt at spawning a new command without the `no_window` flag
//! will result in a new terminal temporarily flashing in front of the application.
//!
//! [`blocking::Command`] can be used as a drop-in replacement for [`std::process::Command`],
//! and [`r#async::Command`] can be used as a drop-in replacement for [`async_process::Command`].
#[cfg(not(target_family = "wasm"))]
pub mod r#async;
pub mod blocking;
#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows;
pub mod wsl;

pub use std::process::{ExitStatus, Output, Stdio};
