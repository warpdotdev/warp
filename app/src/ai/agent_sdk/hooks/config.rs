use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use regex::Regex;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use super::trust::{HookTrustKey, HookTrustStore};
use super::{
    CONFIG_SCHEMA_VERSION, FailureMode, HookConfigSource, HookEventName, MAX_CONFIG_BYTES,
    MAX_HANDLERS_PER_FILE,
};

const USER_CONFIG_RELATIVE_PATH: &str = ".warp/hooks.json";
const PROJECT_CONFIG_RELATIVE_PATH: &str = ".warp/hooks.json";

#[derive(Clone, Debug)]
pub(crate) struct ConfiguredHook {
    pub(crate) event: HookEventName,
    pub(crate) matcher_text: Option<String>,
    matcher: Option<Regex>,
    pub(crate) command: String,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) command_windows: Option<String>,
    pub(crate) timeout: Duration,
    pub(crate) on_failure: FailureMode,
    pub(crate) source: HookConfigSource,
    pub(crate) config_path: PathBuf,
    pub(crate) definition_hash: String,
}

impl ConfiguredHook {
    pub(crate) fn matches(&self, subject: Option<&str>) -> bool {
        if self.event.ignores_matcher() {
            return true;
        }
        self.matcher
            .as_ref()
            .is_none_or(|matcher| subject.is_some_and(|subject| matcher.is_match(subject)))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HookConfigDiagnosticKind {
    Invalid,
    UntrustedProject,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HookConfigDiagnostic {
    pub(crate) path: PathBuf,
    pub(crate) kind: HookConfigDiagnosticKind,
    pub(crate) definition_hash: Option<String>,
    pub(crate) message: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct HookConfigSnapshot {
    handlers: Arc<Vec<ConfiguredHook>>,
    pub(crate) diagnostics: Arc<Vec<HookConfigDiagnostic>>,
}

impl HookConfigSnapshot {
    pub(crate) fn matching_handlers(
        &self,
        event: HookEventName,
        subject: Option<&str>,
    ) -> impl Iterator<Item = &ConfiguredHook> {
        self.handlers
            .iter()
            .filter(move |hook| hook.event == event && hook.matches(subject))
    }

    pub(crate) fn enabled_events(&self) -> impl Iterator<Item = HookEventName> + '_ {
        HookEventName::ALL
            .into_iter()
            .filter(|event| self.handlers.iter().any(|handler| handler.event == *event))
    }

    #[cfg(test)]
    pub(crate) fn handlers(&self) -> &[ConfiguredHook] {
        &self.handlers
    }
}

pub(crate) fn discover_hook_config(
    initial_cwd: &Path,
    trust_store: &dyn HookTrustStore,
) -> HookConfigSnapshot {
    let user_path = dirs::home_dir().map(|home| home.join(USER_CONFIG_RELATIVE_PATH));
    let project = git2::Repository::discover(initial_cwd)
        .ok()
        .and_then(|repository| repository.workdir().map(Path::to_path_buf))
        .and_then(|git_root| {
            let canonical_git_root = fs::canonicalize(git_root).ok()?;
            Some(ProjectConfig {
                path: canonical_git_root.join(PROJECT_CONFIG_RELATIVE_PATH),
                git_root: canonical_git_root,
            })
        });
    load_hook_config(user_path.as_deref(), project.as_ref(), trust_store)
}

#[derive(Clone, Debug)]
pub(crate) struct ProjectConfig {
    pub(crate) path: PathBuf,
    pub(crate) git_root: PathBuf,
}

pub(crate) fn load_hook_config(
    user_path: Option<&Path>,
    project: Option<&ProjectConfig>,
    trust_store: &dyn HookTrustStore,
) -> HookConfigSnapshot {
    let mut handlers = Vec::new();
    let mut diagnostics = Vec::new();

    if let Some(path) = user_path {
        load_file(
            path,
            HookConfigSource::User,
            None,
            trust_store,
            &mut handlers,
        )
        .err()
        .into_iter()
        .for_each(|diagnostic| diagnostics.push(diagnostic));
    }
    if let Some(project) = project {
        load_file(
            &project.path,
            HookConfigSource::Project,
            Some(&project.git_root),
            trust_store,
            &mut handlers,
        )
        .err()
        .into_iter()
        .for_each(|diagnostic| diagnostics.push(diagnostic));
    }

    HookConfigSnapshot {
        handlers: Arc::new(handlers),
        diagnostics: Arc::new(diagnostics),
    }
}

fn load_file(
    path: &Path,
    source: HookConfigSource,
    git_root: Option<&Path>,
    trust_store: &dyn HookTrustStore,
    handlers: &mut Vec<ConfiguredHook>,
) -> Result<(), HookConfigDiagnostic> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(invalid_diagnostic(
                path,
                format!("failed to read file: {error}"),
            ));
        }
    };
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(invalid_diagnostic(
            path,
            format!("file exceeds {MAX_CONFIG_BYTES} bytes"),
        ));
    }

    let raw: RawHookConfig = serde_json::from_slice(&bytes)
        .map_err(|error| invalid_diagnostic(path, format!("invalid JSON: {error}")))?;
    let parsed = validate_config(raw, source, path, &bytes)?;
    if source == HookConfigSource::Project {
        let canonical_git_root = git_root
            .and_then(|root| fs::canonicalize(root).ok())
            .ok_or_else(|| invalid_diagnostic(path, "project Git root is unavailable".into()))?;
        let canonical_path = fs::canonicalize(path).map_err(|error| {
            invalid_diagnostic(path, format!("failed to canonicalize path: {error}"))
        })?;
        let key = HookTrustKey {
            git_root: canonical_git_root,
            config_path: canonical_path,
            definition_hash: parsed.definition_hash.clone(),
        };
        if !trust_store.is_trusted(&key) {
            return Err(HookConfigDiagnostic {
                path: path.to_path_buf(),
                kind: HookConfigDiagnosticKind::UntrustedProject,
                definition_hash: Some(parsed.definition_hash),
                message: "project hook definition is not trusted".into(),
            });
        }
    }
    handlers.extend(parsed.handlers);
    Ok(())
}

