// TODO: restrict what we make public here.
mod builtins;
pub mod context_chip;
pub mod current_prompt;
pub mod directory_fetcher;
pub mod display;
pub mod display_chip;
pub mod display_menu;
pub(crate) mod git_branch_on_click;
pub(crate) mod logging;
pub mod node_version_popup;
pub mod prompt;
pub mod prompt_snapshot;
pub mod prompt_type;
pub mod renderer;
pub mod spacing;

use std::collections::HashMap;
use std::time::Duration;

use context_chip::PromptGenerator;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use warpui::{
    color::ColorU,
    elements::Text,
    fonts::{Properties, Weight},
};

use crate::ui_components::{blended_colors, icons::Icon};
use crate::{appearance::Appearance, features::FeatureFlag, themes::theme::PromptColors};

#[allow(unused_imports)]
pub use self::context_chip::{
    ChipAvailability, ChipDisabledReason, ChipRuntimeCapabilities, ExternalCommandsAvailability,
};
use self::{
    context_chip::{ChipFingerprintInput, ChipRuntimePolicy, ContextChip, RefreshConfig},
    renderer::RendererStyles,
};

/// The value of a context chip. Most chips produce plain text, but some
/// (like `GitDiffStats`) carry structured data to avoid string round-trips.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChipValue {
    Text(String),
    GitDiffStats(display_chip::GitLineChanges),
}

impl ChipValue {
    /// Returns the text representation, or `None` for non-text variants.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ChipValue::Text(s) => Some(s),
            ChipValue::GitDiffStats(_) => None,
        }
    }

    /// Returns the `GitLineChanges` payload, if this is the `GitDiffStats` variant.
    pub fn as_git_diff_stats(&self) -> Option<&display_chip::GitLineChanges> {
        match self {
            ChipValue::GitDiffStats(g) => Some(g),
            ChipValue::Text(_) => None,
        }
    }
}

impl Default for ChipValue {
    fn default() -> Self {
        ChipValue::Text(String::new())
    }
}

impl std::fmt::Display for ChipValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChipValue::Text(s) => f.write_str(s),
            ChipValue::GitDiffStats(g) => {
                write!(
                    f,
                    "{} • +{} -{}",
                    g.files_changed, g.lines_added, g.lines_removed
                )
            }
        }
    }
}

impl From<String> for ChipValue {
    fn from(s: String) -> Self {
        ChipValue::Text(s)
    }
}

pub(crate) fn github_pr_number_from_url(url: &str) -> Option<&str> {
    let (_, tail) = url.trim().rsplit_once("/pull/")?;
    let number = tail.split(['/', '?', '#']).next()?;
    (!number.is_empty() && number.chars().all(|c| c.is_ascii_digit())).then_some(number)
}

pub(crate) fn github_pr_display_text_from_url(url: &str) -> Option<String> {
    github_pr_number_from_url(url).map(|number| format!("PR #{number}"))
}

/// The refresh settings for the date context chip.
/// unless the clock strikes midnight without the user running a command.
const DATE_REFRESH_CONFIG: RefreshConfig = RefreshConfig::Periodically {
    interval: Duration::from_secs(30 * 60),
};

/// The refresh settings for the time context chip.
const TIME_REFRESH_CONFIG: RefreshConfig = RefreshConfig::Periodically {
    interval: Duration::from_secs(1),
};

/// Refresh settings for Git context chips.
const GIT_REFRESH_CONFIG: RefreshConfig =
    // TODO: Should we watch .git/HEAD instead? Needs to be relative to the current Git repo.
    RefreshConfig::Periodically {
        interval: Duration::from_secs(30),
    };

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChipResult {
    kind: ContextChipKind,
    value: Option<ChipValue>,
    on_click_values: Vec<String>,
}

impl ChipResult {
    pub fn kind(&self) -> &ContextChipKind {
        &self.kind
    }

    pub fn value(&self) -> Option<&ChipValue> {
        self.value.as_ref()
    }

    pub fn on_click_values(&self) -> &[String] {
        &self.on_click_values
    }
}

