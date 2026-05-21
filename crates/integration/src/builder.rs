use crate::util::{set_zsh_histfile_location, write_rc_files_for_test, ShellRcType};
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;
use warpui::integration::{self, PersistedDataMap, TestStep};
use warpui::integration::{TestDriver, TestSetupUtils};
use warpui::{App, WindowId};
use warpui_extras::user_preferences::file_backed::FileBackedUserPreferences;
use warpui_extras::user_preferences::UserPreferences;

// We have logic in our build script to pass the path of the cargo target
// tmp directory to our app. This needs to be done as a build script because
// the relevant env var is only available at build time to ensure things like
// debuggers work correctly (https://github.com/rust-lang/cargo/pull/9375#issuecomment-824204383).
include!(concat!(env!("OUT_DIR"), "/cargo_target_tmpdir.rs"));

/// Set a test timeout of 2 minutes.
///
/// We currently configure nextest with a timeout of 60s, so 120s is a safe
/// hard timeout for the test itself.  nextest should kill tests that hit the
/// timeout, but we sometimes see test processes sticking around on the test
/// runner devices, and this should help ensure those get cleaned up.
const TEST_TIMEOUT: instant::Duration = instant::Duration::from_secs(2 * 60);

/// Custom wrapper around an [`integration::Builder`] that ensures we create and setup tests in a
/// consistent way.
pub struct Builder {
    inner: integration::Builder,
    setup: Option<integration::SetupFn>,
    user_prefs: HashMap<String, String>,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    pub fn new() -> Self {
        let tmp_fs = PathBuf::from(cargo_target_tmpdir::get());
        let mut builder = integration::Builder::new(tmp_fs).with_timeout(TEST_TIMEOUT);

        if std::env::var("WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS").is_ok() {
            builder = builder.with_real_display();
        }

        Self {
            inner: builder,
            setup: None,
            user_prefs: Default::default(),
        }
    }

    pub fn with_timeout(self, timeout: Duration) -> Self {
        Self {
            inner: self.inner.with_timeout(timeout),
            ..self
        }
    }

    pub fn set_should_run_test<P>(self, predicate: P) -> Self
    where
        P: FnMut() -> bool + 'static,
    {
        Self {
            inner: self.inner.set_should_run_test(predicate),
            ..self
        }
    }

    pub fn with_real_display(self) -> Self {
        Self {
            inner: self.inner.with_real_display(),
            ..self
        }
    }

    pub fn with_step(self, step: TestStep) -> Self {
        Self {
            inner: self.inner.with_step(step),
            ..self
        }
    }

    pub fn with_steps(self, steps: Vec<TestStep>) -> Self {
        Self {
            inner: self.inner.with_steps(steps),
            ..self
        }
    }

    /// Applies to every TestStep added after this call in the builder, unless
    /// TestStep already has Some step_group_name.
    pub fn with_step_group_name(self, step_group_name: &str) -> Self {
        Self {
            inner: self.inner.with_step_group_name(step_group_name),
            ..self
        }
    }

    pub fn with_setup<C>(self, callback: C) -> Self
    where
        C: FnMut(&mut TestSetupUtils) + 'static,
    {
        assert!(
            self.setup.is_none(),
            "Can only register a single callback using with_setup!"
        );
        Self {
            setup: Some(Box::new(callback)),
            ..self
        }
    }

    pub fn with_cleanup<C>(self, callback: C) -> Self
    where
        C: FnMut(&mut TestSetupUtils) + 'static,
    {
        Self {
            inner: self.inner.with_cleanup(callback),
            ..self
        }
    }

    pub fn with_on_finish<C>(self, callback: C) -> Self
    where
        C: FnMut(
                &mut App,
                WindowId,
                &mut PersistedDataMap,
            ) -> Pin<Box<dyn Future<Output = ()> + Send>>
            + 'static,
    {
        Self {
            inner: self.inner.with_on_finish(callback),
            ..self
        }
    }

    pub fn with_user_defaults(mut self, user_defaults: HashMap<String, String>) -> Self {
        self.user_prefs.extend(user_defaults);
        self
    }

    pub fn with_static_persisted_data(self, data: PersistedDataMap) -> Self {
        Self {
            inner: self.inner.with_static_persisted_data(data),
            ..self
        }
    }

    /// Configures the test to run with its root directory under the /tmp
    /// directory instead of under CARGO_TARGET_TMPDIR.
    pub fn use_tmp_filesystem_for_test_root_directory(self) -> Self {
        Self {
            inner: self.inner.use_tmp_filesystem_for_test_root_directory(),
            ..self
        }
    }

    pub fn build(self, test_name: &str, create_temp_dir_for_test: bool) -> TestDriver {
        let Self {
            inner,
            mut setup,
            user_prefs,
        } = self;

        let inner = inner.with_setup(move |utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                &dir,
                "",
                [ShellRcType::Bash, ShellRcType::Zsh, ShellRcType::Fish],
            );
            set_zsh_histfile_location(&dir);

            // Set the DISABLE_SAVE_ENV_VAR to make sure we don't write any keybinding changes to the
            // filesystem
            utils.set_env(warp::keyboard::DISABLE_SAVE_ENV_VAR, Some("true"));

            // On Ubuntu (and possibly other Linux distros), a message is
            // printed out during shell initialization telling the user how to
            // use `sudo`. This can interfere with tests that make assertions
            // about the block list, so suppress the message.
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            std::fs::File::create(dir.join(".sudo_as_admin_successful"))
                .expect("should not fail to create file in home directory");

            if let Some(ref mut callback) = setup {
                callback(utils);
            }
        });

        let driver = inner.build(test_name, create_temp_dir_for_test);

        // As part of initializing the test driver, $HOME gets set to a unique
        // temporary directory.  We can now construct a file containing any
        // initial user preferences that are needed for the test.
        let file_path = warp::settings::user_preferences_file_path();
        // Use println because logging may not have been initialized yet.
        println!("Initializing preferences file at {file_path:?}");
        let prefs = match FileBackedUserPreferences::new(file_path.clone()) {
            Ok(prefs) => prefs,
            Err(err) => {
                eprintln!(
                    "Contents of existing preferences file: {:?}",
                    std::fs::read_to_string(file_path)
                );
                panic!("should not fail to initialize file-backed preferences store: {err:#}");
            }
        };
        for (key, value) in user_prefs {
            prefs
                .write_value(&key, value)
                .expect("should not fail to write initial user preferences");
        }

        driver
    }
}
