pub mod external_editor;
use std::path::{Path, PathBuf};
use std::{fs, io};

#[cfg(windows)]
use warp_util::path::is_network_resource;
use warp_util::path::{CleanPathResult, LineAndColumnArg};

pub use self::external_editor::{open_file_path_in_external_editor, open_file_path_with_editor};
use crate::terminal::ShellLaunchData;
use crate::terminal::model::grid::grid_handler::{ContainsPoint, Link};
use crate::terminal::model::index::Point;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilePathType {
    Absolute,
    /// Contains the working directory PathBuf.
    Relative(PathBuf),
}

#[derive(Debug)]
pub enum ShellPathType {
    /// The path comes from the shell and may need to be converted in a shell-aware way.
    ShellNative(String),
    /// The path has already been converted to a OS-native path.
    PlatformNative(PathBuf),
}

/// Checks if a file path exists and is valid for a file link.
pub fn absolute_path_if_valid(
    clean_path_result: &CleanPathResult,
    working_directory: ShellPathType,
    shell_launch_data: Option<&ShellLaunchData>,
) -> Option<PathBuf> {
    let (maybe_absolute_path, relative_path) = match shell_launch_data {
        Some(shell_launch_data) => {
            // Attempt to parse the clean path result as an absolute path.
            let maybe_absolute_path =
                shell_launch_data.maybe_convert_absolute_path(&clean_path_result.path);
            let relative_path = match working_directory {
                ShellPathType::ShellNative(base_path_str) => shell_launch_data
                    .maybe_convert_relative_path(&base_path_str, &clean_path_result.path),
                ShellPathType::PlatformNative(base_path) => {
                    shell_launch_data.join_to_native_path(&base_path, &clean_path_result.path)
                }
            };
            (maybe_absolute_path, relative_path)
        }
        None => {
            // We naively attempt to treat the given paths as platform-native.
            let maybe_absolute_path = PathBuf::from(&clean_path_result.path);
            let relative_path = match working_directory {
                ShellPathType::ShellNative(path_str) => {
                    let mut path_buf = PathBuf::from(path_str);
                    path_buf.push(&clean_path_result.path);
                    path_buf
                }
                ShellPathType::PlatformNative(path_buf) => path_buf.join(&clean_path_result.path),
            };
            (Some(maybe_absolute_path), Some(relative_path))
        }
    };

    if relative_path
        .as_ref()
        .is_some_and(|path| is_path_valid(path, clean_path_result))
    {
        return relative_path;
    } else if maybe_absolute_path
        .as_ref()
        .is_some_and(|path| is_path_valid(path, clean_path_result))
    {
        return maybe_absolute_path;
    }

    None
}

fn is_path_valid(path: &Path, clean_path_result: &CleanPathResult) -> bool {
    // Checking for the existence of a network resource takes a long time (~15s),
    // and hangs the UI, so we skip validating it.
    #[cfg(windows)]
    if is_network_resource(path) {
        return false;
    }

    // It should only be a valid path if the path links to a file or a folder without
    // line and column number attached.
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    metadata.is_file() || (metadata.is_dir() && clean_path_result.line_and_column_num.is_none())
}

impl FilePathType {
    /// Given a path that we've identified the FilePathType of,
    /// returns the absolute path.
    pub fn absolute_path(&self, path: PathBuf) -> PathBuf {
        match self {
            FilePathType::Absolute => path,
            FilePathType::Relative(directory) => directory.join(&path),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileLink {
    pub link: Link,
    /// This path has been converted (if needed) into a native path from the shell.
    pub absolute_path: PathBuf,
    pub line_and_column_num: Option<LineAndColumnArg>,
}

impl FileLink {
    pub fn absolute_path(&self) -> Option<PathBuf> {
        Some(self.absolute_path.clone())
    }
}

impl ContainsPoint for FileLink {
    fn contains(&self, point: Point) -> bool {
        self.link.contains(point)
    }
}

/// Creates the file at the given path if it doesn't already exist, opening it
/// in write mode. If any directories in the path are missing, those are created
/// as well.
///
/// This always returns an error for unit tests, as they should not directly
/// interact with the filesystem.
pub fn create_file<P: AsRef<Path>>(_path: P) -> io::Result<fs::File> {
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            Err(io::Error::from_raw_os_error(1))
        } else {
            let path = _path.as_ref();
            fs::create_dir_all(path.parent().ok_or_else(|| {
                io::Error::other(
                    "full_path should never be root directory.",
                )
            })?)?;
            fs::File::create(path)
        }
    }
}