#[derive(
    Serialize,
    Deserialize,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Hash,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Type of prompt context chip.",
    rename_all = "snake_case"
)]
pub enum ContextChipKind {
    WorkingDirectory,
    Username,
    Hostname,
    Date,
    Time12,
    Time24,
    VirtualEnvironment,
    CondaEnvironment,
    NodeVersion,
    #[schemars(description = "A user-defined custom chip.")]
    Custom {
        title: String,
    },
    ShellGitBranch,
    GitDiffStats,
    GithubPullRequest,
    KubernetesContext,
    SvnBranch,
    SvnDirtyItems,
    // This is for backwards compatibility with the old "RemoteLogin" chip.
    // We originally had two different chips for different input types, this has since been consolidated.
    #[serde(alias = "RemoteLogin")]
    Ssh,
    Subshell,
    /// A chip that shows the plan and todo list for the current conversation.
    AgentPlanAndTodoList,
}

impl ContextChipKind {
    pub fn to_chip(&self) -> Option<ContextChip> {
        match self {
            Self::WorkingDirectory => Some(ContextChip::builtin_with_runtime_policy(
                "Working Directory",
                builtins::working_directory,
                RefreshConfig::OnDemandOnly,
                ChipRuntimePolicy::new(
                    std::iter::empty::<&str>(),
                    false,
                    None,
                    [
                        ChipFingerprintInput::SessionId,
                        ChipFingerprintInput::WorkingDirectory,
                    ],
                ),
            )),
            Self::Username => Some(ContextChip::builtin_with_runtime_policy(
                "User",
                builtins::username,
                RefreshConfig::OnDemandOnly,
                ChipRuntimePolicy::new(
                    std::iter::empty::<&str>(),
                    false,
                    None,
                    [ChipFingerprintInput::SessionId],
                ),
            )),
            Self::Hostname => Some(ContextChip::builtin_with_runtime_policy(
                "Host",
                builtins::hostname,
                RefreshConfig::OnDemandOnly,
                ChipRuntimePolicy::new(
                    std::iter::empty::<&str>(),
                    false,
                    None,
                    [ChipFingerprintInput::SessionId],
                ),
            )),
            Self::VirtualEnvironment => Some(ContextChip::builtin_with_runtime_policy(
                "Python Virtualenv",
                builtins::virtual_environment,
                RefreshConfig::OnDemandOnly,
                ChipRuntimePolicy::new(
                    std::iter::empty::<&str>(),
                    false,
                    None,
                    [
                        ChipFingerprintInput::SessionId,
                        ChipFingerprintInput::PythonVirtualenv,
                    ],
                ),
            )),
            Self::CondaEnvironment => Some(ContextChip::builtin_with_runtime_policy(
                "Conda Environment",
                builtins::conda_environment,
                RefreshConfig::OnDemandOnly,
                ChipRuntimePolicy::new(
                    std::iter::empty::<&str>(),
                    false,
                    None,
                    [
                        ChipFingerprintInput::SessionId,
                        ChipFingerprintInput::CondaEnvironment,
                    ],
                ),
            )),
            Self::NodeVersion => Some(ContextChip::builtin_with_runtime_policy(
                "Node.js Version",
                builtins::node_version,
                RefreshConfig::OnDemandOnly,
                ChipRuntimePolicy::new(
                    std::iter::empty::<&str>(),
                    false,
                    None,
                    [
                        ChipFingerprintInput::SessionId,
                        ChipFingerprintInput::NodeVersion,
                    ],
                ),
            )),
            Self::Date => Some(ContextChip::builtin(
                "Date",
                builtins::date,
                DATE_REFRESH_CONFIG,
            )),
            Self::Time12 => Some(ContextChip::builtin(
                "Time (12-hour format)",
                builtins::time12,
                TIME_REFRESH_CONFIG,
            )),
            Self::Time24 => Some(ContextChip::builtin(
                "Time (24-hour format)",
                builtins::time24,
                TIME_REFRESH_CONFIG,
            )),
            Self::Custom { title } => {
                log::warn!("Tried to use custom chip {title}");
                None
            }
            Self::ShellGitBranch => Some(ContextChip::shell_builtin(
                "Git Branch",
                builtins::shell_git_branch(),
                Some(builtins::shell_other_git_branches()),
                GIT_REFRESH_CONFIG,
            )),
            Self::GitDiffStats => Some(
                ContextChip::shell_builtin(
                    "Git Diff Stats",
                    builtins::shell_git_line_changes(),
                    None,
                    GIT_REFRESH_CONFIG,
                )
                .with_allow_empty_value(),
            ),
            Self::GithubPullRequest if !FeatureFlag::GithubPrPromptChip.is_enabled() => None,
            Self::GithubPullRequest => {
                let generator = builtins::github_pull_request_url();
                let policy = ChipRuntimePolicy::new(
                    generator.dependencies().to_vec(),
                    true,
                    Some(Duration::from_secs(5)),
                    [
                        ChipFingerprintInput::SessionId,
                        ChipFingerprintInput::WorkingDirectory,
                        ChipFingerprintInput::GitBranch,
                        ChipFingerprintInput::RequiredExecutablesPresence,
                        ChipFingerprintInput::InvalidatingCommandCount,
                    ],
                )
                .with_suppress_on_failure()
                .with_invalidate_on_commands(["git", "gh", "gt"]);
                Some(ContextChip::shell_builtin_with_runtime_policy(
                    "GitHub Pull Request",
                    generator,
                    None,
                    GIT_REFRESH_CONFIG,
                    policy,
                ))
            }
            Self::KubernetesContext => Some(ContextChip::shell_builtin(
                "Kubernetes Context",
                builtins::kubernetes_current_context(),
                None,
                RefreshConfig::OnDemandOnly,
            )),
            Self::SvnBranch => Some(ContextChip::shell_builtin(
                "Svn Branch",
                builtins::svn_branch_context(),
                None,
                RefreshConfig::OnDemandOnly,
            )),
            Self::SvnDirtyItems => Some(ContextChip::shell_builtin(
                "Svn Uncommitted File Count",
                builtins::svn_dirty_items(),
                None,
                RefreshConfig::OnDemandOnly,
            )),
            Self::Ssh => Some(ContextChip::builtin(
                "Remote Login",
                builtins::ssh_session,
                RefreshConfig::OnDemandOnly,
            )),
            Self::Subshell => Some(ContextChip::builtin(
                "subshell",
                builtins::subshell,
                RefreshConfig::OnDemandOnly,
            )),
            Self::AgentPlanAndTodoList => Some(ContextChip::builtin(
                "Agent Plan and Todo List",
                |_| Some(ChipValue::Text(String::new())),
                RefreshConfig::OnDemandOnly,
            )),
        }
    }

