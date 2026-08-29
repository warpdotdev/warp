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

use std::time::Duration;

use context_chip::PromptGenerator;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::PromptColors;
#[allow(unused_imports)]
pub use warp_terminal::context_chips::{
    ChipResult, ChipValue, ContextChipKind, GitBranchTrackingStatus, GitLineChanges,
    PromptSnapshot, RendererStyles, WarpPromptSeparator, agent_view_chip_color, chips_to_string,
    git_line_changes_from_chips, github_pr_display_text_from_url, github_pr_number_from_url,
    readable_chip_label_color,
};
use warpui::elements::Text;
use warpui::fonts::{Properties, Weight};

#[allow(unused_imports)]
pub use self::context_chip::{
    ChipAvailability, ChipDisabledReason, ChipRuntimeCapabilities, ExternalCommandsAvailability,
};
use self::context_chip::{ChipFingerprintInput, ChipRuntimePolicy, ContextChip, RefreshConfig};
use crate::features::FeatureFlag;

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

pub trait ContextChipKindAppExt {
    fn to_chip(&self) -> Option<ContextChip>;
    fn initial_value_generator(&self) -> Option<PromptGenerator>;
}

impl ContextChipKindAppExt for ContextChipKind {
    fn to_chip(&self) -> Option<ContextChip> {
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
            Self::GitBranchStatus => Some(ContextChip::shell_builtin(
                "Git Branch Status",
                builtins::shell_git_branch_status(),
                // Same branch list as ShellGitBranch, so clicking the chip
                // opens the same branch-switcher menu.
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
            Self::GithubPullRequest => Some(ContextChip::builtin(
                "GitHub Pull Request",
                |_| None,
                RefreshConfig::OnDemandOnly,
            )),
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

    /// Returns a generator to be used for the first fetch of
    /// a periodic generator. Is mostly used to use the PreCmd value
    /// of git-branch for ShellGitBranch, while using a shell command
    /// for the periodic updates.
    fn initial_value_generator(&self) -> Option<PromptGenerator> {
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
        ContextChipKind::GitBranchStatus,
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
        ContextChipKind::ShellGitBranch | ContextChipKind::GitBranchStatus => {
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
        ContextChipKind::ShellGitBranch | ContextChipKind::GitBranchStatus => {
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

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
