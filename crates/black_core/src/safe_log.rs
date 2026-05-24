/// Safe Logger for sensitive info messages
///
/// Includes two log messages, labeled `safe:` and `full:`, the safe one will be sent in any
/// release channel, while the full log will only be used for local development, to aid in
/// debugging
#[macro_export]
macro_rules! safe_info {
    (safe: ($($safe_arg:tt)+), full: ($($full_arg:tt)+)) => (
        if $crate::channel::ChannelState::channel().is_dogfood() {
            log::info!($($full_arg)+)
        } else {
            log::info!($($safe_arg)+)
        }
    )
}

/// Safe Logger for sensitive warning messages
///
/// Includes two log messages, labeled `safe:` and `full:`, the safe one will be sent in any
/// release channel, while the full log will only be used for local development, to aid in
/// debugging
#[macro_export]
macro_rules! safe_warn {
    (safe: ($($safe_arg:tt)+), full: ($($full_arg:tt)+)) => (
        if $crate::channel::ChannelState::channel().is_dogfood() {
            log::warn!($($full_arg)+)
        } else {
            log::warn!($($safe_arg)+)
        }
    )
}

/// Safe Logger for sensitive error messages
///
/// Includes two log messages, labeled `safe:` and `full:`, the safe one will be sent in any
/// release channel, while the full log will only be used for local development, to aid in
/// debugging
#[macro_export]
macro_rules! safe_error {
    (safe: ($($safe_arg:tt)+), full: ($($full_arg:tt)+)) => ({
        if $crate::channel::ChannelState::channel().is_dogfood() {
            log::error!($($full_arg)+)
        } else {
            log::error!($($safe_arg)+)
        }
    })
}

/// Safe Logger for sensitive debug messages. Debug messages are generally not
/// logged at all in release channels, but could be enabled if a user sets
/// the `RUST_LOG` environment variable.
///
/// Includes two log messages, labeled `safe:` and `full:`, the safe one will be sent in any
/// release channel, while the full log will only be used for local development, to aid in
/// debugging.
#[macro_export]
macro_rules! safe_debug {
    (safe: ($($safe_arg:tt)+), full: ($($full_arg:tt)+)) => (
        if $crate::channel::ChannelState::channel().is_dogfood() {
            log::debug!($($full_arg)+)
        } else {
            log::debug!($($safe_arg)+)
        }
    )
}

/// Safe `anyhow::Error` builder for sensitive error messages.
///
/// Includes two error messages, labeled `safe:` and `full:`, the safe one will be sent in any
/// release channel, while the full log will only be used for local development, to aid in
/// debugging.
#[macro_export]
macro_rules! safe_anyhow {
    (safe: ($($safe_arg:tt)+), full: ($($full_arg:tt)+)) => (
        if $crate::channel::ChannelState::channel().is_dogfood() {
            anyhow::anyhow!($($full_arg)+)
        } else {
            anyhow::anyhow!($($safe_arg)+)
        }
    )
}

/// Safe `eprint!` for sensitive error messages.
///
/// Includes two error messages, labeled `safe:` and `full:`, the safe one will be sent in any
/// release channel, while the full log will only be used for local development, to aid in
/// debugging.
/// The safe message will only be printed if it is not empty.
/// This macro is mostly useful for the SDK, where access to the debug log is limited.
#[macro_export]
macro_rules! safe_eprintln {
    (safe: ($($safe_arg:tt)*), full: ($($full_arg:tt)+)) => (
        if $crate::channel::ChannelState::channel().is_dogfood() {
            eprintln!($($full_arg)+)
        } else if !stringify!($($safe_arg)*).trim().is_empty() {
            eprintln!($($safe_arg)*)
        }
    )
}
