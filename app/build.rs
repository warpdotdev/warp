// We can use `std::process:Command` here because this is invoked within a build script,
// _not_ within the Warp binary (where it could cause a terminal to temporarily flash on
// Windows).
#![allow(clippy::disallowed_types)]

use cfg_aliases::cfg_aliases;

use anyhow::Result;
use sha2::Digest;
use std::path::Path;
use std::{env, fs, process::Command};
use walkdir::WalkDir;
use warp_util::assets::{
    ASSETS_DIR, ASYNC_ASSETS_DIR, CONPTY_DLL_FILE, DXCOMPILER_DLL_FILE, DXIL_DLL_FILE,
    OPEN_CONSOLE_EXE_FILE, REMOTE_ASSETS_DIR, WINDOWS_ASSETS_DIR,
};
use warp_util::path::app_target_dir;

fn main() -> Result<()> {
    cfg_aliases! {
        linux_or_windows: { any(target_os = "linux", windows) },
        enable_crash_recovery: { linux_or_windows },
    }

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_FAMILY");

    let target_os = env::var("CARGO_CFG_TARGET_OS")?;
    let target_family = env::var("CARGO_CFG_TARGET_FAMILY")?;

    add_features(&target_family, &target_os);

    if target_os == "macos" && target_family != "wasm" {
        println!("cargo:rustc-link-lib=framework=MetalKit");
        println!("cargo:rustc-link-lib=framework=UserNotifications");

        println!("cargo:rerun-if-changed=src/platform/mac/objc/app_bundle.h");
        println!("cargo:rerun-if-changed=src/platform/mac/objc/app_bundle.m");
        println!("cargo:rerun-if-changed=src/platform/mac/objc/services.h");
        println!("cargo:rerun-if-changed=src/platform/mac/objc/services.m");

        cc::Build::new()
            .file("src/platform/mac/objc/app_bundle.m")
            .file("src/platform/mac/objc/services.m")
            .compile("warp_objc");

        // Build the dock tile plugin
        println!("cargo:rerun-if-changed=DockTilePlugin/WarpDockTilePlugin.m");
        println!("cargo:rerun-if-changed=DockTilePlugin/WarpDockTilePlugin.h");
        println!("cargo:rerun-if-changed=DockTilePlugin/Info.plist");
        println!("cargo:rerun-if-changed=DockTilePlugin/Makefile");

        let min_macos_version = env::var("MACOSX_DEPLOYMENT_TARGET")
            .expect("MACOSX_DEPLOYMENT_TARGET must be set for macos builds");
        let status = Command::new("make")
            .current_dir("DockTilePlugin")
            .env("MACOSX_DEPLOYMENT_TARGET", min_macos_version)
            .status()
            .expect("Failed to build dock tile plugin");
        if !status.success() {
            panic!("Dock tile plugin build failed");
        }

        // Copy the dock tile plugin to the output directory
        let profile = get_build_profile_name();
        let target_dir = app_target_dir(&profile).expect("Failed to get app target directory");
        let plugin_src = Path::new("DockTilePlugin/WarpDockTilePlugin.docktileplugin");
        let plugin_dst = target_dir.join("WarpDockTilePlugin.docktileplugin");

        if !status.success() {
            fs::remove_dir_all(plugin_src).expect("Failed to clean up plugin directory");
            panic!("Dock tile plugin build failed");
        }

        if plugin_src.exists() {
            fs::remove_dir_all(&plugin_dst).ok(); // Remove existing if any
            fs::create_dir_all(&plugin_dst).expect("Failed to create plugin directory");

            // Copy the plugin directory recursively
            for entry in WalkDir::new(plugin_src) {
                let entry = entry.expect("Failed to read plugin directory");
                let path = entry.path();
                let relative = path
                    .strip_prefix(plugin_src)
                    .expect("Failed to strip path prefix");
                let target = plugin_dst.join(relative);

                if path.is_dir() {
                    fs::create_dir_all(target).expect("Failed to create plugin subdirectory");
                } else {
                    fs::copy(path, target).expect("Failed to copy plugin file");
                }
            }

            // Clean up the source plugin directory after copying
            fs::remove_dir_all(plugin_src).expect("Failed to clean up plugin directory");
        }

        // In standalone mode, embed the Info.plist file. We don't use embed_plist! for this
        // because the plist file is dynamically generated.
        if env::var("CARGO_FEATURE_STANDALONE").is_ok() {
            // Don't fail if INFO_PLIST_PATH is unset, since CI runs clippy with --all-features.
            if let Ok(info_plist_path) = env::var("INFO_PLIST_PATH") {
                println!("cargo:rerun-if-env-changed=INFO_PLIST_PATH");
                println!("cargo:rerun-if-changed={info_plist_path}");
                println!("cargo:rustc-link-arg=-sectcreate");
                println!("cargo:rustc-link-arg=__TEXT");
                println!("cargo:rustc-link-arg=__info_plist");
                println!("cargo:rustc-link-arg={info_plist_path}");
            } else {
                eprintln!("Expected INFO_PLIST_PATH to be set")
            }
        }
    }

    if target_os == "windows" {
        // Retrieve the Cargo profile name so that we can put a copy of ConPTY in
        // the correct target subdirectory.
        //
        // We need to pass this information manually through an environment variable.
        // Of the built-in variables set by Cargo: `OUT_DIR` is only a temporary
        // directory, and `PROFILE` can only be `debug` or `release`.
        // See https://doc.rust-lang.org/cargo/reference/environment-variables.html#environment-variables-cargo-sets-for-build-scripts
        // for more on Cargo environment variables.
        //
        // Ideally we could access `CARGO_TARGET_DIR` but this doesn't exist at build time.
        // See https://github.com/rust-lang/cargo/issues/9661.
        //
        // Cargo defaults to the `debug` profile.
        let cargo_full_profile = env::var("CARGO_FULL_PROFILE").unwrap_or(String::from("debug"));
        let target_dir =
            app_target_dir(&cargo_full_profile).expect("Could not get app target directory");
        copy_windows_assets(&target_dir);

        #[cfg(windows)]
        embed_resource_file(&target_dir);
    }

    if target_family == "wasm" {
        copy_async_assets();
    }

    Ok(())
}

