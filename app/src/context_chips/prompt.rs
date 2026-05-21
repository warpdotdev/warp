use crate::{
    settings::{
        AISettings, AISettingsChangedEvent, InputSettings, InputSettingsChangedEvent,
        WarpPromptSeparator,
    },
    terminal::session_settings::{SessionSettings, SessionSettingsChangedEvent},
};

pub use super::ContextChipKind;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use settings::Setting as _;
use warpui::{Entity, GetSingletonModelHandle, ModelContext, SingletonEntity, UpdateModel};

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;

#[derive(
    Clone,
    Debug,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "Configuration for a prompt chip.")]
pub struct ChipConfig {
    // TODO in the future
}

/// PromptChip holds the configuration of the specific chip in the prompt that the user set.
#[derive(
    Clone,
    Debug,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "A chip in the prompt with its configuration.")]
pub struct PromptChip {
    #[schemars(description = "The type of context chip.")]
    chip: ContextChipKind,
    #[schemars(description = "Configuration options for this chip.")]
    config: ChipConfig,
}

impl PromptChip {
    fn new(chip: ContextChipKind, config: ChipConfig) -> Self {
        Self { chip, config }
    }

    pub fn chip(&self) -> &ContextChipKind {
        &self.chip
    }
}

/// Deserialize prompt chips, silently dropping any with unrecognized chip kinds.
/// This ensures saved prompt configs remain intact when chip kinds are removed.
fn deserialize_prompt_chips<'de, D>(deserializer: D) -> Result<Vec<PromptChip>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values: Vec<serde_json::Value> = serde::Deserialize::deserialize(deserializer)?;
    Ok(values
        .into_iter()
        .filter_map(|value| serde_json::from_value::<PromptChip>(value).ok())
        .collect())
}

/// Serializable configuration for the current prompt.
#[derive(
    Clone,
    Debug,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "Prompt layout configuration.")]
pub struct PromptConfiguration {
    #[serde(default, deserialize_with = "deserialize_prompt_chips")]
    #[schemars(description = "Ordered list of chips to display in the prompt.")]
    chips: Vec<PromptChip>,
    #[serde(default)]
    #[schemars(description = "Whether git diff stats have been separated into their own chip.")]
    did_separate_git_diff_stats: bool,

    #[schemars(description = "Whether the prompt is displayed on the same line as the input.")]
    same_line_prompt_enabled: bool,
    /// The separator to use as a trailing character at the end of Warp prompt, if any.
    #[schemars(description = "Trailing separator character for the prompt.")]
    separator: WarpPromptSeparator,
}

#[derive(
    Clone,
    Debug,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Whether using the default or a custom prompt layout.",
    rename_all = "snake_case"
)]
pub enum PromptSelection {
    #[default]
    #[schemars(description = "Use the default prompt.")]
    Default,
    #[schemars(description = "Use a custom prompt chip selection.")]
    CustomChipSelection(PromptConfiguration),
}

impl From<PromptConfiguration> for PromptSelection {
    fn from(config: PromptConfiguration) -> Self {
        Self::CustomChipSelection(config)
    }
}

impl PromptSelection {
    pub fn same_line_prompt_enabled(&self) -> bool {
        match self {
            PromptSelection::Default => false,
            PromptSelection::CustomChipSelection(config) => config.same_line_prompt_enabled(),
        }
    }

    pub fn separator(&self) -> WarpPromptSeparator {
        match self {
            PromptSelection::Default => WarpPromptSeparator::None,
            PromptSelection::CustomChipSelection(config) => config.separator(),
        }
    }
}

/// Prompt is the singleton entity that stores the selected prompt configuration.
pub struct Prompt {
    config: PromptConfiguration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptEvent {
    /// The prompt configuration changed.
    Changed,
}

impl Prompt {
    /// Creates a global singleton [`Prompt`] that responds to settings changes.
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let session_settings = SessionSettings::handle(ctx);
        ctx.subscribe_to_model(&session_settings, Self::handle_session_settings_change);
        let ai_settings = AISettings::handle(ctx);
        ctx.subscribe_to_model(&ai_settings, Self::handle_ai_settings_change);
        let input_settings = InputSettings::handle(ctx);
        ctx.subscribe_to_model(&input_settings, Self::handle_input_settings_change);

