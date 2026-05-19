use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use crate::{
    menu::{MenuItem, MenuItemFields},
    settings::WarpPromptSeparator,
    terminal::{
        model::session::Sessions,
        session_settings::{SessionSettings, ToolbarChipSelection},
        view::{ContextMenuAction, PromptPart, PromptPosition, TerminalAction},
    },
};

use super::{
    current_prompt::CurrentPrompt, prompt_snapshot::PromptSnapshot, ChipResult, ChipValue,
    ContextChipKind,
};

/// The type of warp prompt being used
#[derive(Clone)]
pub enum PromptType {
    /// A warp prompt that refreshes chip values on its own. Typical for local sessions.
    Dynamic { prompt: ModelHandle<CurrentPrompt> },
    /// A warp prompt that does not change unless explicitly overwritten. Used for viewers of shared sessions.
    Static { snapshot: PromptSnapshot },
}

impl PromptType {
    pub fn new_dynamic_from_sessions(
        sessions: ModelHandle<Sessions>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let current_prompt = ctx.add_model(|ctx| CurrentPrompt::new(sessions, ctx));
        Self::new_dynamic(current_prompt, ctx)
    }

    pub fn new_dynamic(
        current_prompt: ModelHandle<CurrentPrompt>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.observe(&current_prompt, |_, _, ctx| ctx.notify());
        Self::Dynamic {
            prompt: current_prompt,
        }
    }

    pub fn new_static(
        chips: Vec<ChipResult>,
        same_line_prompt_enabled: bool,
        separator: WarpPromptSeparator,
    ) -> Self {
        PromptType::Static {
            snapshot: PromptSnapshot::from_chips(chips, same_line_prompt_enabled, separator),
        }
    }

    /// Returns menu items for copying parts of the prompt given a prompt snapshot.
    pub fn copy_menu_items(
        &self,
        position: PromptPosition,
        ctx: &AppContext,
    ) -> Vec<MenuItem<TerminalAction>> {
        self.chips(ctx)
            .into_iter()
            .filter_map(|chip_result| {
                if chip_result.value.is_some() && chip_result.kind.is_copyable() {
                    if let Some(chip) = chip_result.kind.to_chip() {
                        Some(
                            MenuItemFields::new(format!("Copy {}", chip.title()))
                                .with_on_select_action(TerminalAction::ContextMenu(
                                    ContextMenuAction::CopyPrompt {
                                        position,
                                        part: PromptPart::ContextChip(chip_result.kind),
                                    },
                                ))
                                .into_item(),
                        )
                    } else {
                        log::error!("Missing definition for chip: {:?}", chip_result.kind);
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn latest_chip_value(
        &self,
        chip_kind: &ContextChipKind,
        ctx: &AppContext,
    ) -> Option<ChipValue> {
        match self {
            Self::Dynamic { prompt } => prompt.as_ref(ctx).latest_chip_value(chip_kind).cloned(),
            Self::Static { snapshot } => snapshot.chip_value(chip_kind),
        }
    }

    pub fn apply_github_pull_request_artifact(
        &mut self,
        url: &str,
        branch: &str,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let url = url.trim();
        if url.is_empty() || ContextChipKind::GithubPullRequest.to_chip().is_none() {
            return false;
        }

        match self {
            Self::Dynamic { prompt } => prompt.update(ctx, |prompt, _| {
                prompt.apply_github_pull_request_artifact(url, branch)
            }),
            Self::Static { snapshot } => {
                let artifact_branch = branch.trim();
                let current_branch = snapshot
                    .chip_value(&ContextChipKind::ShellGitBranch)
                    .and_then(|value| value.as_text().map(str::to_string))
                    .map(|branch| branch.trim().to_string())
                    .filter(|branch| !branch.is_empty());

                if current_branch.as_deref().is_some_and(|current_branch| {
                    !artifact_branch.is_empty() && current_branch != artifact_branch
                }) {
                    return false;
                }

                if current_branch.is_none() && !artifact_branch.is_empty() {
                    snapshot.set_chip_value(
                        ContextChipKind::ShellGitBranch,
                        Some(ChipValue::Text(artifact_branch.to_string())),
                    );
                }
                snapshot.set_chip_value(
                    ContextChipKind::GithubPullRequest,
                    Some(ChipValue::Text(url.to_string())),
                );
                ctx.notify();
                true
            }
        }
    }

    pub fn prompt_as_string(&self, ctx: &AppContext) -> String {
        match self {
            Self::Dynamic { prompt } => prompt.as_ref(ctx).prompt_as_string(ctx),
            Self::Static { snapshot } => snapshot.to_string(),
        }
    }

    pub fn snapshot(&self, ctx: &AppContext) -> PromptSnapshot {
        match self {
            Self::Dynamic { prompt } => {
                PromptSnapshot::from_current_prompt(prompt.as_ref(ctx), ctx)
            }
            Self::Static { snapshot } => snapshot.clone(),
        }
    }

    pub fn chips(&self, ctx: &AppContext) -> Vec<ChipResult> {
        self.snapshot(ctx).chips().clone()
    }

    pub fn agent_view_chips(&self, ctx: &AppContext) -> Vec<ChipResult> {
        let chip_kinds = SessionSettings::as_ref(ctx)
            .agent_footer_chip_selection
            .all_chips();
        self.resolve_chip_kinds(chip_kinds, ctx)
    }

    pub fn agent_view_left_chips(&self, ctx: &AppContext) -> Vec<ChipResult> {
        let chip_kinds = SessionSettings::as_ref(ctx)
            .agent_footer_chip_selection
            .left_chips();
        self.resolve_chip_kinds(chip_kinds, ctx)
    }

    pub fn agent_view_right_chips(&self, ctx: &AppContext) -> Vec<ChipResult> {
        let chip_kinds = SessionSettings::as_ref(ctx)
            .agent_footer_chip_selection
            .right_chips();
        self.resolve_chip_kinds(chip_kinds, ctx)
    }

    pub fn cli_agent_chips(&self, ctx: &AppContext) -> Vec<ChipResult> {
        let chip_kinds = SessionSettings::as_ref(ctx)
            .cli_agent_footer_chip_selection
            .all_chips();
        self.resolve_chip_kinds(chip_kinds, ctx)
    }

    fn resolve_chip_kinds(
        &self,
        chip_kinds: Vec<ContextChipKind>,
        ctx: &AppContext,
    ) -> Vec<ChipResult> {
        chip_kinds
            .into_iter()
            .filter_map(|chip_kind| match self {
                Self::Dynamic { prompt } => prompt.as_ref(ctx).latest_chip_result(&chip_kind),
                Self::Static { snapshot } => snapshot
                    .chips()
                    .iter()
                    .find(|chip_result| chip_result.kind() == &chip_kind)
                    .cloned(),
            })
            .collect()
    }

    /// Whether same line prompt is enabled for the Warp Prompt.
    pub fn same_line_prompt_enabled(&self, ctx: &AppContext) -> bool {
        match self {
            Self::Dynamic { prompt } => prompt.as_ref(ctx).same_line_prompt_enabled(),
            Self::Static { snapshot } => snapshot.same_line_prompt_enabled(),
        }
    }

    /// The separator for the Warp prompt.
    pub fn separator(&self, ctx: &AppContext) -> WarpPromptSeparator {
        match self {
            Self::Dynamic { prompt } => prompt.as_ref(ctx).separator(),
            Self::Static { snapshot } => snapshot.separator(),
        }
    }
}

impl Entity for PromptType {
    type Event = ();
}
