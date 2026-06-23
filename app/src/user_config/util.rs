// Allowing dead code when targeting wasm as most of the functions in this
// module are only used on native.
#![cfg_attr(target_family = "wasm", allow(dead_code))]

use std::fs;
use std::path::{Path, PathBuf};

use itertools::Itertools;
use serde::de::DeserializeOwned;
use walkdir::{DirEntry, WalkDir};

use crate::launch_configs::launch_config::LaunchConfig;
use crate::tab_configs::{TabConfig, TabConfigError};
use crate::themes::theme::{ThemeKind, WarpTheme, WarpThemeConfig};
use crate::workflows::workflow::Workflow;

const CONFIG_FILE_SUFFIXES: &[&str] = &[".yaml", ".yml"];
const TOML_CONFIG_FILE_SUFFIX: &str = ".toml";

fn get_file_name(item: &DirEntry) -> Option<String> {
    match item.metadata() {
        Ok(metadata) if metadata.is_file() => item.file_name().to_str().map(|s| s.to_string()),
        // The item was something else, like a directory.
        Ok(_) => None,
        // The file was deleted between when we generated the DirEntry and now.
        Err(_) => None,
    }
}

pub fn from_yaml<R>(path: PathBuf) -> anyhow::Result<R>
where
    R: DeserializeOwned,
{
    let file = fs::File::open(path.as_path())?;
    let reader = std::io::BufReader::new(file);

    let u: R = serde_yaml::from_reader(reader)?;
    Ok(u)
}

pub fn from_toml<R>(path: PathBuf) -> anyhow::Result<R>
where
    R: DeserializeOwned,
{
    let contents = fs::read_to_string(path.as_path())?;
    let u: R = toml::from_str(&contents)?;
    Ok(u)
}

/// Deserializes a `DirEntry` into an object of type `G` if the file is a valid
/// config file containing a single item.
fn parse_single_item_file<T, F, G>(item: &DirEntry, post_deserialize_fn: F) -> Option<G>
where
    F: Fn(String, T) -> G,
    T: DeserializeOwned,
{
    if let Some(file_name) = get_file_name(item) {
        if is_config_file(&file_name) {
            let parsed = from_yaml::<T>(item.path().into());
            match parsed {
                Ok(parsed) => return Some(post_deserialize_fn(file_name, parsed)),
                Err(e) => {
                    log::warn!("Failed to parse config file at {file_name:?} with error: {e:?}")
                }
            }
        }
    }
    None
}

fn from_multi_doc_yaml<R>(path: PathBuf) -> anyhow::Result<Vec<R>>
where
    R: DeserializeOwned,
{
    let file = fs::File::open(path.as_path())?;
    let reader = std::io::BufReader::new(file);

    serde_yaml::Deserializer::from_reader(reader)
        .map(|document| R::deserialize(document).map_err(Into::into))
        .collect()
}

/// Deserializes a `DirEntry` into an object of type `Vec<G>` if the file is a
/// valid config file containing one or more items.
fn parse_multi_item_file<T, F, G>(item: &DirEntry, post_deserialize_fn: F) -> Option<Vec<G>>
where
    F: Fn(String, T) -> G,
    T: DeserializeOwned,
{
    if let Some(file_name) = get_file_name(item) {
        if is_config_file(&file_name) {
            let parsed = from_multi_doc_yaml::<T>(item.path().into());
            match parsed {
                Ok(parsed) => {
                    return Some(
                        parsed
                            .into_iter()
                            .map(|val| post_deserialize_fn(file_name.clone(), val))
                            .collect_vec(),
                    );
                }
                Err(e) => {
                    log::warn!("Failed to parse config file at {file_name:?} with error: {e:?}")
                }
            }
        }
    }
    None
}

fn title_case(s: &str) -> String {
    let lowercase = s.to_lowercase();
    let mut chars = lowercase.chars();
    chars
        .next()
        .map(|first_letter| first_letter.to_uppercase())
        .into_iter()
        .flatten()
        .chain(chars)
        .collect()
}