fn get_build_profile_name() -> String {
    // The profile name is always the 3rd last part of the path (with 1 based indexing).
    // e.g. /code/core/target/cli/build/my-build-info-9f91ba6f99d7a061/out
    env::var("OUT_DIR")
        .expect("OUT_DIR must be set")
        .split(std::path::MAIN_SEPARATOR)
        .nth_back(3)
        .expect("could not get profile name")
        .to_string()
}

fn add_features(target_family: &str, target_os: &str) {
    if target_family != "wasm" {
        println!("cargo:rustc-cfg=feature=\"local_fs\"");
        println!("cargo:rustc-cfg=feature=\"local_tty\"");
    }

    if target_os != "windows" {
        println!("cargo:rustc-cfg=feature=\"iterm_images\"");
    }

    if env::var("PROFILE").ok().is_some_and(|val| val == "debug") {
        println!("cargo:rustc-cfg=feature=\"agent_mode_debug\"");
    }
}

fn copy_async_assets() {
    println!("cargo:rerun-if-changed=assets/async");
    println!("cargo:rerun-if-env-changed=ASSET_TARGET_DIR");
    let Ok(out_dir_str) = env::var("ASSET_TARGET_DIR") else {
        // Don't build assets if no target dir specified.
        return;
    };
    let out_dir = Path::new(&out_dir_str);

    let remote_asset_subdirs = &[ASYNC_ASSETS_DIR, REMOTE_ASSETS_DIR];
    for remote_asset_subdir in remote_asset_subdirs {
        let asset_dir = Path::new(ASSETS_DIR).join(remote_asset_subdir);

        for asset in WalkDir::new(&asset_dir) {
            let asset = asset.expect("access error");
            let asset_path = asset.path();
            if asset_path.is_file() {
                let contents = fs::read(asset_path).expect("could not read file");

                let mut hasher = sha2::Sha256::new();
                hasher.update(&contents);
                let hash: [u8; 32] = hasher.finalize().into();
                let new_relative_path = warp_util::assets::hashed_asset_path(
                    asset_path
                        .strip_prefix(&asset_dir)
                        .expect("asset in unexpected location"),
                    &hash,
                );
                let new_path = out_dir.join(new_relative_path);

                fs::create_dir_all(new_path.parent().unwrap())
                    .expect("failed to create directories");
                fs::write(new_path, contents).expect("failed to copy file");
            }
        }
    }
}

