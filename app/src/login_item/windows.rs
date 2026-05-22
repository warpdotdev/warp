//! Windows implementation of login-item registration via the HKCU
//! `Software\Microsoft\Windows\CurrentVersion\Run` registry key.
//!
//! This is the standard user-scope startup mechanism surfaced by
//! **Settings → Apps → Startup** and **Task Manager → Startup apps** on
//! Windows 10/11. It doesn't require admin elevation and is scoped to the
//! current user, matching the UX of macOS's `SMAppService`.

use std::path::{Path, PathBuf};

use ::settings::Setting;
use warp_core::channel::ChannelState;
use warpui::{AppContext, SingletonEntity};
use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
use winreg::RegKey;

use crate::report_if_error;
use crate::terminal::general_settings::GeneralSettings;

/// The registry subkey Windows scans on sign-in to launch per-user startup apps.
const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub(super) fn maybe_register_app_as_login_item(ctx: &mut AppContext) {
    GeneralSettings::handle(ctx).update(ctx, |settings, ctx| {
        let add_app_as_login_item = *settings.add_app_as_login_item;
        if add_app_as_login_item && *settings.app_added_as_login_item {
            // App is already registered as a login item. Don't re-register — the
            // user may have manually removed us via Settings → Apps → Startup or
            // Task Manager, and re-adding on every launch would override that.
            return;
        }

        let exe = match current_exe_path() {
            Some(p) => p,
            None => {
                log::warn!("Could not resolve current exe; skipping login-item registration");
                return;
            }
        };

        let value_name = login_item_value_name();

        // Registry I/O is fast but still sync and touches the disk — run it off
        // the UI thread to match the macOS path.
        ctx.spawn(
            async move {
                if add_app_as_login_item {
                    match register(&value_name, &exe) {
                        Ok(()) => true,
                        Err(err) => {
                            log::warn!("Failed to register Warp as a login item: {err}");
                            false
                        }
                    }
                } else {
                    match unregister(&value_name) {
                        Ok(()) => {}
                        Err(err) => {
                            // Don't flip app_added_as_login_item on failure — let a
                            // later retoggle try again.
                            log::warn!("Failed to unregister Warp as a login item: {err}");
                        }
                    }
                    false
                }
            },
            |settings, app_added_as_login_item, ctx| {
                report_if_error!(settings
                    .app_added_as_login_item
                    .set_value(app_added_as_login_item, ctx));
            },
        );
    });
}

fn current_exe_path() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| dunce::canonicalize(p).ok())
}

/// Returns the per-channel registry value name used under the `Run` subkey.
///
/// Using the channel's application name keeps Dogfood / Preview / Stable installs
/// isolated (`Warp`, `WarpPreview`, `WarpDev`, etc.) so installing multiple
/// channels doesn't cause one to overwrite another's startup entry.
fn login_item_value_name() -> String {
    ChannelState::app_id().application_name().to_owned()
}

/// Writes the startup registry value pointing at `exe` under `value_name`.
///
/// The path is wrapped in quotes so paths containing spaces (e.g.
/// `C:\Program Files\Warp\warp.exe`) are parsed as a single executable path.
fn register(value_name: &str, exe: &Path) -> std::io::Result<()> {
    register_in(HKEY_CURRENT_USER, RUN_SUBKEY, value_name, exe)
}

fn register_in(
    hive: winreg::HKEY,
    subkey: &str,
    value_name: &str,
    exe: &Path,
) -> std::io::Result<()> {
    let hkey = RegKey::predef(hive);
    let (run_key, _) = hkey.create_subkey(subkey)?;
    let quoted = format!("\"{}\"", exe.display());
    run_key.set_value(value_name, &quoted)
}

/// Removes the startup registry value under `value_name` if present. It is not
/// an error for the value to already be absent.
fn unregister(value_name: &str) -> std::io::Result<()> {
    unregister_in(HKEY_CURRENT_USER, RUN_SUBKEY, value_name)
}

