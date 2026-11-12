use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::Lines;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use tempfile::TempPath;
use uuid::Uuid;
use warpui::ModelSpawner;
use warpui::r#async::FutureExt as _;

use super::git_credentials::restricted::protected_environment_key;
use super::{AgentDriver, AgentDriverError};
use crate::terminal::shell::ShellType;

const VARS_FILE_ENV: &str = "WARP_VARS_FILE";

pub(super) struct SetupVars {
    path: PathBuf,
    values: HashMap<String, String>,
    metadata: HashMap<String, String>,
}

fn extend_environment(
    environment: &mut HashMap<OsString, OsString>,
    entries: impl IntoIterator<Item = (OsString, OsString)>,
) {
    for (key, value) in entries {
        if cfg!(windows) {
            environment.retain(|name, _| !same_key(name, &key.to_string_lossy()));
        }
        environment.insert(key, value);
    }
}

pub(super) fn file_path(working_dir: &Path, environment: &HashMap<OsString, OsString>) -> PathBuf {
    let root = environment
        .get(OsStr::new("OZ_WORKSPACE_ROOT"))
        .cloned()
        .or_else(|| std::env::var_os("OZ_WORKSPACE_ROOT"))
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| working_dir.to_owned());
    let root = if root.is_absolute() {
        root
    } else {
        working_dir.join(root)
    };
    root.join(".warp.vars")
}

fn valid_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn reserved_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    key.starts_with("WARP_") || key.starts_with("OZ_")
}

fn read_heredoc(lines: &mut Lines<'_>, delimiter: &str) -> Option<String> {
    if delimiter.is_empty() {
        return None;
    }
    let mut value = Vec::new();
    for line in lines {
        if line == delimiter {
            return Some(value.join("\n"));
        }
        value.push(line);
    }
    None
}

fn parse(contents: &str, metadata: &HashMap<String, String>) -> HashMap<String, String> {
    let mut values = HashMap::new();
    let mut lines = contents.trim_start_matches('\u{feff}').lines();
    while let Some(line) = lines.next() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = if let Some((key, value)) =
            line.split_once('=').filter(|(key, _)| !key.contains("<<"))
        {
            (key, value.to_owned())
        } else if let Some((key, delimiter)) = line.split_once("<<") {
            let Some(value) = read_heredoc(&mut lines, delimiter) else {
                warn_invalid_key(key);
                continue;
            };
            (key, value)
        } else {
            log::warn!("Skipping malformed .warp.vars line (key unknown)");
            continue;
        };
        if !valid_key(key) || value.contains('\0') {
            warn_invalid_key(key);
        } else if reserved_key(key) {
            if metadata.get(key) != Some(&value) {
                log::warn!("Skipping protected .warp.vars key {key}");
            }
        } else {
            let key = if cfg!(windows) {
                key.to_ascii_uppercase()
            } else {
                key.to_owned()
            };
            values.insert(key, value);
        }
    }
    values
}

fn warn_invalid_key(key: &str) {
    if valid_key(key) {
        log::warn!("Skipping malformed .warp.vars entry for key {key}");
    } else {
        // Invalid identifiers may contain arbitrary file contents, including secret values.
        log::warn!("Skipping malformed .warp.vars entry (invalid key)");
    }
}

fn open_vars_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;

        use windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

impl SetupVars {
    fn create(path: &Path, metadata: HashMap<String, String>) -> io::Result<Self> {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        write_metadata(&mut file, &metadata)?;
        Ok(Self {
            path: path.to_owned(),
            metadata,
            values: HashMap::new(),
        })
    }

    fn read(&mut self) {
        match std::fs::read_to_string(&self.path) {
            Ok(contents) => self.values = parse(&contents, &self.metadata),
            Err(error) => log::warn!("Could not read .warp.vars: {}", error.kind()),
        }
    }