/// Copies the DLLs needed to run Warp on Windows.
///
/// They are organized as follows:
/// - `conpty.dll`
/// - `{platform}/OpenConsole.exe` (ex: `x64/OpenConsole.exe`)
/// - `dxcompiler.dll` (ex: `dxcompiler.dll`)
/// - `dxil.dll` (ex: `dxil.dll`)
fn copy_windows_assets(target_dir: &Path) {
    println!("cargo:rerun-if-changed=assets/windows");

    let target_arch = match std::env::var("CARGO_CFG_TARGET_ARCH")
        .expect("Target arhcitecture expected")
        .as_str()
    {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => {
            panic!("Unsupported architecture");
        }
    };

    // This directory is architecture-specific.
    let windows_asset_dir = Path::new(ASSETS_DIR)
        .join(WINDOWS_ASSETS_DIR)
        .join(target_arch);

    // Copy conpty.dll into target directory.
    fs::copy(
        windows_asset_dir.join(CONPTY_DLL_FILE),
        target_dir.join(CONPTY_DLL_FILE),
    )
    .unwrap_or_else(|err| {
        panic!("Could not copy conpty.dll from {windows_asset_dir:?} to {target_dir:?}: {err:#}")
    });

    // Copy the DXC DLLs into the target directory.
    for dxc_file in [DXCOMPILER_DLL_FILE, DXIL_DLL_FILE] {
        fs::copy(
            windows_asset_dir.join(dxc_file),
            target_dir.join(dxc_file),
        )
        .unwrap_or_else(|err| {
            panic!("Could not copy {dxc_file} from {windows_asset_dir:?} to {target_dir:?}: {err:#}")
        });
    }

    // Copy OpenConsole.exe into {target_directory}/{arch}.
    let old_open_console_exe = windows_asset_dir.join(OPEN_CONSOLE_EXE_FILE);
    let new_platform_dir = target_dir.join(target_arch);
    let new_open_console_exe = new_platform_dir.join(OPEN_CONSOLE_EXE_FILE);
    fs::create_dir_all(&new_platform_dir).expect("Could not create new platform directory");
    fs::copy(old_open_console_exe, new_open_console_exe)
        .expect("Could not copy platform OpenConsole.exe");
}

#[cfg(windows)]
fn embed_resource_file(target_dir: &Path) {
    use std::io::Write;

    let version = env::var("GIT_RELEASE_TAG").unwrap_or("v0".to_owned());
    let app_name = env::var("WARP_APP_NAME").unwrap_or("Warp".to_owned());
    let bin_name = env::var("CARGO_BIN_NAME").unwrap_or("warp-oss".to_owned());
    let channel_name = match bin_name.as_str() {
        "warp-oss" => "oss",
        name => name,
    };

    let icon_path = Path::new("channels")
        .join(channel_name)
        .join("icon")
        .join("no-padding")
        .join("icon.ico");

    fs::copy(icon_path, target_dir.join("icon.ico"))
        .unwrap_or_else(|err| panic!("Could not copy icon: {err:#}"));

    let resource_file_path = target_dir.join("resource.rc");
    let mut rcfile = fs::File::create(&resource_file_path).unwrap();
    write!(
        rcfile,
        r#"
#pragma code_page(65001)
#include <winres.h>
#define IDI_ICON 0x101

IDI_ICON ICON "icon.ico"
VS_VERSION_INFO VERSIONINFO
FILEVERSION     1,0,0,0
PRODUCTVERSION  1,0,0,0
FILEFLAGSMASK   VS_FFI_FILEFLAGSMASK
FILEFLAGS       0
FILEOS          VOS__WINDOWS32
FILETYPE        VFT_APP
FILESUBTYPE     VFT2_UNKNOWN
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904E4"
        BEGIN
            VALUE "CompanyName",      "Warper contributors\0"
            VALUE "FileDescription",  "{app_name}\0"
            VALUE "FileVersion",      "{version}\0"
            VALUE "LegalCopyright",   "Copyleft 2026 Warper contributors\0"
            VALUE "InternalName",     "\0"
            VALUE "OriginalFilename", "\0"
            VALUE "ProductName",      "Warper\0"
            VALUE "ProductVersion",   "{version}\0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1252
    END
END
"#,
    )
    .unwrap();
    drop(rcfile);

    // Obtain MSVC environment so that the rc compiler can find the right headers.
    // https://github.com/nabijaczleweli/rust-embed-resource/issues/11#issuecomment-603655972
    let target = env::var("TARGET").unwrap();
    if let Some(tool) = cc::windows_registry::find_tool(target.as_str(), "cl.exe") {
        for (key, value) in tool.env() {
            env::set_var(key, value);
        }
    }
    embed_resource::compile(resource_file_path, embed_resource::NONE)
        .manifest_required()
        .unwrap_or_else(|err| panic!("Unable to embed resource file: {err:#}"));
}