fn unregister_in(hive: winreg::HKEY, subkey: &str, value_name: &str) -> std::io::Result<()> {
    let hkey = RegKey::predef(hive);
    let run_key = match hkey.open_subkey_with_flags(subkey, KEY_SET_VALUE) {
        Ok(k) => k,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    match run_key.delete_value(value_name) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};

    use super::*;

    /// A scratch subkey under HKCU that tests create/destroy to avoid touching
    /// the real `Software\Microsoft\Windows\CurrentVersion\Run` hive.
    struct ScratchSubkey {
        path: String,
    }

    impl ScratchSubkey {
        fn new(name: &str) -> Self {
            let suffix = format!(
                "{}_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                name,
            );
            let path = format!(r"Software\Warp\LoginItemTests\{suffix}");
            RegKey::predef(HKEY_CURRENT_USER)
                .create_subkey(&path)
                .expect("create scratch subkey");
            Self { path }
        }

        fn read(&self, value_name: &str) -> Option<String> {
            let key = RegKey::predef(HKEY_CURRENT_USER)
                .open_subkey_with_flags(&self.path, KEY_READ)
                .ok()?;
            key.get_value::<String, _>(value_name).ok()
        }
    }

    impl Drop for ScratchSubkey {
        fn drop(&mut self) {
            let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.path);
        }
    }

    #[test]
    fn register_writes_quoted_path() {
        let scratch = ScratchSubkey::new("register_writes_quoted_path");
        let exe = PathBuf::from(r"C:\Program Files\Warp\warp.exe");
        register_in(HKEY_CURRENT_USER, &scratch.path, "Warp", &exe).unwrap();
        assert_eq!(
            scratch.read("Warp").as_deref(),
            Some(r#""C:\Program Files\Warp\warp.exe""#)
        );
    }

    #[test]
    fn register_overwrites_previous_path() {
        let scratch = ScratchSubkey::new("register_overwrites_previous_path");
        register_in(
            HKEY_CURRENT_USER,
            &scratch.path,
            "Warp",
            &PathBuf::from(r"C:\old\warp.exe"),
        )
        .unwrap();
        register_in(
            HKEY_CURRENT_USER,
            &scratch.path,
            "Warp",
            &PathBuf::from(r"C:\new\warp.exe"),
        )
        .unwrap();
        assert_eq!(
            scratch.read("Warp").as_deref(),
            Some(r#""C:\new\warp.exe""#)
        );
    }

    #[test]
    fn unregister_is_idempotent() {
        let scratch = ScratchSubkey::new("unregister_is_idempotent");
        // Never registered: unregister should be Ok.
        unregister_in(HKEY_CURRENT_USER, &scratch.path, "Warp").unwrap();
        // Register, then unregister twice.
        register_in(
            HKEY_CURRENT_USER,
            &scratch.path,
            "Warp",
            &PathBuf::from(r"C:\warp.exe"),
        )
        .unwrap();
        unregister_in(HKEY_CURRENT_USER, &scratch.path, "Warp").unwrap();
        unregister_in(HKEY_CURRENT_USER, &scratch.path, "Warp").unwrap();
        assert!(scratch.read("Warp").is_none());
    }

    #[test]
    fn unregister_leaves_other_values_alone() {
        let scratch = ScratchSubkey::new("unregister_leaves_other_values_alone");
        register_in(
            HKEY_CURRENT_USER,
            &scratch.path,
            "Warp",
            &PathBuf::from(r"C:\warp.exe"),
        )
        .unwrap();
        register_in(
            HKEY_CURRENT_USER,
            &scratch.path,
            "WarpPreview",
            &PathBuf::from(r"C:\warp-preview.exe"),
        )
        .unwrap();

        unregister_in(HKEY_CURRENT_USER, &scratch.path, "Warp").unwrap();

        assert!(scratch.read("Warp").is_none());
        assert_eq!(
            scratch.read("WarpPreview").as_deref(),
            Some(r#""C:\warp-preview.exe""#)
        );
    }

    #[test]
    fn unregister_missing_subkey_is_ok() {
        unregister_in(
            HKEY_CURRENT_USER,
            r"Software\Warp\LoginItemTests\does-not-exist",
            "Warp",
        )
        .unwrap();
    }
}
