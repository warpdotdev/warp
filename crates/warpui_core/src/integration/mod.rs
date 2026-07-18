use std::collections::HashSet;
use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::{env, fs};

mod action_log;
mod artifacts;
pub mod capture_recorder;
mod driver;
pub mod overlay;
mod step;
pub mod video_recorder;
pub use action_log::ActionLog;
pub use artifacts::ARTIFACTS_DIR_ENV_VAR;
pub use driver::{Builder, SetupFn, TestDriver, RERUN_EXIT_CODE, RUNTIME_TAG_FAILURE_REASON};
pub use overlay::OverlayLog;
pub use step::{
    AssertionCallback, AssertionOutcome, AssertionWithDataCallback, IntegrationTestEvent,
    PersistedDataMap, StepData, StepDataMap, TestStep,
};
pub use video_recorder::{save_captured_frame_as_png, VideoRecorder};
use warp_errors::report_error;

#[macro_export]
macro_rules! async_assert {
    ($left:expr) => {
        match (&$left) {
            (left_val) => {
                if *left_val {
                    $crate::integration::AssertionOutcome::Success
                } else {
                    let assertion_message = format!("assertion failed: {}", stringify!($left));
                    $crate::integration::AssertionOutcome::failure(assertion_message)
                }
            }
        }
    };
    ($left:expr, $($arg:tt)+) => {
        match (&$left) {
            (left_val) => {
                if *left_val {
                    $crate::integration::AssertionOutcome::Success
                } else {
                    let assertion_message = format!("assertion failed: {}", format_args!($($arg)+));
                    $crate::integration::AssertionOutcome::failure(assertion_message)
                }
            }
        }
    };
}

/// Asserts that the condition is true immediately,
/// but allows for some on_finish behavior in the app before it panics.
#[macro_export]
macro_rules! integration_assert {
    ($left:expr) => {
        match (&$left) {
            (left_val) => {
                if !*left_val {
                    let assertion_message = format!("assertion failed: {}", stringify!($left));
                    return $crate::integration::AssertionOutcome::immediate_failure(assertion_message);
                }
            }
        }
    };
    ($left:expr, $($arg:tt)+) => {
        match (&$left) {
            (left_val) => {
                if !*left_val {
                    let assertion_message = format!("assertion failed: {}", format_args!($($arg)+));
                    return $crate::integration::AssertionOutcome::immediate_failure(assertion_message);
                }
            }
        }
    };
}

#[macro_export]
macro_rules! async_assert_eq {
    ($left:expr, $right:expr) => {
        match (&$left, &$right) {
            (left_val, right_val) => {
                if *left_val == *right_val {
                $crate::integration::AssertionOutcome::Success
            } else {
                let assertion_message = format!(
                    "assertion failed: `(left = right)`
  left: `{:?}`,
 right: `{:?}`",
                    left_val, right_val
                );
                $crate::integration::AssertionOutcome::failure(assertion_message)
            }
            }
        }
    };
    ($left:expr, $right:expr, $($arg:tt)+) => {
        match (&$left, &$right) {
            (left_val, right_val) => {
                if *left_val == *right_val {
                    $crate::integration::AssertionOutcome::Success
                } else {
                    let assertion_message = format!(
                    "assertion failed: `{}`
  left: `{:?}`,
 right: `{:?}`",
                    format_args!($($arg)+), left_val, right_val
                );
                    $crate::integration::AssertionOutcome::failure(assertion_message)
                }
            }
        }
    };
}

pub struct TestSetupUtils {
    env_vars: HashSet<String>,
    root_dir: RootDir,
}

impl TestSetupUtils {
    fn new(root_dir: RootDir) -> Self {
        TestSetupUtils {
            env_vars: HashSet::new(),
            root_dir,
        }
    }

    /// Returns the $HOME dir for the test.
    pub fn test_dir(&self) -> PathBuf {
        self.root_dir.as_path().to_path_buf()
    }

    pub fn set_env<K, V>(&mut self, key: K, value: Option<V>)
    where
        K: Into<String>,
        V: AsRef<OsStr>,
    {
        let key = key.into();
        match value {
            Some(v) => {
                println!(
                    "Setting env var {} to {} for test",
                    key,
                    v.as_ref().to_string_lossy()
                );
                env::set_var(&key, v);
                self.env_vars.insert(key);
            }
            None => {
                println!("Clearing env var {key}");
                env::remove_var(key);
            }
        };
    }

    pub fn cleanup_env(&mut self) {
        for key in &self.env_vars {
            println!("Clearing env var {key}");
            env::remove_var(key);
        }
        self.env_vars = HashSet::new();
    }

    // For each test, we create an empty directory in the temp filesystem. This will be the root
    // for that test's specific resources.
    fn create_temp_dir_for_test(&self) {
        let test_dir = self.root_dir.as_path();

        // Remove anything we failed to remove from previous runs of the test.
        match fs::remove_dir_all(test_dir) {
            Ok(_) => (),
            Err(err) => {
                // Not found is fine because there's no old data to interfere with this test.
                if err.kind() != ErrorKind::NotFound {
                    eprintln!("failure cleaning up test temp dir at {test_dir:?}");
                    if let Ok(rd) = test_dir.read_dir() {
                        eprintln!("contents of test temp dir:");
                        for entry in rd.flatten() {
                            eprintln!("  - {:?}", entry.file_name());
                        }
                    }
                    panic!("failed to remove previous run test data: {err}");
                }
            }
        }

        let res = fs::create_dir_all(test_dir);
        if let Err(err_code) = res {
            if err_code.kind() != ErrorKind::AlreadyExists {
                panic!("Failed to create directory {test_dir:?}");
            }
        }
    }

    /// Configures the home directory path for the test.
    fn set_home_dir_for_test(&mut self) {
        if cfg!(unix) {
            self.set_env("ORIGINAL_HOME", dirs::home_dir());
            // Override the home directory path.  This helps keep tests more
            // hermetic by making them not depend on the contents of the user's
            // home directory (which could be very different on a developer's
            // machine vs. on cloud CI runners).
            //
            // We canonicalize the path to resolve symlinks (e.g. /var ->
            // /private/var on macOS) so that the shell's resolved $PWD matches
            // $HOME exactly, which is required for ~ substitution to work.
            let canonical_test_dir = self
                .test_dir()
                .canonicalize()
                .unwrap_or_else(|_| self.test_dir());
            self.set_env("HOME", Some(canonical_test_dir));
        }
    }

    pub fn cleanup_dir(&mut self) {
        if let Err(err) = fs::remove_dir_all(self.root_dir.as_path()) {
            report_error!("Could not cleanup directory", extra: { "error" => ?err });
        }
    }
}

enum RootDir {
    /// Uses the provided path as the test's root directory.
    Path(PathBuf),
    /// Uses the provided TempDir as the test's root directory.
    TempDir(tempfile::TempDir),
}

impl RootDir {
    fn as_path(&self) -> &Path {
        match self {
            RootDir::Path(path) => path.as_path(),
            RootDir::TempDir(tempdir) => tempdir.path(),
        }
    }
}
