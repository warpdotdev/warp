use std::path::{Path, PathBuf};

pub const ASSETS_DIR: &str = "assets";
pub const BUNDLED_ASSETS_DIR: &str = "bundled";
pub const ASYNC_ASSETS_DIR: &str = "async";
pub const REMOTE_ASSETS_DIR: &str = "remote";
pub const WINDOWS_ASSETS_DIR: &str = "windows";
pub const CONPTY_DLL_FILE: &str = "conpty.dll";
pub const OPEN_CONSOLE_EXE_FILE: &str = "OpenConsole.exe";
pub const DXCOMPILER_DLL_FILE: &str = "dxcompiler.dll";
pub const DXIL_DLL_FILE: &str = "dxil.dll";

/// Returns the relative path where an asset should be stored based on its path name and the sha256
/// hash of the contents.
/// The result will be of the form `path/to/file/filename-HASH.extension`
pub fn hashed_asset_path(asset_path: &Path, sha256_hash: &[u8]) -> PathBuf {
    // We use the sha256 hash here because that's also what's used by RustEmbed.
    let hash_str = hex::encode(sha256_hash);
    // There aren't many ways to manipulate PathBufs or OsStrings, so we build the new name
    // manually.
    let mut new_name = asset_path
        .file_stem()
        .expect("Path should not be empty")
        .to_os_string();
    new_name.push("-");
    new_name.push(hash_str);
    if let Some(extension) = asset_path.extension() {
        new_name.push(".");
        new_name.push(extension);
    }

    asset_path.with_file_name(new_name)
}

/// Returns a domain-relative URL of an async asset based on it's hashed asset path.
pub fn hashed_asset_url(hashed_asset_path: &Path) -> String {
    // This needs to be kept in sync with:
    // - The local asset server in the serve-wasm dir.
    // - The staging load balancer paths: https://console.cloud.google.com/net-services/loadbalancing/edit/http/serverless-lb?hl=en&project=warp-server-staging
    // - The prod load balancer paths: https://console.cloud.google.com/net-services/loadbalancing/edit/http/app-warp-dev-lb?hl=en&project=astral-field-294621
    format!(
        "/assets/client/static/{}",
        hashed_asset_path.to_str().unwrap()
    )
}

#[cfg(target_family = "wasm")]
/// Makes a domain-relative url absolute by prepending the current origin.
pub fn make_absolute_url(relative_url: &str) -> String {
    // This should be infallible.
    let origin = gloo::utils::window()
        .location()
        .origin()
        .expect("Can't get window origin.");

    format!("{origin}{relative_url}")
}
