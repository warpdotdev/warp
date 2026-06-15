use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub(crate) fn augmented_path() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    for path in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/opt/local/bin",
    ] {
        let path = PathBuf::from(path);
        if !paths.iter().any(|candidate| candidate == &path) {
            paths.push(path);
        }
    }
    for path in user_bin_paths() {
        if !paths.iter().any(|candidate| candidate == &path) {
            paths.push(path);
        }
    }
    paths
}

pub(crate) fn augmented_path_env() -> OsString {
    std::env::join_paths(augmented_path()).unwrap_or_default()
}

pub(crate) fn resolve_command(command: &str) -> Option<PathBuf> {
    let command_path = Path::new(command);
    if command_path.components().count() > 1 {
        return command_path.exists().then(|| command_path.to_path_buf());
    }

    augmented_path()
        .into_iter()
        .map(|path| path.join(command))
        .find(|path| path.is_file())
}

fn user_bin_paths() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };

    let mut paths = vec![
        home.join(".local/bin"),
        home.join("bin"),
        home.join(".cargo/bin"),
        home.join(".bun/bin"),
        home.join(".deno/bin"),
        home.join(".npm/bin"),
        home.join(".npm-global/bin"),
        home.join(".yarn/bin"),
        home.join(".config/yarn/global/node_modules/.bin"),
        home.join(".local/share/pnpm"),
        home.join("Library/pnpm"),
        home.join(".volta/bin"),
        home.join(".asdf/shims"),
        home.join(".local/share/mise/shims"),
    ];

    paths.extend(version_manager_bin_paths(&home.join(".nvm/versions/node")));
    paths.extend(version_manager_bin_paths(&home.join(".fnm/node-versions")));

    paths
}

fn version_manager_bin_paths(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("bin"))
        .filter(|path| path.is_dir())
        .collect()
}