    /// Whether the context chip has a copyable value.
    pub fn is_copyable(&self) -> bool {
        !matches!(self, Self::AgentPlanAndTodoList)
    }

    /// Returns a generator to be used for the first fetch of
    /// a periodic generator. Is mostly used to use the PreCmd value
    /// of git-branch for ShellGitBranch, while using a shell command
    /// for the periodic updates.
    pub fn initial_value_generator(&self) -> Option<PromptGenerator> {
        match self {
            Self::ShellGitBranch => Some(PromptGenerator::Contextual {
                from_context_fn: |context| {
                    context
                        .current_environment
                        .git_branch()
                        .map(|s| ChipValue::Text(s.to_string()))
                },
            }),
            _ => None,
        }
    }

    /// TODO: we might need to move this API to support custom chips.
    pub fn placeholder_value(&self) -> ChipValue {
        match self {
            Self::WorkingDirectory => ChipValue::Text("~/Desktop".to_string()),
            Self::Username => ChipValue::Text("alice".to_string()),
            Self::Hostname => ChipValue::Text("ubuntu-04".to_string()),
            Self::ShellGitBranch => ChipValue::Text("git-feature-branch".to_string()),
            Self::GitDiffStats => ChipValue::Text("3 • +10 -2".to_string()),
            Self::GithubPullRequest => ChipValue::Text("PR #123".to_string()),
            Self::VirtualEnvironment => ChipValue::Text("pyenv".to_string()),
            Self::CondaEnvironment => ChipValue::Text("condaenv".to_string()),
            Self::NodeVersion => ChipValue::Text("v18.17.0".to_string()),
            Self::Date => ChipValue::Text("July 12, 2023".to_string()),
            Self::Time12 => ChipValue::Text("03:48 pm".to_string()),
            Self::Time24 => ChipValue::Text("15:48".to_string()),
            Self::Custom { .. } => ChipValue::Text("custom chip".to_string()),
            Self::KubernetesContext => ChipValue::Text("kube-context".to_string()),
            Self::SvnBranch => ChipValue::Text("svn-feature-branch".to_string()),
            Self::SvnDirtyItems => ChipValue::Text("3".to_string()),
            Self::Ssh => ChipValue::Text("alice@127.0.0.1".to_string()),
            Self::Subshell => ChipValue::Text("bash".to_string()),
            Self::AgentPlanAndTodoList => ChipValue::Text("Plan and Todo List".to_string()),
        }
    }

