// We can use `std::process:Command` here because this is invoked within a build script,
// _not_ within the Warp binary (where it could cause a terminal to temporarily flash on
// Windows).
#![allow(clippy::disallowed_types)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use anyhow::Result;
use cfg_aliases::cfg_aliases;
use sha2::Digest;
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
        build_and_link_sentry();

        println!("cargo:rerun-if-changed=src/platform/mac/objc/services.h");
        println!("cargo:rerun-if-changed=src/platform/mac/objc/services.m");

        cc::Build::new()
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
        // `CARGO_FULL_PROFILE` is set by bundle scripts for custom profiles (e.g.
        // release-lto). Fall back to Cargo's built-in `PROFILE` ("debug"/"release")
        // for direct `cargo build` invocations. See also:
        // https://doc.rust-lang.org/cargo/reference/environment-variables.html#environment-variables-cargo-sets-for-build-scripts
        //
        // Ideally we could access `CARGO_TARGET_DIR` but this doesn't exist at build time.
        // See https://github.com/rust-lang/cargo/issues/9661.
        let cargo_full_profile = env::var("CARGO_FULL_PROFILE")
            .or_else(|_| env::var("PROFILE"))
            .unwrap_or_else(|_| String::from("debug"));
        let target_dir =
            app_target_dir(&cargo_full_profile).expect("Could not get app target directory");
        copy_windows_assets(&target_dir);

        #[cfg(windows)]
        embed_resource_file(&target_dir);
    }

    if target_family == "wasm" {
        copy_async_assets();
    }

    generate_channel_config_if_needed(&target_family, &target_os);

    Ok(())
}

/// If `warp-channel-config` is available on PATH and the `release_bundle` feature is enabled,
/// invoke the config generator binary and write the JSON output to `OUT_DIR` so it can be
/// embedded via `include_str!` in the binary entry points.
fn generate_channel_config_if_needed(target_family: &str, target_os: &str) {
    if env::var("CARGO_FEATURE_RELEASE_BUNDLE").is_err() {
        // For non-bundled builds, config is loaded at runtime — nothing to embed.
        return;
    }

    let config_bin = "warp-channel-config";

    // Check if the config binary is available on PATH. If not, we can't generate embedded
    // configs. This is expected for external contributors building Warp OSS.
    if Command::new(config_bin)
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
    {
        return;
    }

    // Only track these for bundled builds, where they affect the embedded config.
    // For non-bundled builds these are runtime variables and should not trigger recompilation.
    println!("cargo:rerun-if-env-changed=WITH_LOCAL_SERVER");
    println!("cargo:rerun-if-env-changed=WITH_LOCAL_SESSION_SHARING_SERVER");
    println!("cargo:rerun-if-env-changed=WITH_SANDBOX_TELEMETRY");
    println!("cargo:rerun-if-env-changed=SERVER_ROOT_URL");
    println!("cargo:rerun-if-env-changed=WS_SERVER_URL");

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR must be set");
    let family_arg = if target_family == "wasm" {
        "wasm"
    } else {
        "native"
    };

    // Generate config for all internal channels. The build script runs once per crate (not
    // once per binary), so we generate all configs here and each binary's include_str! picks
    // up its own file.
    for channel in ["local", "dev", "stable", "preview"] {
        let output = Command::new(config_bin)
            .arg("--channel")
            .arg(channel)
            .arg("--target-family")
            .arg(family_arg)
            .arg("--target-os")
            .arg(target_os)
            .output()
            .unwrap_or_else(|err| {
                panic!("Failed to execute config generator at '{config_bin}': {err}")
            });

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            panic!("Config generator failed for channel '{channel}':\n{stderr}");
        }

        let config_path = Path::new(&out_dir).join(format!("{channel}_config.json"));
        fs::write(&config_path, &output.stdout).unwrap_or_else(|err| {
            panic!("Failed to write config to {}: {err}", config_path.display())
        });
    }
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

