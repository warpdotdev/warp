use std::env;
use std::path::PathBuf;

use tini::Ini;

static CURSOR_DIR_NAME: &'static &str = &"cursors";
static CURSOR_INDEX_FILE_NAME: &'static &str = &"index.theme";
static THEME_FILE_CURSOR_SECTION: &'static &str = &"Icon Theme";
static THEME_FILE_INHERITS_KEY: &'static &str = &"Inherits";

static ENV_DATA_DIRS: &'static &str = &"XDG_DATA_DIRS";
static ENV_CURSOR_THEME: &'static &str = &"XCURSOR_THEME";

static DEFAULT_THEME: &'static &str = &"default";
static KNOWN_THEMES: &[&str] = &["Yaru", "Adwaita"];

pub fn ensure_cursor_theme() {
    // If the XCURSOR_THEME value is explicitly set,
    // then we do not want to modify the user's environment
    if env::var(ENV_CURSOR_THEME).is_ok() {
        return;
    }

    let crawler = CursorThemeCrawler::new();

    if let Some(theme) = crawler.determine_cursor_theme() {
        // winit and it's dependencies will automatically check for
        // the default theme, so we do not need to mess with the
        // env var here.
        if theme != *DEFAULT_THEME {
            env::set_var(ENV_CURSOR_THEME, theme);
        }
    }
}

struct CursorThemeCrawler {
    /// Directories to search when looking for a cursor theme.
    /// Directories are searched in vec order from first to last.
    /// However, because themes can reference other themes, it is
    /// possible for search results to traverse multiple directories.
    /// For example, a default theme can be found in directories[1]
    /// that inherits from a theme in directories[3], which itself
    /// inherits from a theme in directories[0]
    directories: Vec<PathBuf>,
}

fn non_empty_var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|val| !val.is_empty())
}

impl CursorThemeCrawler {
    pub fn new() -> Self {
        // Per https://specifications.freedesktop.org/icon-theme-spec/icon-theme-spec-latest.html#directory_layout,
        // we search:
        // - $HOME/.icons (for backwards compatibility)
        // - $XDG_DATA_HOME/icons (technically this should be part of XDG_DATA_DIRS, but we add it in here)
        //     - Defaults to $HOME/.local/share
        // - $XDG_DATA_DIRS/icons
        //     - Defaults to /usr/local/share/:/usr/share/
        // - /usr/share/pixmaps
        let mut directories = vec![];

        let xdg_data_dirs = non_empty_var(ENV_DATA_DIRS)
            .or_else(|| Some("/usr/local/share/:/usr/share/".to_string()));

        if let Some(home) = dirs::home_dir() {
            directories.push(home.join(".icons"));
        }
        if let Some(xdg_data_home) = dirs::data_dir() {
            directories.push(xdg_data_home.join("icons"));
        }
        if let Some(xdg_data_dirs) = xdg_data_dirs {
            for dir in xdg_data_dirs.split(':') {
                if !dir.is_empty() {
                    directories.push(PathBuf::from(dir).join("icons"));
                }
            }
        }
        directories.push(PathBuf::from("/usr/share/pixmaps"));
        Self { directories }
    }

    /// First checks to see if there is a default cursor theme set.
    /// If there is no default set, we check a list of known themes.
    /// The first theme to be confirmed exist is returned, else None
    /// is returned.
    fn determine_cursor_theme(&self) -> Option<String> {
        if self.check_cursor_theme(DEFAULT_THEME) {
            return Some(DEFAULT_THEME.to_string());
        }

        for theme in KNOWN_THEMES {
            if self.check_cursor_theme(theme) {
                return Some((*theme).to_string());
            }
        }
        None
    }

    /// Returns true if an icon theme exists and has a `cursors/`
    /// folder, indicating that the cursors for that theme are installed.
    /// Per the specification, an icon theme can exist along multiple
    /// directories. As long as at least one of those directories
    /// contains the `cursors/` subdir, we consider it valid
    fn check_cursor_theme_installed(&self, theme: &str) -> bool {
        for dir in &self.directories {
            if dir.join(theme).join(CURSOR_DIR_NAME).exists() {
                return true;
            }
        }
        false
    }

    /// Checks that a given icon theme is a valid cursor theme.
    ///
    /// When we check a cursor theme, we verify that either:
    /// a. The icon theme has a cursors/ folder
    /// b. The icon theme inherits from an existing cursor theme.
    ///
    /// This can cause us to traverse multiple themes as part of our validation.
    /// we do this verification to handle cases like the `adwaita-icon-theme`
    /// deb packages, which sets Adwaita to the default icon theme without
    /// installing a cursor theme.
    fn check_cursor_theme(&self, root_theme: &str) -> bool {
        let mut visited = std::collections::HashSet::from([root_theme.to_string()]);
        let mut pending = std::collections::VecDeque::from([root_theme.to_string()]);

        while let Some(theme) = pending.pop_front() {
            if self.check_cursor_theme_installed(&theme) {
                return true;
            }

            // Per the spec, the **first** index.theme found when traversing
            // the directories is used
            let inherited_themes = &self
                .directories
                .iter()
                .filter_map(|index_dir| {
                    let index_path = index_dir.join(&theme).join(CURSOR_INDEX_FILE_NAME);
                    if let Ok(theme_file) = Ini::from_file(&index_path) {
                        theme_file.get_vec_with_sep::<String>(
                            THEME_FILE_CURSOR_SECTION,
                            THEME_FILE_INHERITS_KEY,
                            ",",
                        )
                    } else {
                        None
                    }
                })
                .next();

            if let Some(inherited_themes) = inherited_themes {
                for new_theme in inherited_themes {
                    if !visited.contains(new_theme) {
                        visited.insert(new_theme.clone());
                        pending.push_back(new_theme.clone());
                    }
                }
            }
        }

        false
    }
}

#[cfg(test)]
#[path = "cursor_theme_tests.rs"]
mod tests;