    pub(super) fn write_assignment_metadata(
        &mut self,
        runtime_env: &HashMap<String, String>,
    ) -> io::Result<()> {
        self.metadata = runtime_env
            .iter()
            .filter(|(key, _)| key.starts_with("WARP_METADATA_") && valid_key(key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        write_metadata(&mut open_vars_file(&self.path)?, &self.metadata)
    }
}

fn write_metadata(file: &mut File, metadata: &HashMap<String, String>) -> io::Result<()> {
    for (key, value) in metadata {
        let delimiter = format!("WARP_METADATA_{}", Uuid::new_v4().simple());
        writeln!(file, "\n{key}<<{delimiter}\n{value}\n{delimiter}")?;
    }
    Ok(())
}

fn same_key(left: &OsStr, right: &str) -> bool {
    if cfg!(windows) {
        left.to_str()
            .is_some_and(|left| left.eq_ignore_ascii_case(right))
    } else {
        left == right
    }
}

fn prepend_path(value: &OsStr, existing: &OsStr) -> Option<OsString> {
    let mut seen = HashSet::new();
    let paths = std::env::split_paths(value)
        .chain(std::env::split_paths(existing))
        .filter(|path| {
            let identity = if cfg!(windows) {
                PathBuf::from(path.to_string_lossy().to_lowercase())
            } else {
                path.clone()
            };
            seen.insert(identity)
        });
    std::env::join_paths(paths).ok()
}

fn merge(
    values: &HashMap<String, String>,
    protected: &HashMap<OsString, OsString>,
    environment: &mut HashMap<OsString, OsString>,
) -> HashMap<OsString, OsString> {
    let mut applied = HashMap::new();
    for (key, value) in values {
        if same_key(OsStr::new(key), "PATH") {
            let existing_key = environment
                .keys()
                .find(|name| same_key(name, "PATH"))
                .cloned()
                .unwrap_or_else(|| OsString::from("PATH"));
            let existing = environment.get(&existing_key).cloned().unwrap_or_default();
            if let Some(path) = prepend_path(OsStr::new(value), &existing) {
                environment.insert(existing_key.clone(), path.clone());
                applied.insert(existing_key, path);
            } else {
                log::warn!("Skipping malformed .warp.vars PATH");
            }
        } else if reserved_key(key)
            || protected.keys().any(|name| same_key(name, key))
            || (protected
                .get(OsStr::new("WARP_GITHUB_CREDENTIAL_MODE"))
                .is_some_and(|mode| mode == "restricted_installation")
                && protected_environment_key(&key.to_ascii_uppercase()))
        {
            log::warn!("Skipping protected .warp.vars key {key}");
        } else {
            environment.insert(key.into(), value.into());
            applied.insert(key.into(), value.into());
        }
    }
    applied
}

fn quote(value: &str, shell: ShellType) -> String {
    let escaped = match shell {
        ShellType::Bash | ShellType::Zsh => value.replace('\'', "'\\''"),
        ShellType::Fish => value.replace('\\', "\\\\").replace('\'', "\\'"),
        ShellType::PowerShell => {
            // PowerShell also recognizes Unicode smart quotes as string delimiters.
            let encoded = STANDARD.encode(value);
            return format!(
                "([System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String('{encoded}')))"
            );
        }
    };
    format!("'{escaped}'")
}

fn assignment_script(values: &HashMap<OsString, OsString>, shell: ShellType) -> String {
    let mut script = String::new();
    for (key, value) in values {
        let (Some(key), Some(value)) = (key.to_str(), value.to_str()) else {
            continue;
        };
        let value = quote(value, shell);
        let command = match shell {
            ShellType::Bash | ShellType::Zsh => format!("export {key}={value}\n"),
            ShellType::Fish => format!("set -gx {key} {value}\n"),
            ShellType::PowerShell => format!("$env:{key} = {value}\n"),
        };
        script.push_str(&command);
    }
    script
}

fn private_assignment_script(
    values: &HashMap<OsString, OsString>,
    shell: ShellType,
) -> io::Result<(TempPath, String)> {
    let suffix = match shell {
        ShellType::Bash | ShellType::Zsh => ".sh",
        ShellType::Fish => ".fish",
        ShellType::PowerShell => ".ps1",
    };
    let mut file = tempfile::Builder::new()
        .prefix("warp-vars-")
        .suffix(suffix)
        .tempfile()?;
    file.write_all(assignment_script(values, shell).as_bytes())?;
    let file = file.into_temp_path();
    let path = quote(&file.to_string_lossy(), shell);
    let command = match shell {
        ShellType::Bash | ShellType::Zsh => format!(". {path}"),
        ShellType::Fish => format!("source {path}"),
        ShellType::PowerShell => format!(
            ". ([scriptblock]::Create([System.IO.File]::ReadAllText({path}, [System.Text.Encoding]::UTF8)))"
        ),
    };
    Ok((file, command))
}

impl AgentDriver {
    pub(super) async fn initialize_setup_vars(
        foreground: &ModelSpawner<Self>,
        include_metadata: bool,
    ) -> Result<(), AgentDriverError> {
        foreground
            .spawn(move |me, _| {
                me.setup_vars = None;
                let Some(path) = me.resolved_env_vars.get(OsStr::new(VARS_FILE_ENV)) else {
                    log::warn!("Could not initialize .warp.vars: exported path is missing");
                    return;
                };
                let mut runtime: HashMap<_, _> = std::env::vars_os().collect();
                extend_environment(
                    &mut runtime,
                    me.resolved_env_vars
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone())),
                );
                let metadata = if include_metadata {
                    runtime
                        .iter()
                        .filter_map(|(key, value)| {
                            let (key, value) = (key.to_str()?, value.to_str()?);
                            (key.starts_with("WARP_METADATA_") && valid_key(key))
                                .then(|| (key.to_owned(), value.to_owned()))
                        })
                        .collect()
                } else {
                    HashMap::new()
                };
                match SetupVars::create(Path::new(path), metadata) {
                    Ok(vars) => me.setup_vars = Some(vars),
                    Err(error) => {
                        log::warn!("Could not initialize .warp.vars: {}", error.kind());
                    }
                }
            })
            .await?;
        Ok(())
    }