        let initial_config = Self::from_user_settings(ctx);
        Self {
            config: initial_config,
        }
    }

    pub fn update<I>(
        &mut self,
        chips: I,
        same_line_prompt_enabled: bool,
        separator: WarpPromptSeparator,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()>
    where
        I: IntoIterator<Item = ContextChipKind>,
    {
        let config = PromptConfiguration::from_chips(chips, same_line_prompt_enabled, separator);
        // Eagerly set the new config - it will be re-updated when the settings change propagates.
        self.config = config.clone();
        SessionSettings::handle(ctx).update(ctx, |session_settings, ctx| {
            session_settings.honor_ps1.set_value(false, ctx)?;
            session_settings
                .saved_prompt
                .set_value(PromptSelection::CustomChipSelection(config), ctx)?;
            Ok(())
        })
    }

    /// Reset to the default Warp prompt.
    pub fn reset<C: UpdateModel + GetSingletonModelHandle>(
        &mut self,
        ctx: &mut C,
    ) -> anyhow::Result<()> {
        // Note that because settings are being updated, `handle_session_settings_change` will be called
        // and will set the new prompt value
        let session_settings = SessionSettings::handle(ctx);
        session_settings.update(ctx, |session_settings, ctx| {
            session_settings
                .saved_prompt
                .set_value(PromptSelection::Default, ctx)?;
            session_settings.honor_ps1.set_value(false, ctx)?;
            Ok(())
        })
    }

    /// Builds the [`PromptConfiguration`] from current user settings.
    fn from_user_settings(ctx: &mut ModelContext<Self>) -> PromptConfiguration {
        let session_settings = SessionSettings::handle(ctx);
        let settings = session_settings.as_ref(ctx);
        match settings.saved_prompt.clone() {
            PromptSelection::Default => {
                let suppress_pr = settings.github_pr_chip_default_validation.is_suppressed();
                PromptConfiguration::default_prompt_with_pr_chip_suppressed(suppress_pr)
            }
            PromptSelection::CustomChipSelection(config) => config.normalize_custom_prompt_config(),
        }
    }

    /// Mock an empty prompt.
    #[cfg(test)]
    pub fn mock() -> Self {
        Self {
            config: Default::default(),
        }
    }

    /// Mock a prompt with the given chips.
    #[cfg(test)]
    pub fn mock_with(
        chips: impl IntoIterator<Item = ContextChipKind>,
        same_line_prompt_enabled: bool,
        separator: WarpPromptSeparator,
    ) -> Self {
        Self {
            config: PromptConfiguration::from_chips(chips, same_line_prompt_enabled, separator),
        }
    }

    /// Whether same line prompt is enabled for Warp prompt.
    pub fn same_line_prompt_enabled(&self) -> bool {
        self.config.same_line_prompt_enabled
    }

    /// The separator to be used for the Warp prompt.
    pub fn separator(&self) -> WarpPromptSeparator {
        self.config.separator
    }

    /// The chips included in the prompt, in order from left to right.
    pub fn chip_kinds(&self) -> Vec<ContextChipKind> {
        self.config.chip_kinds()
    }

    /// Updates the in-memory prompt configuration to reflect a settings change.
    fn handle_session_settings_change(
        &mut self,
        event: &SessionSettingsChangedEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if matches!(
            event,
            SessionSettingsChangedEvent::SavedPrompt { .. }
                | SessionSettingsChangedEvent::GithubPrChipDefaultValidation { .. }
        ) {
            log::debug!("Loading new prompt configuration");
            self.config = Self::from_user_settings(ctx);
            ctx.emit(PromptEvent::Changed);
        }
    }

    fn handle_input_settings_change(
        &mut self,
        event: &InputSettingsChangedEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if matches!(event, InputSettingsChangedEvent::InputBoxTypeSetting { .. }) {
            self.config = Self::from_user_settings(ctx);
            ctx.emit(PromptEvent::Changed);
        }
    }

    /// Updates the in-memory prompt configuration to reflect an AI settings change.
    fn handle_ai_settings_change(
        &mut self,
        event: &AISettingsChangedEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if let AISettingsChangedEvent::IsAnyAIEnabled { .. } = event {
            log::debug!("Loading new prompt configuration");
            self.config = Self::from_user_settings(ctx);
            ctx.emit(PromptEvent::Changed);
        }
    }

    /// Updates the in-memory prompt configuration to reflect an AI input model change.
    fn handle_ai_input_model_change(&mut self, ctx: &mut ModelContext<Self>) {
        log::debug!("Loading new prompt configuration due to AI input model change");
        self.config = Self::from_user_settings(ctx);
        ctx.emit(PromptEvent::Changed);
    }
}

impl Entity for Prompt {
    type Event = PromptEvent;
}

impl SingletonEntity for Prompt {}

impl PromptConfiguration {
    /// The default Warp prompt, synthesized from legacy prompt settings.
    /// The order of chips is important and would affect a lot of users if rearranged.
    pub fn default_prompt() -> Self {
        Self::default_prompt_with_pr_chip_suppressed(false)
    }

