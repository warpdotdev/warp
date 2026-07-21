pub mod ai_agent_tasks;
pub mod blockgrid;
pub mod settings;
pub mod terminal;
mod virtual_fs;

pub use blockgrid::mock_blockgrid;
pub use terminal::add_window_with_terminal;
pub use virtual_fs::{Stub, VirtualFS};

macro_rules! assert_eventually {
    ($cond:expr_2021, $($arg:tt)+) => {
        $crate::test_util::assert_eventually!(20 => $cond, $($arg)+);
    };
    // Run the condition up to ticks times, yielding to the executor in between.  If it does
    // not become true, this panics with the provided format string + args.
    ($ticks:literal => $cond:expr_2021, $($arg:tt)+) => {{
        let mut pass = false;
        for _ in 0..$ticks {
            if $cond {
                pass = true;
                break;
            }
            warpui::r#async::Timer::after(std::time::Duration::from_millis(5)).await;
        }
        if !pass {
            panic!("{}", format_args!($($arg)+));
        }
    }};
}
pub(crate) use assert_eventually;