fn invalid_diagnostic(path: &Path, message: String) -> HookConfigDiagnostic {
    HookConfigDiagnostic {
        path: path.to_path_buf(),
        kind: HookConfigDiagnosticKind::Invalid,
        definition_hash: None,
        message,
    }
}

struct ValidatedConfig {
    handlers: Vec<ConfiguredHook>,
    definition_hash: String,
}

fn validate_config(
    raw: RawHookConfig,
    source: HookConfigSource,
    path: &Path,
    bytes: &[u8],
) -> Result<ValidatedConfig, HookConfigDiagnostic> {
    if raw.schema_version != CONFIG_SCHEMA_VERSION {
        return Err(invalid_diagnostic(
            path,
            format!("unsupported schema_version {:?}", raw.schema_version),
        ));
    }
    let handler_count = raw
        .hooks
        .values()
        .flat_map(|groups| groups.iter())
        .map(|group| group.hooks.len())
        .sum::<usize>();
    if handler_count > MAX_HANDLERS_PER_FILE {
        return Err(invalid_diagnostic(
            path,
            format!("file exceeds {MAX_HANDLERS_PER_FILE} command handlers"),
        ));
    }

    let definition_hash = hex::encode(Sha256::digest(bytes));
    let mut handlers = Vec::with_capacity(handler_count);
    for (event, groups) in raw.hooks {
        for group in groups {
            if group.hooks.is_empty() {
                return Err(invalid_diagnostic(
                    path,
                    format!("{event} matcher group has no handlers"),
                ));
            }
            let matcher_text = group
                .matcher
                .filter(|matcher| !matcher.is_empty() && matcher != "*");
            let matcher = matcher_text
                .as_deref()
                .map(Regex::new)
                .transpose()
                .map_err(|error| {
                    invalid_diagnostic(path, format!("invalid {event} matcher: {error}"))
                })?;
            for raw_handler in group.hooks {
                let RawCommandHandler {
                    handler_type,
                    command,
                    command_windows,
                    timeout,
                    on_failure,
                } = raw_handler;
                match handler_type {
                    CommandHandlerType::Command => {}
                }
                if command.is_empty() {
                    return Err(invalid_diagnostic(
                        path,
                        format!("{event} command must be non-empty"),
                    ));
                }
                if on_failure == FailureMode::Deny && event != HookEventName::PreToolUse {
                    return Err(invalid_diagnostic(
                        path,
                        format!("on_failure deny is unsupported for {event}"),
                    ));
                }
                let timeout_seconds = timeout.unwrap_or_else(|| {
                    if event == HookEventName::SessionEnd {
                        1
                    } else {
                        10
                    }
                });
                let maximum = if event == HookEventName::SessionEnd {
                    3
                } else {
                    120
                };
                if timeout_seconds == 0 || timeout_seconds > maximum {
                    return Err(invalid_diagnostic(
                        path,
                        format!("{event} timeout must be between 1 and {maximum} seconds"),
                    ));
                }
                handlers.push(ConfiguredHook {
                    event,
                    matcher_text: matcher_text.clone(),
                    matcher: matcher.clone(),
                    command,
                    command_windows,
                    timeout: Duration::from_secs(timeout_seconds),
                    on_failure,
                    source,
                    config_path: path.to_path_buf(),
                    definition_hash: definition_hash.clone(),
                });
            }
        }
    }

    Ok(ValidatedConfig {
        handlers,
        definition_hash,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHookConfig {
    schema_version: String,
    hooks: BTreeMap<HookEventName, Vec<RawMatcherGroup>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMatcherGroup {
    matcher: Option<String>,
    hooks: Vec<RawCommandHandler>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommandHandler {
    #[serde(rename = "type")]
    handler_type: CommandHandlerType,
    command: String,
    command_windows: Option<String>,
    timeout: Option<u64>,
    #[serde(default)]
    on_failure: FailureMode,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum CommandHandlerType {
    Command,
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