    pub fn default_styles(
        &self,
        appearance: &Appearance,
        is_in_agent_view: bool,
    ) -> RendererStyles {
        if is_in_agent_view {
            return RendererStyles::new(agent_view_chip_color(appearance), Properties::default());
        }
        let prompt_colors: PromptColors = appearance.theme().clone().into();

        let color = match self {
            Self::WorkingDirectory => prompt_colors.input_prompt_pwd,
            Self::Username => prompt_colors.input_prompt_user_and_host,
            Self::Hostname => prompt_colors.input_prompt_user_and_host,
            Self::ShellGitBranch => prompt_colors.input_prompt_branch,
            Self::GitDiffStats => prompt_colors.input_prompt_branch,
            Self::GithubPullRequest => prompt_colors.input_prompt_branch,
            Self::VirtualEnvironment => prompt_colors.input_prompt_virtual_env,
            Self::CondaEnvironment => prompt_colors.input_prompt_virtual_env,
            Self::NodeVersion => prompt_colors.input_prompt_virtual_env,
            Self::Date => prompt_colors.input_prompt_date,
            Self::Time12 => prompt_colors.input_prompt_time,
            Self::Time24 => prompt_colors.input_prompt_time,
            Self::KubernetesContext => prompt_colors.input_prompt_kubernetes,
            Self::SvnBranch => prompt_colors.input_prompt_branch,
            Self::SvnDirtyItems => prompt_colors.input_prompt_svn,
            Self::Ssh => prompt_colors.input_prompt_ssh,
            Self::Subshell => prompt_colors.input_prompt_subshell,
            Self::AgentPlanAndTodoList => prompt_colors.input_prompt_agent_mode_hint,
            Self::Custom { .. } => ColorU::new(255, 255, 255, 255),
        };

        let font_properties = Properties::default().weight(Weight::Semibold);

        RendererStyles::new(color, font_properties)
    }

    /// The name of this context chip to use in telemetry, or `None` if it should not
    /// be reported at all.
    ///
    /// This lets us measure which chips are being used without reporting private
    /// user-created chips.
    pub fn telemetry_name(&self) -> Option<String> {
        match self {
            Self::Custom { .. } => None,
            chip => Some(format!("{chip:?}")),
        }
    }

    /// Formats a value of this context chip for display.
    ///
    /// This is temporary until chip prefixes/suffixes are user-configurable.
    /// Keep in sync with [`display::PromptDisplay`].
    pub fn display_value(&self, value: &ChipValue) -> String {
        let text = value.to_string();
        match self {
            Self::ShellGitBranch => format!("git:({text})"),
            Self::GithubPullRequest => github_pr_display_text_from_url(&text).unwrap_or(text),
            Self::KubernetesContext => format!("⎈ {text}"),
            Self::SvnBranch => format!("svn:({text})"),
            Self::SvnDirtyItems => format!("±{text}"),
            _ => text,
        }
    }

