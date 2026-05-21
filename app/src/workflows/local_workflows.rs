use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use warp_util::path::ShellFamily;
use warp_workflows::workflows as global_workflows;
#[cfg(not(target_family = "wasm"))]
use warpui::platform::OperatingSystem;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

#[cfg(feature = "local_fs")]
use crate::user_config::load_workflows;
use crate::{terminal::model::session::Session, user_config::WarpConfig};

use super::{workflow::Workflow, WorkflowSource};

pub fn workflows_dir(base_dir: impl AsRef<Path>) -> PathBuf {
    base_dir.as_ref().join("workflows")
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UseCache {
    Yes,
    No,
}

/// Singleton model that loads and caches local (non-WarpDrive) workflows.
pub struct LocalWorkflows {
    app_workflows: Vec<Workflow>,

    global_workflows: Vec<Workflow>,

    project_workflows: HashMap<PathBuf, Vec<Workflow>>,
}

impl LocalWorkflows {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            app_workflows: app_workflows(),
            global_workflows: global_workflows().into_iter().map(Workflow::from).collect(), // convert from public-facing Workflow type to warp-internal Workflow type
            project_workflows: Default::default(),
        }
    }

    /// Returns an iterator over hardcoded "application" workflows included in the Warp binary.
    pub fn app_workflows(&self) -> impl Iterator<Item = &Workflow> {
        self.app_workflows.iter()
    }

    /// Returns an iterator over the static set of workflows for 3rd party tools loaded from Warp's
    /// workflows GitHub repo.
    pub fn global_workflows(
        &self,
        session: Option<Arc<Session>>,
    ) -> impl Iterator<Item = &Workflow> {
        let top_level_commands = session.map(|session| {
            session
                .top_level_commands()
                .map(|command| command.to_lowercase())
                .collect::<HashSet<_>>()
        });

        self.global_workflows.iter().filter(move |workflow| {
            let Some(top_level_commands) = top_level_commands.as_ref() else {
                return true;
            };

            if let Some(first_token) = workflow
                .command()
                .and_then(|command| command.split_ascii_whitespace().next())
            {
                // Show any workflows that start with punctuation, as we can't
                // be sure of what the first executable token is.
                return first_token.starts_with(|c: char| c.is_ascii_punctuation())
                            // Show any workflows that start with a token matching the top-level
                            // commands available in the user's session.
                            || top_level_commands.contains(&first_token.to_lowercase());
            }
            false
        })
    }

    /// Returns an iterator over file-based workflows loaded from the `.warp/workflows` directory in
    /// the `working_directory`.
    ///
    /// The loaded workflows vector is cached.
    ///
    /// If `use_cache` is `UseCache::Yes` and there is an existing cached vector, returns an
    /// iterator over the cached workflows.
    ///
    /// If `use_cache` is `UseCache::No`, reads the workflows from disk regardless of whether or
    /// not there is an existing cached vector and updates the cached vector.
    #[cfg(feature = "local_fs")]
    pub fn project_workflows(
        &mut self,
        working_directory: &Path,
        use_cache: UseCache,
    ) -> impl Iterator<Item = &Workflow> {
        let has_cached_copy = self.project_workflows.contains_key(working_directory);
        if !has_cached_copy || use_cache == UseCache::No {
            let repo_workflows = load_project_workflows(working_directory);
            self.project_workflows
                .insert(working_directory.to_owned(), repo_workflows);
        }

        self.project_workflows
            .get(working_directory)
            .expect("Workflows should exist; they were just inserted")
            .iter()
    }

    /// Returns the workflow with the given `command` along with its corresponding
    /// `WorkflowSource`.
    ///
    /// `command` is the parameterized `command` value of the workflow, e.g. `echo {{foo}}` if the
    /// workflow contains a parameter called "foo".
    pub fn workflow_with_command(
        &self,
        ctx: &AppContext,
        command: &str,
    ) -> Option<(WorkflowSource, Workflow)> {
        self.app_workflows
            .iter()
            .map(|workflow| (WorkflowSource::App, workflow))
            .chain(
                self.global_workflows
                    .iter()
                    .map(|workflow| (WorkflowSource::Global, workflow)),
            )
            .chain(
                self.project_workflows
                    .values()
                    .flatten()
                    .map(|workflow| (WorkflowSource::Project, workflow)),
            )
            .chain(
                WarpConfig::as_ref(ctx)
                    .local_user_workflows()
                    .iter()
                    .map(|workflow| (WorkflowSource::Local, workflow)),
            )
            .find(|(_, workflow)| {
                if let Workflow::Command {
                    command: workflow_command,
                    ..
                } = workflow
                {
                    workflow_command == command
                } else {
                    false
                }
            })
            .map(|(workflow_source, workflow)| (workflow_source, workflow.clone()))
    }
}

impl Entity for LocalWorkflows {
    type Event = ();
}

impl SingletonEntity for LocalWorkflows {}

/// Returns all app workflows.
fn app_workflows() -> Vec<Workflow> {
    #[cfg(not(target_family = "wasm"))]
    {
        let shell_family = OperatingSystem::get().default_shell_family();
        self::prompt_chip_logging_workflow(shell_family)
            .into_iter()
            .collect()
    }
    #[cfg(target_family = "wasm")]
    {
        Vec::new()
    }
}

/// Loads project-level workflows (if any) from the warp config directory in the current working
/// directory.
#[cfg(feature = "local_fs")]
pub(super) fn load_project_workflows(path: &Path) -> Vec<Workflow> {
    match git2::Repository::discover(path) {
        Ok(repository) => repository.workdir().map_or(Vec::new(), |workdir| {
            load_workflows(&workflows_dir(
                workdir.join(warp_core::paths::WARP_CONFIG_DIR),
            ))
        }),
        Err(_) => Vec::new(),
    }
}

/// Runs `tail` or equivalent command on the given path.
/// Note: On Windows this may cause a lossy conversion if the path is not valid UTF-8.
pub fn tail_command_for_shell(shell_family: ShellFamily, path: &PathBuf) -> String {
    match shell_family {
        // Use debug formatting for `PathBuf` so that any non-Unicode components of the path get
        // escaped.  This will also add quotes around the path, so there's no need to add them in
        // the format string.
        ShellFamily::Posix => format!("tail -f {path:?}"),
        // We avoid the debug formatting here so that backslashes don't get escaped, which is not
        // desirable for PowerShell.  Note that this may be lossy conversion if the path is not
        // valid UTF-8.
        ShellFamily::PowerShell => {
            format!("Get-Content -Wait -Tail 10 -Path \"{}\"", path.display())
        }
    }
}

#[cfg(not(target_family = "wasm"))]
pub fn prompt_chip_logging_workflow(shell_family: ShellFamily) -> Option<Workflow> {
    if !warp_core::channel::ChannelState::enable_debug_features() {
        return None;
    }
    let log_file_path = crate::context_chips::logging::log_file_path().ok()?;
    Some(Workflow::Command {
        name: "Tail prompt chip log".into(),
        command: tail_command_for_shell(shell_family, &log_file_path),
        tags: vec!["warp".into(), "debug".into()],
        description: Some(
            "Shows the diagnostic log of shell commands run by prompt context chips (dogfood only)"
                .into(),
        ),
        arguments: vec![],
        source_url: None,
        author: Some("Warp".into()),
        author_url: None,
        shells: vec![],
        environment_variables: None,
    })
}

#[cfg(test)]
#[path = "local_workflows_tests.rs"]
mod tests;
