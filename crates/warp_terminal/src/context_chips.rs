use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::color::contrast::{MinimumAllowedContrast, high_enough_contrast};
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::{Fill, PromptColors, WarpTheme};
use warpui_core::color::ColorU;
use warpui_core::fonts::{Properties, Weight};
#[derive(Clone)]
pub struct RendererStyles {
    pub value_color: ColorU,
    pub font_properties: Properties,
}

impl RendererStyles {
    pub fn new(value_color: ColorU, font_properties: Properties) -> Self {
        Self {
            value_color,
            font_properties,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitBranchTrackingStatus {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub counts_available: bool,
    #[serde(default)]
    pub rebased: bool,
}

impl GitBranchTrackingStatus {
    pub fn new(branch: String, upstream: Option<String>, ahead: u32, behind: u32) -> Self {
        let counts_available = upstream.is_some();
        Self {
            branch,
            upstream,
            ahead,
            behind,
            counts_available,
            rebased: false,
        }
    }

    pub fn without_counts(branch: String, upstream: Option<String>) -> Self {
        Self {
            branch,
            upstream,
            ahead: 0,
            behind: 0,
            counts_available: false,
            rebased: false,
        }
    }

    pub fn rebased(branch: String, upstream: String) -> Self {
        Self {
            branch,
            upstream: Some(upstream),
            ahead: 0,
            behind: 0,
            counts_available: true,
            rebased: true,
        }
    }

    pub fn from_display_text(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }

        let Some((branch, status_text)) = text.rsplit_once(" • ") else {
            return Some(Self {
                branch: text.to_string(),
                upstream: None,
                ahead: 0,
                behind: 0,
                counts_available: false,
                rebased: false,
            });
        };

        let Some((ahead, behind, rebased)) = Self::parse_display_status(status_text) else {
            return Some(Self {
                branch: text.to_string(),
                upstream: None,
                ahead: 0,
                behind: 0,
                counts_available: false,
                rebased: false,
            });
        };

        let branch = branch.trim();
        if branch.is_empty() {
            return None;
        }

        Some(Self {
            branch: branch.to_string(),
            upstream: None,
            ahead,
            behind,
            counts_available: true,
            rebased,
        })
    }

    pub fn status_text(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.is_rebased() {
            parts.push("⇅".to_string());
        } else {
            if let Some(ahead) = self.ahead_display_count() {
                parts.push(format!("↑{ahead}"));
            }
            if let Some(behind) = self.behind_display_count() {
                parts.push(format!("↓{behind}"));
            }
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    pub fn display_text(&self) -> String {
        match self.status_text() {
            Some(status) => format!("{} • {status}", self.branch),
            None => self.branch.clone(),
        }
    }

    pub fn is_rebased(&self) -> bool {
        self.counts_available && self.rebased
    }

    pub fn ahead_display_count(&self) -> Option<String> {
        (!self.is_rebased() && self.counts_available && self.ahead > 0)
            .then(|| Self::format_display_count(self.ahead))
    }

    pub fn behind_display_count(&self) -> Option<String> {
        (!self.is_rebased() && self.counts_available && self.behind > 0)
            .then(|| Self::format_display_count(self.behind))
    }

    fn format_display_count(count: u32) -> String {
        const MAX_DISPLAY_COUNT: u32 = 999;
        if count > MAX_DISPLAY_COUNT {
            format!("{MAX_DISPLAY_COUNT}+")
        } else {
            count.to_string()
        }
    }

    fn parse_display_count(count: &str) -> Option<u32> {
        if let Some(capped_count) = count.strip_suffix('+') {
            capped_count.parse::<u32>().ok()?.checked_add(1)
        } else {
            count.parse::<u32>().ok()
        }
    }

    fn parse_display_status(status_text: &str) -> Option<(u32, u32, bool)> {
        let mut ahead = 0;
        let mut behind = 0;
        let mut rebased = false;
        let mut saw_status_token = false;

        for part in status_text.split_whitespace() {
            saw_status_token = true;
            if part == "⇅" {
                rebased = true;
            } else if let Some(ahead_count) = part.strip_prefix('↑') {
                ahead = Self::parse_display_count(ahead_count)?;
            } else if let Some(behind_count) = part.strip_prefix('↓') {
                behind = Self::parse_display_count(behind_count)?;
            } else {
                return None;
            }
        }

        saw_status_token.then_some((ahead, behind, rebased))
    }

    pub fn tooltip_text(&self) -> String {
        match &self.upstream {
            Some(upstream) if self.is_rebased() => {
                format!("Tracking {upstream} • branch was rebased")
            }
            Some(upstream) if self.counts_available => format!(
                "Tracking {upstream} • ahead {}, behind {}",
                self.ahead, self.behind
            ),
            Some(upstream) => {
                format!("Tracking {upstream}; ahead/behind counts are unavailable")
            }
            None if self.is_rebased() => {
                "Branch was rebased; upstream name is unavailable".to_string()
            }
            None if self.counts_available => format!(
                "Ahead {}, behind {}; upstream name is unavailable",
                self.ahead, self.behind
            ),
            None => "No upstream configured".to_string(),
        }
    }
}

impl std::fmt::Display for GitBranchTrackingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display_text())
    }
}

impl GitLineChanges {
    /// Parse git diff --shortstat output into GitLineChanges struct
    /// Input example: " 1 file changed, 2 insertions(+), 17 deletions(-)"
    pub fn parse_from_git_output(raw_output: &str) -> Option<Self> {
        let line = raw_output.trim();

        if line.is_empty() {
            return None;
        }

        let mut files_changed = 0;
        let mut lines_added = 0;
        let mut lines_removed = 0;

        let words: Vec<&str> = line.split_whitespace().collect();
        for (i, word) in words.iter().enumerate() {
            if let Ok(num) = word.parse::<u32>()
                && let Some(next_word) = words.get(i + 1)
            {
                if next_word.starts_with("file") {
                    files_changed = num;
                } else if next_word.starts_with("insertion") {
                    lines_added = num;
                } else if next_word.starts_with("deletion") {
                    lines_removed = num;
                }
            }
        }

        Some(Self {
            files_changed,
            lines_added,
            lines_removed,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitLineChanges {
    pub files_changed: u32,
    pub lines_added: u32,
    pub lines_removed: u32,
}

pub fn agent_view_chip_color(appearance: &Appearance) -> ColorU {
    let theme = appearance.theme();
    readable_chip_label_color(theme, Fill::Solid(internal_colors::neutral_1(theme)))
}

/// The label/icon color for a chip drawn on `background`.
///
/// Chips normally use the muted `sub_text_color` (which is `font_color` at 60%
/// opacity). Because the contrast machinery is alpha-blind, that muted color can
/// composite to a faint mid-grey that drops below WCAG AA on light themes,
/// making chip labels hard to read. So we keep the muted look wherever it is
/// still legible (e.g. dark themes) and only fall back to the fully-opaque,
/// contrast-enforced `font_color` where the muted color would be sub-AA.
pub fn readable_chip_label_color(theme: &WarpTheme, background: Fill) -> ColorU {
    let muted = theme.sub_text_color(background).into_solid();
    let solid_background = background.into_solid();
    if high_enough_contrast(
        solid_background.blend(&muted),
        solid_background,
        MinimumAllowedContrast::Text,
    ) {
        muted
    } else {
        theme.font_color(background).into_solid()
    }
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
/// Parses [`GitLineChanges`] from the raw shell command output stored in a
/// [`GitDiffStats`](ContextChipKind::GitDiffStats) chip's value.
///
/// Used as a fallback when `GitRepoStatusModel` is unavailable (e.g. remote sessions,
/// local subshells).
pub fn git_line_changes_from_chips(chips: &[ChipResult]) -> Option<GitLineChanges> {
    chips.iter().find_map(|chip| {
        if matches!(chip.kind(), ContextChipKind::GitDiffStats) {
            chip.value().map(|value| match value {
                // Structured data from GitRepoStatusModel — use directly.
                ChipValue::GitDiffStats(g) => g.clone(),
                // Raw shell command output (remote sessions) — parse.
                ChipValue::Text(raw) => {
                    GitLineChanges::parse_from_git_output(raw).unwrap_or(GitLineChanges {
                        files_changed: 0,
                        lines_added: 0,
                        lines_removed: 0,
                    })
                }
                ChipValue::GitBranchStatus(_) => GitLineChanges {
                    files_changed: 0,
                    lines_added: 0,
                    lines_removed: 0,
                },
            })
        } else {
            None
        }
    })
}

impl ContextChipKind {
    /// TODO: we might need to move this API to support custom chips.
    pub fn placeholder_value(&self) -> ChipValue {
        match self {
            Self::WorkingDirectory => ChipValue::Text("~/Desktop".to_string()),
            Self::Username => ChipValue::Text("alice".to_string()),
            Self::Hostname => ChipValue::Text("ubuntu-04".to_string()),
            Self::ShellGitBranch => ChipValue::Text("git-feature-branch".to_string()),
            Self::GitBranchStatus => ChipValue::GitBranchStatus(GitBranchTrackingStatus::new(
                "main".to_string(),
                Some("origin/main".to_string()),
                1,
                2,
            )),
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
            Self::GitBranchStatus => prompt_colors.input_prompt_branch,
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
            Self::ShellGitBranch | Self::GitBranchStatus => format!("git:({text})"),
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
            Self::ShellGitBranch | Self::GitBranchStatus | Self::SvnBranch => Some(Icon::GitBranch),
            Self::GitDiffStats | Self::SvnDirtyItems => Some(Icon::File),
            Self::GithubPullRequest => Some(Icon::Github),
            Self::KubernetesContext => Some(Icon::Globe),
            Self::AgentPlanAndTodoList => Some(Icon::CheckSkinny),
            Self::Custom { .. } => None,
        }
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
    GitBranchStatus,
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChipResult {
    kind: ContextChipKind,
    value: Option<ChipValue>,
    on_click_values: Vec<String>,
}

impl ChipResult {
    pub fn new(
        kind: ContextChipKind,
        value: Option<ChipValue>,
        on_click_values: Vec<String>,
    ) -> Self {
        Self {
            kind,
            value,
            on_click_values,
        }
    }
    pub fn kind(&self) -> &ContextChipKind {
        &self.kind
    }

    pub fn value(&self) -> Option<&ChipValue> {
        self.value.as_ref()
    }

    pub fn on_click_values(&self) -> &[String] {
        &self.on_click_values
    }

    pub fn into_parts(self) -> (ContextChipKind, Option<ChipValue>, Vec<String>) {
        (self.kind, self.value, self.on_click_values)
    }
}
/// The value of a context chip. Most chips produce plain text, but some
/// (like `GitDiffStats`) carry structured data to avoid string round-trips.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChipValue {
    Text(String),
    GitDiffStats(GitLineChanges),
    GitBranchStatus(GitBranchTrackingStatus),
}

impl ChipValue {
    /// Returns the text representation, or `None` for non-text variants.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ChipValue::Text(s) => Some(s),
            ChipValue::GitDiffStats(_) | ChipValue::GitBranchStatus(_) => None,
        }
    }

    /// Returns the `GitLineChanges` payload, if this is the `GitDiffStats` variant.
    pub fn as_git_diff_stats(&self) -> Option<&GitLineChanges> {
        match self {
            ChipValue::GitDiffStats(g) => Some(g),
            ChipValue::Text(_) | ChipValue::GitBranchStatus(_) => None,
        }
    }

    pub fn as_git_branch_tracking_status(&self) -> Option<&GitBranchTrackingStatus> {
        match self {
            ChipValue::GitBranchStatus(status) => Some(status),
            ChipValue::Text(_) | ChipValue::GitDiffStats(_) => None,
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
            ChipValue::GitBranchStatus(status) => f.write_str(&status.display_text()),
        }
    }
}

impl From<String> for ChipValue {
    fn from(s: String) -> Self {
        ChipValue::Text(s)
    }
}

pub fn github_pr_number_from_url(url: &str) -> Option<i32> {
    let (_, tail) = url.trim().rsplit_once("/pull/")?;
    let number = tail.split(['/', '?', '#']).next()?;
    parse_github_pr_number(number)
}

fn parse_github_pr_number(number: &str) -> Option<i32> {
    if !number.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    number.parse::<i32>().ok().filter(|number| *number > 0)
}

pub fn github_pr_display_text_from_url(url: &str) -> Option<String> {
    github_pr_number_from_url(url).map(|number| format!("PR #{number}"))
}
/// This enum is used to enforce options in the dropdown for selecting a separator with the Warp prompt.
/// Note that these separators are added at the END of the Warp prompt (used in the case of same line prompt).
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Deserialize,
    Serialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Trailing separator character displayed at the end of the prompt.",
    rename_all = "snake_case"
)]
pub enum WarpPromptSeparator {
    /// No separator for the prompt.
    #[default]
    None,
    /// "%" separator for the prompt. Note this is the default separator used in zsh traditionally.
    PercentSign,
    /// "$" separator for the prompt. Note this is the default separator used in bash traditionally.
    DollarSign,
    /// ">" separator for the prompt. Note this is the default separator used in fish traditionally.
    ChevronSymbol,
}

impl WarpPromptSeparator {
    pub fn dropdown_item_label(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::PercentSign => "%",
            Self::DollarSign => "$",
            Self::ChevronSymbol => ">",
        }
    }

    pub fn renderable_string(&self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::PercentSign => Some("%"),
            Self::DollarSign => Some("$"),
            Self::ChevronSymbol => Some(">"),
        }
    }
}

impl PromptSnapshot {
    pub fn from_chips(
        chips: Vec<ChipResult>,
        same_line_prompt_enabled: bool,
        separator: WarpPromptSeparator,
    ) -> Self {
        Self {
            chips,
            same_line_prompt_enabled,
            separator,
        }
    }

    /// The value of the given chip, in this snapshot.
    pub fn chip_value(&self, chip: &ContextChipKind) -> Option<ChipValue> {
        self.chips.iter().find_map(|chip_result| {
            if chip_result.kind == *chip {
                chip_result.value.clone()
            } else {
                None
            }
        })
    }

    pub fn chips(&self) -> &Vec<ChipResult> {
        &self.chips
    }

    pub fn same_line_prompt_enabled(&self) -> bool {
        self.same_line_prompt_enabled
    }

    pub fn separator(&self) -> WarpPromptSeparator {
        self.separator
    }
}

impl std::fmt::Display for PromptSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&chips_to_string(self.chips.clone().into_iter()))
    }
}
/// Struct that holds a point in time snapshot of a prompt (chips are no longer interactive)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptSnapshot {
    chips: Vec<ChipResult>,

    same_line_prompt_enabled: bool,
    /// The separator to use as a trailing character at the end of Warp prompt, if any.
    separator: WarpPromptSeparator,
}

impl ContextChipKind {
    /// Whether the context chip has a copyable value.
    pub fn is_copyable(&self) -> bool {
        !matches!(self, Self::AgentPlanAndTodoList)
    }
}