    /// Whether or not a context chip should render given the current command
    /// in the input.
    pub fn should_render(&self, command: &str, aliases: &HashMap<SmolStr, String>) -> bool {
        match self {
            Self::KubernetesContext => {
                const KUBERNETES_COMMANDS: [&str; 20] = [
                    "kubectl",
                    "helm",
                    "kubens",
                    "kubectx",
                    "oc",
                    "istioctl",
                    "kogito",
                    "k9s",
                    "helmfile",
                    "flux",
                    "fluxctl",
                    "stern",
                    "kubeseal",
                    "skaffold",
                    "kubent",
                    "kubecolor",
                    "cmctl",
                    "sparkctl",
                    "etcd",
                    "fubectl",
                ];

                command.split_whitespace().next().is_some_and(|first_word| {
                    KUBERNETES_COMMANDS.contains(&first_word)
                        || aliases.get(first_word).is_some_and(|expanded| {
                            KUBERNETES_COMMANDS.contains(&expanded.as_str())
                        })
                })
            }

            // All other chips unconditionally render
            _ => true,
        }
    }

    pub fn udi_icon(&self) -> Option<Icon> {
        match self {
            Self::WorkingDirectory => Some(Icon::Folder),
            Self::Username | Self::Ssh => Some(Icon::User),
            Self::Hostname => Some(Icon::Laptop),
            Self::Date => Some(Icon::CalendarDate),
            Self::Time12 | Self::Time24 => Some(Icon::Clock),
            Self::VirtualEnvironment | Self::CondaEnvironment | Self::Subshell => {
                Some(Icon::Terminal)
            }
            Self::NodeVersion => Some(Icon::NodeJS),
            Self::ShellGitBranch | Self::SvnBranch => Some(Icon::GitBranch),
            Self::GitDiffStats | Self::SvnDirtyItems => Some(Icon::File),
            Self::GithubPullRequest => Some(Icon::Github),
            Self::KubernetesContext => Some(Icon::Globe),
            Self::AgentPlanAndTodoList => Some(Icon::CheckSkinny),
            Self::Custom { .. } => None,
        }
    }
}

/// Returns the set of chips that are available for use in the agent footer.
pub fn agent_footer_available_chips() -> Vec<ContextChipKind> {
    let mut chips = available_chips();
    chips.push(ContextChipKind::AgentPlanAndTodoList);
    chips
}

/// TODO: this needs to also fetch the custom chips from sqlite
pub fn available_chips() -> Vec<ContextChipKind> {
    let mut chips = vec![
        ContextChipKind::WorkingDirectory,
        ContextChipKind::Username,
        ContextChipKind::Hostname,
        ContextChipKind::Ssh,
        ContextChipKind::ShellGitBranch,
        ContextChipKind::GitDiffStats,
    ];
    if FeatureFlag::GithubPrPromptChip.is_enabled() {
        chips.push(ContextChipKind::GithubPullRequest);
    }
    chips.extend([
        ContextChipKind::Date,
        ContextChipKind::Time12,
        ContextChipKind::Time24,
        ContextChipKind::VirtualEnvironment,
        ContextChipKind::CondaEnvironment,
        ContextChipKind::NodeVersion,
        ContextChipKind::KubernetesContext,
        ContextChipKind::SvnBranch,
        ContextChipKind::SvnDirtyItems,
    ]);
    chips
}

/// Parses [`display_chip::GitLineChanges`] from the raw shell command output stored in a
/// [`GitDiffStats`](ContextChipKind::GitDiffStats) chip's value.
///
/// Used as a fallback when `GitRepoStatusModel` is unavailable (e.g. remote sessions,
/// local subshells).
pub fn git_line_changes_from_chips(chips: &[ChipResult]) -> Option<display_chip::GitLineChanges> {
    chips.iter().find_map(|chip| {
        if matches!(chip.kind(), ContextChipKind::GitDiffStats) {
            chip.value().map(|value| match value {
                // Structured data from GitRepoStatusModel — use directly.
                ChipValue::GitDiffStats(g) => g.clone(),
                // Raw shell command output (remote sessions) — parse.
                ChipValue::Text(raw) => display_chip::GitLineChanges::parse_from_git_output(raw)
                    .unwrap_or(display_chip::GitLineChanges {
                        files_changed: 0,
                        lines_added: 0,
                        lines_removed: 0,
                    }),
            })
        } else {
            None
        }
    })
}

