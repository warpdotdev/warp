use command::blocking::Command;
use std::env;
use std::process::Stdio;
use warpui::integration::RERUN_EXIT_CODE;

const MAX_TEST_RUNS: usize = 10;

/// Runs a single integration test.
///
/// This runs the `integration` binary from the `warp` crate, passing it the
/// name of the test to execute as the one positional argument.
pub fn run_integration_test(name: &str) -> Result<(), String> {
    let mut keep_going = true;
    let mut run_num = 0;
    while keep_going {
        let inherited_envs = env::vars_os().filter(|(k, _v)| {
            let k = k
                .to_str()
                .expect("environment variable keys should contain valid unicode");
            // Propagate the PATH to the integration test
            // process, otherwise the shell it spawns might not
            // be able to find the binaries it needs to execute.
            k == "PATH"
                // Propagate any Rust-related variables.
                || k.starts_with("RUST_")
                // Propagate any Warp-specific variables.
                || k.starts_with("WARP_")
                || k.starts_with("WARPUI_")
                // Propagate any wgpu-specific variables.
                || k.starts_with("WGPU_")
                // Make sure the test knows what X or Wayland server to use.
                || k == "DISPLAY"
                || k == "WAYLAND_DISPLAY"
                // Propagate XDG_RUNTIME_DIR, which is needed for tests to run.
                // We actively _do not_ want to propagate other XDG_ variables,
                // as they tend to encode the home directory, which we override
                // in tests to point to a per-test temporary directory.
                || k == "XDG_RUNTIME_DIR"
                // Propagate XAUTHORITY so we can run headless tests using xvfb.
                || k == "XAUTHORITY"
        });
        keep_going = match Command::new(env!("CARGO_BIN_EXE_integration"))
            .arg(name)
            .env_clear()
            .envs(inherited_envs)
            .env("WARP_INTEGRATION", "1")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
        {
            Ok(status) => match status.code() {
                Some(0) => {
                    println!("Test exited with success.");
                    false
                }
                Some(RERUN_EXIT_CODE) if run_num < MAX_TEST_RUNS => {
                    println!("Test exited with rerun code, trying again.");
                    run_num += 1;
                    true
                }
                Some(exit_code) => {
                    return std::result::Result::Err(format!(
                        "Test {name} failed with exit code {exit_code}",
                    ));
                }
                None => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::ExitStatusExt;
                        let signal = status
                            .signal()
                            .and_then(|signal| nix::sys::signal::Signal::try_from(signal).ok());
                        if let Some(signal) = signal {
                            return std::result::Result::Err(format!(
                                "Test {name} failed due to signal {}",
                                signal.as_str(),
                            ));
                        } else {
                            return std::result::Result::Err(format!(
                                "Test {name} failed for unknown reason",
                            ));
                        }
                    }
                    #[cfg(windows)]
                    {
                        return std::result::Result::Err(format!(
                            "Test {name} failed for unknown reason",
                        ));
                    }
                }
            },
            Err(err) => {
                return std::result::Result::Err(format!("Test {name} failed with error {err:#}"));
            }
        }
    }
    Ok(())
}

#[macro_export]
macro_rules! integration_tests {
    (   $(
            $(#[$args:meta])*
            $name:ident,
        )*
    ) => {
        $(
            $(#[$args])*
            // Ignore unused attributes, in case we're marking a test as
            // ignored twice, once via arguments passed to the macro and once
            // below.
            #[allow(unused_attributes)]
            // For right now, we only want to run integration tests on macOS
            // and Linux (iff the run_on_linux feature is enabled).
            #[cfg_attr(not(any(target_os = "macos", feature = "run_on_linux")), ignore)]
            #[test]
            fn $name() -> Result<(), String> {
                $crate::common::run_integration_test(stringify!($name))
            }
        )*
    }
}