    pub fn default_prompt_with_pr_chip_suppressed(suppress_pr_chip: bool) -> Self {
        use crate::features::FeatureFlag;

        let mut chips = vec![
            ContextChipKind::CondaEnvironment,
            ContextChipKind::VirtualEnvironment,
            ContextChipKind::Ssh,
            ContextChipKind::Subshell,
            ContextChipKind::NodeVersion,
            ContextChipKind::WorkingDirectory,
            ContextChipKind::ShellGitBranch,
            ContextChipKind::GitDiffStats,
            ContextChipKind::KubernetesContext,
        ];
        if FeatureFlag::GithubPrPromptChip.is_enabled() && !suppress_pr_chip {
            chips.push(ContextChipKind::GithubPullRequest);
        }

        Self::from_chips(chips, false, WarpPromptSeparator::None)
    }

    pub fn from_chips(
        chips: impl IntoIterator<Item = ContextChipKind>,
        same_line_prompt_enabled: bool,
        separator: WarpPromptSeparator,
    ) -> Self {
        Self {
            chips: chips
                .into_iter()
                .map(|chip| PromptChip::new(chip, Default::default()))
                .collect(),
            did_separate_git_diff_stats: true,
            same_line_prompt_enabled,
            separator,
        }
    }

    pub fn chip_kinds(&self) -> Vec<ContextChipKind> {
        self.chips
            .iter()
            .map(|chip| chip.chip.clone())
            .collect_vec()
    }

    /// Normalizes custom prompt configs after deserialization.
    ///
    /// `ShellGitBranch` originally rendered both the branch selector and git diff stats.
    /// To preserve the previous behavior, we insert `GitDiffStats` immediately after `ShellGitBranch`.
    ///
    /// This is gated by `did_separate_git_diff_stats` so we do not re-insert `GitDiffStats` after a
    /// user intentionally removes it and saves their custom prompt.
    fn normalize_custom_prompt_config(mut self) -> Self {
        if !self.did_separate_git_diff_stats {
            let already_has_git_diff_stats = self
                .chips
                .iter()
                .any(|chip| chip.chip == ContextChipKind::GitDiffStats);
            if !already_has_git_diff_stats {
                let shell_git_branch_index = self
                    .chips
                    .iter()
                    .position(|chip| chip.chip == ContextChipKind::ShellGitBranch);
                if let Some(index) = shell_git_branch_index {
                    self.chips.insert(
                        index + 1,
                        PromptChip::new(ContextChipKind::GitDiffStats, Default::default()),
                    );
                }
            }
            self.did_separate_git_diff_stats = true;
        }
        self
    }

    pub fn same_line_prompt_enabled(&self) -> bool {
        self.same_line_prompt_enabled
    }

    pub fn separator(&self) -> WarpPromptSeparator {
        self.separator
    }

    pub fn remove_chip(&mut self, chip: ContextChipKind) {
        self.chips.retain(|c| c.chip != chip);
    }

    pub fn add_chip_to_end(&mut self, chip: ContextChipKind) {
        self.chips.push(PromptChip::new(chip, Default::default()));
    }
}
