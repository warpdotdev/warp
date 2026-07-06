//! Generates a TOML settings file containing all registered public settings
//! with their default values.
//!
//! Unlike a runtime `SettingsManager` walk, this binary iterates the
//! `inventory::iter::<SettingSchemaEntry>` registry populated by the
//! `define_setting!` / `implement_setting_for_enum!` macros. Any setting
//! defined via those macros is picked up automatically — there is no
//! per-generator registration list to keep in sync.
//!
//! Usage:
//!   cargo run --example generate_default_settings -- --surface gui|tui [--channel dev|preview|stable] <output_path>
//!
//! Example:
//!   cargo run --example generate_default_settings -- --surface gui ./default_settings.toml

use std::collections::HashSet;
use std::path::PathBuf;

use settings::schema::SettingSchemaEntry;
use settings::SettingsMode;
use warp_core::features::{FeatureFlag, DEBUG_FLAGS, DOGFOOD_FLAGS, PREVIEW_FLAGS, RELEASE_FLAGS};
use warpui_extras::user_preferences::toml_backed::TomlBackedUserPreferences;
use warpui_extras::user_preferences::UserPreferences as _;

/// Ensures all `inventory::submit!` registrations from the app crate's
/// dependency tree are linked into the binary.
///
/// Binary targets only link crate code that is transitively referenced.
/// Without an explicit reference to the `warp` library, the linker will
/// not include most of the app's object files and the `inventory`
/// submissions they contain.
fn ensure_settings_linked() {
    let _ = std::hint::black_box(warp::settings::RESTORE_SESSION);
}

fn active_flags_for_channel(channel: &str) -> HashSet<FeatureFlag> {
    let mut flags = HashSet::new();

    let flag_lists: &[&[FeatureFlag]] = match channel {
        "stable" => &[RELEASE_FLAGS],
        "preview" => &[RELEASE_FLAGS, PREVIEW_FLAGS],
        "dev" => &[RELEASE_FLAGS, PREVIEW_FLAGS, DOGFOOD_FLAGS, DEBUG_FLAGS],
        other => {
            eprintln!("Unknown channel '{other}', defaulting to dev");
            &[RELEASE_FLAGS, PREVIEW_FLAGS, DOGFOOD_FLAGS, DEBUG_FLAGS]
        }
    };

    for list in flag_lists {
        for flag in *list {
            flags.insert(*flag);
        }
    }

    flags
}

fn main() {
    ensure_settings_linked();

    let args: Vec<String> = std::env::args().collect();

    let mut channel = "dev";
    let mut surface: Option<&str> = None;
    let mut output_path: Option<PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--channel" => {
                i += 1;
                if i < args.len() {
                    channel = &args[i];
                }
            }
            "--surface" => {
                i += 1;
                if i < args.len() {
                    surface = Some(&args[i]);
                } else {
                    eprintln!("Missing value for --surface (expected 'gui' or 'tui')");
                    std::process::exit(1);
                }
            }
            arg if !arg.starts_with('-') => {
                output_path = Some(PathBuf::from(arg));
            }
            other => {
                eprintln!("Unknown argument: {other}");
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let Some(output_path) = output_path else {
        eprintln!(
            "Usage: generate_default_settings --surface gui|tui [--channel dev|preview|stable] <output_path>"
        );
        std::process::exit(1);
    };

    let active_flags = active_flags_for_channel(channel);

    // Only emit settings that apply to the target surface, so e.g. the TUI
    // file excludes GUI-only keys. Required (with no default) so a typo or a
    // missing value never silently generates the wrong surface's file.
    let surface_mode = match surface {
        Some("gui") => SettingsMode::Gui,
        Some("tui") => SettingsMode::Tui,
        Some(other) => {
            eprintln!("Unknown surface '{other}' (expected 'gui' or 'tui')");
            std::process::exit(1);
        }
        None => {
            eprintln!("Missing required --surface (expected 'gui' or 'tui')");
            std::process::exit(1);
        }
    };

    // Generate a fresh document at `output_path`. If the file already exists
    // and contains invalid TOML, `TomlBackedUserPreferences::new` falls back
    // to an empty document and hands back the parse error; we ignore it here
    // because any subsequent writes will overwrite the file anyway.
    let (toml_prefs, _) = TomlBackedUserPreferences::new(output_path.clone());

    let mut written = 0usize;
    let mut failed = 0usize;

    for entry in inventory::iter::<SettingSchemaEntry> {
        // Skip private settings — they live in the platform-native store and
        // never appear in the user-visible TOML file.
        if entry.is_private {
            continue;
        }

        // Skip settings whose feature flag is not active for this channel.
        if let Some(flag) = entry.feature_flag {
            if !active_flags.contains(&flag) {
                continue;
            }
        }

        // Skip settings that don't apply to the target surface.
        if !(entry.surfaces_fn)().includes(surface_mode) {
            continue;
        }

        let default_json = (entry.file_default_value_fn)();

        if let Err(err) = toml_prefs.write_value_with_hierarchy(
            entry.storage_key,
            default_json,
            entry.hierarchy,
            entry.max_table_depth,
        ) {
            eprintln!("Warning: failed to write {}: {err}", entry.storage_key);
            failed += 1;
        } else {
            written += 1;
        }
    }

    println!(
        "Generated default settings at {} ({written} written, {failed} failed)",
        output_path.display()
    );
}