    pub(super) async fn read_setup_vars(
        foreground: &ModelSpawner<Self>,
        apply_to_session: bool,
        update_mcp: bool,
    ) -> Result<(), AgentDriverError> {
        let (values, mcp_environment) = foreground
            .spawn(move |me, ctx| {
                let protected = &me.resolved_env_vars;
                let mut environment: HashMap<_, _> = std::env::vars_os().collect();
                extend_environment(
                    &mut environment,
                    me.resolved_env_vars
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone())),
                );
                if apply_to_session
                    && let Some(path) = me.terminal_driver.as_ref(ctx).session_path(ctx)
                {
                    extend_environment(&mut environment, [("PATH".into(), path.into())]);
                }
                let Some(vars) = me.setup_vars.as_mut() else {
                    return (HashMap::new(), HashMap::new());
                };
                vars.read();
                let applied = merge(&vars.values, protected, &mut environment);
                let mut mcp: HashMap<_, _> = std::env::vars_os().collect();
                let mcp_applied = merge(&vars.values, protected, &mut mcp);
                let mut mcp_environment: HashMap<_, _> = mcp_applied
                    .into_iter()
                    .filter_map(|(key, value)| {
                        Some((key.into_string().ok()?, value.into_string().ok()?))
                    })
                    .collect();
                if !mcp_environment.is_empty() {
                    mcp_environment.insert(
                        VARS_FILE_ENV.into(),
                        vars.path.to_string_lossy().into_owned(),
                    );
                }
                extend_environment(Arc::make_mut(&mut me.resolved_env_vars), applied.clone());
                (applied, mcp_environment)
            })
            .await?;
        if apply_to_session {
            Self::apply_setup_vars_to_session(foreground, values).await?;
        }
        if update_mcp && !mcp_environment.is_empty() {
            let result = Self::replace_prewarmed_mcp_environment(foreground, mcp_environment).await;
            Self::handle_mcp_startup_result(result, foreground).await?;
        }
        Ok(())
    }

    pub(super) fn merge_assignment_setup_vars(
        &mut self,
        mcp_environment: &mut HashMap<String, String>,
    ) {
        let Some(vars) = self.setup_vars.as_ref() else {
            return;
        };
        let protected = self.resolved_env_vars.clone();
        let mut environment: HashMap<_, _> = std::env::vars_os().collect();
        extend_environment(
            &mut environment,
            self.resolved_env_vars
                .iter()
                .map(|(k, v)| (k.clone(), v.clone())),
        );
        let applied = merge(&vars.values, &protected, &mut environment);
        extend_environment(Arc::make_mut(&mut self.resolved_env_vars), applied);
        Arc::make_mut(&mut self.resolved_env_vars)
            .insert(VARS_FILE_ENV.into(), vars.path.as_os_str().to_owned());
        let mut mcp: HashMap<_, _> = std::env::vars_os().collect();
        extend_environment(
            &mut mcp,
            mcp_environment.iter().map(|(k, v)| (k.into(), v.into())),
        );
        let applied = merge(&vars.values, &protected, &mut mcp);
        mcp_environment.extend(
            applied.into_iter().filter_map(|(key, value)| {
                Some((key.into_string().ok()?, value.into_string().ok()?))
            }),
        );
        mcp_environment.insert(
            VARS_FILE_ENV.into(),
            vars.path.to_string_lossy().into_owned(),
        );
    }

    async fn apply_setup_vars_to_session(
        foreground: &ModelSpawner<Self>,
        values: HashMap<OsString, OsString>,
    ) -> Result<(), AgentDriverError> {
        if values.is_empty() {
            return Ok(());
        }
        let shell = foreground
            .spawn(|me, ctx| {
                me.terminal_driver
                    .as_ref(ctx)
                    .active_session_shell_type(ctx)
            })
            .await?
            .ok_or(AgentDriverError::InvalidRuntimeState)?;
        // Keep values out of terminal command text and error attribution. Only this generated,
        // private assignment script is evaluated; .warp.vars itself is never shell input.
        let (file, command) = private_assignment_script(&values, shell).map_err(|_| {
            AgentDriverError::EnvironmentSetupFailed(
                "Could not prepare setup variable assignments".into(),
            )
        })?;
        let status = async {
            let handle = foreground
                .spawn(move |me, ctx| {
                    me.terminal_driver
                        .update(ctx, |driver, ctx| driver.execute_command(&command, ctx))
                })
                .await??
                .await?;
            handle.await
        }
        .with_timeout(Duration::from_secs(30))
        .await
        .map_err(|_| {
            AgentDriverError::EnvironmentSetupFailed("Setup variable assignments timed out".into())
        })??;
        drop(file);
        if status != 0.into() {
            return Err(AgentDriverError::EnvironmentSetupFailed(
                "Could not apply setup variable assignments".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "setup_vars_tests.rs"]
mod tests;