/// Formats given context chips as an unstylized string.
/// Compared to displaying individual chips, certain chips when combined are displayed differently.
pub fn chips_to_string(chips: impl Iterator<Item = ChipResult>) -> String {
    let mut prompt = String::new();
    let mut visible_chips = chips
        .into_iter()
        .filter_map(|chip_result| Some((chip_result.kind, chip_result.value?)))
        .peekable();
    while let Some((chip_kind, current_value)) = visible_chips.next() {
        // This is temporary, until we design more generic chip formatting.
        let next_chip_kind = visible_chips.peek().map(|(next_kind, _)| next_kind);
        let chip_display_value = chip_kind.display_value(&current_value);
        prompt.push_str(&chip_display_value);
        match (chip_kind, next_chip_kind) {
            // Omit the space between adjacent Svn chips.
            (ContextChipKind::SvnBranch, Some(ContextChipKind::SvnDirtyItems)) => (),
            (_, Some(_)) => {
                // Add padding after non-empty chips.
                if !chip_display_value.is_empty() {
                    prompt.push(' ');
                }
            }
            _ => (),
        }
    }
    prompt
}

pub(crate) fn agent_view_chip_color(appearance: &Appearance) -> ColorU {
    let theme = appearance.theme();
    theme
        .sub_text_color(blended_colors::neutral_1(theme).into())
        .into_solid()
}

/// Helper function that adds specific styling to chips' text element.
/// Keeps chips in both editor and input in sync.
/// Keep in sync with [`ContextChipKind::display_value`]
pub fn render_text_from_kind(
    text: &mut Text,
    kind: ContextChipKind,
    value: String,
    is_in_agent_view: bool,
    appearance: &Appearance,
) {
    let styles = kind.default_styles(appearance, is_in_agent_view);
    let prompt_colors: PromptColors = appearance.theme().clone().into();

    // Keep in sync with `ContextChipKind::display_value`
    match kind {
        ContextChipKind::ShellGitBranch => {
            text.add_text_with_highlights(
                "git:(",
                if is_in_agent_view {
                    styles.value_color
                } else {
                    prompt_colors.input_prompt_git
                },
                styles.font_properties,
            );
        }
        ContextChipKind::SvnBranch => {
            text.add_text_with_highlights(
                "svn:(",
                if is_in_agent_view {
                    styles.value_color
                } else {
                    prompt_colors.input_prompt_svn
                },
                styles.font_properties,
            );
        }
        ContextChipKind::SvnDirtyItems => {
            text.add_text_with_highlights(
                "±",
                if is_in_agent_view {
                    styles.value_color
                } else {
                    prompt_colors.input_prompt_svn
                },
                styles.font_properties,
            );
        }
        ContextChipKind::KubernetesContext => {
            text.add_text_with_highlights(
                "⎈ ",
                if is_in_agent_view {
                    styles.value_color
                } else {
                    prompt_colors.input_prompt_kubernetes
                },
                if is_in_agent_view {
                    styles.font_properties
                } else {
                    Properties::default().weight(Weight::Thin)
                },
            );
        }
        _ => (),
    }

    text.add_text_with_highlights(value, styles.value_color, styles.font_properties);

    match kind {
        ContextChipKind::ShellGitBranch => {
            text.add_text_with_highlights(
                ")",
                if is_in_agent_view {
                    styles.value_color
                } else {
                    prompt_colors.input_prompt_git
                },
                styles.font_properties,
            );
        }
        ContextChipKind::SvnBranch => {
            text.add_text_with_highlights(
                ")",
                if is_in_agent_view {
                    styles.value_color
                } else {
                    prompt_colors.input_prompt_svn
                },
                styles.font_properties,
            );
        }
        _ => (),
    }
}