pub fn file_name_to_human_readable_name(file_name: &str) -> String {
    let name = if let Some(suffix) = CONFIG_FILE_SUFFIXES
        .iter()
        .find(|&suffix| file_name.ends_with(suffix))
    {
        file_name.strip_suffix(suffix).unwrap_or(file_name)
    } else {
        file_name
    };

    name_to_camel_case(name)
}

fn name_to_camel_case(name: &str) -> String {
    // Camel Case conversion (with spaces) treating each '_' as word separator.
    // solarized_dark.yaml => Solarized Dark
    // SOLARIZED_DARK.yaml => Solarized Dark
    // SolarizedDark.yaml => Solarizeddark (note: no '_', so treating as single word)
    name.split('_').map(title_case).join(" ")
}

pub(super) fn parse_single_theme_dir_entry(item: &DirEntry) -> Option<(ThemeKind, WarpTheme)> {
    parse_single_item_file(item, |file_name, mut theme: WarpTheme| {
        // If the name exists in the .yaml, we use it. Otherwise we treat a "human readable" version of the filename as the theme name.
        let theme_kind = if let Some(name) = theme.name() {
            WarpThemeConfig::file_to_theme(name, item.path().into())
        } else {
            let name = file_name_to_human_readable_name(file_name.as_str());
            theme.set_name(name.clone());
            WarpThemeConfig::file_to_theme(name, item.path().into())
        };

        (theme_kind, theme)
    })
}

pub(super) fn parse_multi_workflow_dir_entry(item: &DirEntry) -> Option<Vec<Workflow>> {
    parse_multi_item_file(item, |_, workflow| workflow)
}

pub(super) fn parse_multi_launch_config_dir_entry(item: &DirEntry) -> Option<Vec<LaunchConfig>> {
    parse_multi_item_file(item, |_file_name, config| config)
}

pub(super) fn parse_tab_config_dir_entry(
    item: &DirEntry,
) -> Option<Result<TabConfig, TabConfigError>> {
    let file_name = get_file_name(item)?;
    if !is_toml_file(&file_name) {
        return None;
    }
    let parsed = from_toml::<TabConfig>(item.path().into());
    Some(
        parsed
            .map(|mut config| {
                config.source_path = Some(item.path().into());
                config
            })
            .map_err(|e| TabConfigError {
                file_name,
                file_path: item.path().into(),
                error_message: e.to_string(),
            }),
    )
}

/// Runs the given function on each `DirEntry` within the `Path`. If the path is not a directory,
/// an empty `Vector` is returned. It works recursively, covering directories within a given path.
pub(super) fn for_each_dir_entry<F, T>(path: &Path, dir_entry_fn: F) -> Vec<T>
where
    F: Fn(&DirEntry) -> Option<T>,
{
    if path.is_dir() {
        WalkDir::new(path)
            .follow_links(true)
            .into_iter()
            .filter_map(Result::ok)
            .filter_map(|item| dir_entry_fn(&item))
            .collect()
    } else {
        vec![]
    }
}

/// Must end with config file suffix
pub(super) fn is_config_file(file_name: &str) -> bool {
    CONFIG_FILE_SUFFIXES
        .iter()
        .any(|&suffix| file_name.ends_with(suffix))
}

/// Must end with `.toml`
pub(super) fn is_toml_file(file_name: &str) -> bool {
    file_name.ends_with(TOML_CONFIG_FILE_SUFFIX)
}

/// Must have a name beyond the suffix
pub(super) fn has_name(file_name: &str) -> bool {
    CONFIG_FILE_SUFFIXES
        .iter()
        .all(|&suffix| file_name != suffix && !file_name.is_empty())
}

#[cfg(test)]
#[path = "util_tests.rs"]
mod tests;
