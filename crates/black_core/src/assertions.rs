/// Asserts that a condition is true, logging an error if it is not.
///
/// This macro is similar to the standard `debug_assert!` macro, but it logs
/// an error if the condition is not met.  This should generally be preferred
/// over `debug_assert!`, as it will log in production, though should not be
/// used in codepaths where the error log could be produced with high volume.
#[macro_export]
macro_rules! safe_assert {
    ($cond:expr $(,)?) => {{
        debug_assert!($cond);
        match &$cond {
            (cond) => {
                if !(*cond) {
                    log::error!("Assertion `{}` failed", stringify!($cond));
                }
            }
        }
    }};
    ($cond:expr, $($arg:tt)+) => {{
        debug_assert!($cond, $($arg)+);
        match &$cond {
            (cond) => {
                if !(*cond) {
                    log::error!("Assertion `{}` failed: {}", stringify!($cond), format_args!($($arg)+));
                }
            }
        }
    }};
}
pub use safe_assert;

/// Asserts that two expressions are equal, logging an error if they are not.
///
/// This macro is similar to the standard `debug_assert_eq!` macro, but it logs
/// an error if the values are not equal. This should generally be preferred
/// over `debug_assert_eq!`, as it will log in production, though should not be
/// used in codepaths where the error log could be produced with high volume.
#[macro_export]
macro_rules! safe_assert_eq {
    ($left:expr, $right:expr $(,)?) => {{
        debug_assert_eq!($left, $right);
        match (&$left, &$right) {
            (left_val, right_val) => {
                if !(*left_val == *right_val) {
                    log::error!("Assertion `{} == {}` failed: expected {left_val}, found {right_val}.", stringify!($left), stringify!($right));
                }
            }
        }
    }};
    ($left:expr, $right:expr, $($arg:tt)+) => {{
        debug_assert_eq!($left, $right, $($arg)+);
        match (&$left, &$right) {
            (left_val, right_val) => {
                if !(*left_val == *right_val) {
                    log::error!("Assertion `{} == {}` failed: expected {left_val}, found {right_val}. {}", stringify!($left), stringify!($right), format_args!($($arg)+));
                }
            }
        }
    }};
}
pub use safe_assert_eq;
