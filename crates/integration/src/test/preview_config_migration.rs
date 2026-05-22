use std::fs;
use std::path::PathBuf;

use warpui::integration::{AssertionOutcome, TestStep};

use super::wait_until_bootstrapped_single_pane_for_tab;
use crate::Builder;

/// Returns the current `$HOME` as a [`PathBuf`].
/// In integration tests, `HOME` is overridden to a hermetic temp directory.
fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME should be set in integration tests"))
}

/// Verifies that `migrate_config_dir_via_symlinks` creates symlinks from
/// an old config directory into a new one, skipping macOS metadata files.
pub fn test_preview_config_dir_migration() -> Builder {
    Builder::new()
        .with_setup(|utils| {
            let home = utils.test_dir();
            let old_dir = home.join(".warp");

            // Populate the old config directory with representative entries.
            fs::create_dir_all(old_dir.join("themes")).expect("create themes dir");
            fs::write(old_dir.join("keybindings.yaml"), "bindings")
                .expect("write keybindings.yaml");
            fs::write(old_dir.join("themes").join("dark.yaml"), "theme").expect("write dark.yaml");
            fs::create_dir_all(old_dir.join("workflows")).expect("create workflows dir");
            fs::write(old_dir.join(".mcp.json"), "{}").expect("write .mcp.json");
            // Files that should be excluded from the migration.
            fs::write(old_dir.join(".DS_Store"), "metadata").expect("write .DS_Store");
            fs::write(old_dir.join("._somefile"), "resource fork").expect("write ._somefile");
            fs::write(old_dir.join("settings.toml"), "[settings]").expect("write settings.toml");

            // Run the migration. We call the inner helper directly because the
            // integration channel is Integration, not Preview, so the public
            // entry point would no-op.
            let new_dir = home.join(".warp-preview");
            warp::integration_testing::preview_config_migration::run_config_dir_symlink_migration(
                &old_dir, &new_dir,
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Assert symlinks were created correctly")
                .add_named_assertion(
                    "new directory exists and is a real directory",
                    |_app, _window_id| {
                        let new_dir = home_dir().join(".warp-preview");

                        if !new_dir.is_dir() {
                            return AssertionOutcome::failure(
                                ".warp-preview should be a directory".to_string(),
                            );
                        }
                        // It should be a real directory, not a symlink.
                        let is_symlink = new_dir
                            .symlink_metadata()
                            .map(|m| m.file_type().is_symlink())
                            .unwrap_or(false);
                        if is_symlink {
                            return AssertionOutcome::failure(
                                ".warp-preview should not itself be a symlink".to_string(),
                            );
                        }
                        AssertionOutcome::Success
                    },
                )
                .add_named_assertion(
                    "expected entries are symlinks pointing to old dir",
                    |_app, _window_id| {
                        let home = home_dir();
                        let old_dir = home.join(".warp");
                        let new_dir = home.join(".warp-preview");

                        for name in ["keybindings.yaml", "themes", "workflows", ".mcp.json"] {
                            let link = new_dir.join(name);
                            let expected_target = old_dir.join(name);
                            match fs::read_link(&link) {
                                Ok(target) => {
                                    // Canonicalize both sides before comparing. On some
                                    // CI runners, the hermetic `$HOME` is reached through
                                    // a symlinked mount (e.g. `/Volumes/cache/...` vs
                                    // `/Users/runner/...`), so the raw symlink target and
                                    // the path we compute from `$HOME` may differ even
                                    // when they point to the same file.
                                    let target_canonical = fs::canonicalize(&target)
                                        .unwrap_or_else(|_| target.clone());
                                    let expected_canonical = fs::canonicalize(&expected_target)
                                        .unwrap_or_else(|_| expected_target.clone());
                                    if target_canonical != expected_canonical {
                                        return AssertionOutcome::failure(format!(
                                            "{name}: symlink points to {} (canonical {}), expected {} (canonical {})",
                                            target.display(),
                                            target_canonical.display(),
                                            expected_target.display(),
                                            expected_canonical.display(),
                                        ));
                                    }
                                }
                                Err(err) => {
                                    return AssertionOutcome::failure(format!(
                                        "{name}: not a symlink: {err}",
                                    ));
                                }
                            }
                        }
                        AssertionOutcome::Success
                    },
                )
                .add_named_assertion("excluded files were not symlinked", |_app, _window_id| {
                    let new_dir = home_dir().join(".warp-preview");

                    for name in [".DS_Store", "._somefile", "settings.toml"] {
                        if new_dir.join(name).exists() {
                            return AssertionOutcome::failure(format!(
                                "{name} should not be symlinked",
                            ));
                        }
                    }
                    AssertionOutcome::Success
                }),
        )
}