fn build_and_link_sentry() {
    // Ensure we re-run the build script if the target framework directory changes.
    println!("cargo:rerun-if-env-changed=FRAMEWORK_OVERRIDE");

    // If the cocoa_sentry feature is not enabled, there's nothing more to do here.
    if env::var("CARGO_FEATURE_COCOA_SENTRY").is_err() {
        return;
    }

    // Download/update the Sentry framework.
    let dir_name = env::var("FRAMEWORK_OVERRIDE").unwrap_or_else(|_| "default".to_string());
    let frameworks_dir = format!("frameworks/{dir_name}");
    let standalone = env::var("CARGO_FEATURE_STANDALONE").is_ok();
    download_sentry_framework(&frameworks_dir, &dir_name, standalone);

    let sentry_dir = if standalone {
        "Sentry.xcframework"
    } else {
        "Sentry-Dynamic-WithARM64e.xcframework"
    };
    let sentry_framework_path = format!("{frameworks_dir}/{sentry_dir}/macos-arm64_arm64e_x86_64");

    // Make sure we re-run the build script if the framework directory changes (e.g.: it gets
    // deleted).
    println!("cargo:rerun-if-changed={sentry_framework_path}");

    // Link the Sentry framework, and compile our objc logic that interfaces with it.
    println!("cargo:rustc-link-search=framework=app/{sentry_framework_path}");
    println!("cargo:rustc-link-lib=framework=Sentry");

    // If building standalone, we need to copy some flags from the Sentry build that the static library requires.
    if standalone {
        println!("cargo:rustc-link-lib=c++");
        let swift_library_path = get_xcode_toolchain().join("usr/lib/swift/macosx");
        println!(
            "cargo:rustc-link-search=all={}",
            swift_library_path.display()
        );
        // Embed /usr/lib/swift as LC_RPATH so the deployed standalone CLI binary can
        // find Swift runtime dylibs on the remote host. On macOS 12.3+, Apple ships
        // the Swift runtime as an OS-level framework at /usr/lib/swift. Without this,
        // remote macOS hosts fail with:
        //   dyld: Library not loaded: @rpath/libswiftCore.dylib
        //   Reason: no LC_RPATH's found
        // See: https://github.com/warpdotdev/warp/issues/12631
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }

    compile_sentry_objc_lib(&sentry_framework_path);
}

fn download_sentry_framework(frameworks_dir: &str, dir_name: &str, standalone: bool) {
    // Build absolute path to the script from workspace root.
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.parent().unwrap();
    let script_path = workspace_root.join("script/macos/update_sentry_cocoa");

    let cocoa_sentry_version = match dir_name {
        "default" | "dev" => "9.4.1",
        name => panic!("Invalid framework override: {name}"),
    };

    let mut cmd = Command::new(&script_path);
    cmd.current_dir(workspace_root)
        .arg("--dir")
        .arg(format!("app/{frameworks_dir}"))
        .arg("--version")
        .arg(cocoa_sentry_version);

    if standalone {
        cmd.arg("--static");
    }

    let output = cmd
        .output()
        .expect("Failed to run update_sentry_cocoa script");
    if !output.status.success() {
        panic!(
            "Failed to download/update Sentry frameworks:\n--- stdout\n{}\n--- stderr\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn compile_sentry_objc_lib(sentry_framework_path: &str) {
    println!("cargo:rerun-if-changed=src/platform/mac/objc/crash_reporting.h");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/crash_reporting.m");

    // We need to tell `Clang` to build with a specific framework path. This is represented within
    // Clang by the `-F` flag, which is not supported directly in the `cc::Build` API, so we
    // directly pass the flag and its value instead.
    cc::Build::new()
        .file("src/platform/mac/objc/crash_reporting.m")
        .flag(format!("-F{sentry_framework_path}").as_str())
        .compile("warp_sentry_objc");
}

#[cfg(unix)]
fn get_xcode_toolchain() -> PathBuf {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let mut output = Command::new("xcode-select")
        .arg("-p")
        .output()
        .expect("Could not run xcode-select");
    if !output.status.success() {
        panic!(
            "`xcode-select -p` failed:\n\n--- stdout\n{}\n\n--- stderr\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Trim trailing whitespace.
    while output
        .stdout
        .last()
        .is_some_and(|b| b.is_ascii_whitespace())
    {
        output.stdout.pop();
    }

    PathBuf::from(OsString::from_vec(output.stdout)).join("Toolchains/XcodeDefault.xctoolchain")
}

#[cfg(not(unix))]
fn get_xcode_toolchain() -> PathBuf {
    panic!("get_xcode_toolchain is only supported on macOS")
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
    let bin_name = env::var("CARGO_BIN_NAME").unwrap_or("local".to_owned());

    let icon_path = Path::new("channels")
        .join(bin_name)
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
            VALUE "CompanyName",      "Denver Technologies, Inc\0"
            VALUE "FileDescription",  "{app_name}\0"
            VALUE "FileVersion",      "{version}\0"
            VALUE "LegalCopyright",   "© 2025, Denver Technologies, Inc\0"
            VALUE "InternalName",     "\0"
            VALUE "OriginalFilename", "\0"
            VALUE "ProductName",      "Warp\0"
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
