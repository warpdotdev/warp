use warpui::{ModelHandle, ViewContext, ViewHandle};

use super::{AgentInputFooter, AgentInputFooterEvent};
use crate::context_chips::display_chip::{DisplayChip, GitLineChanges, PromptDisplayChipEvent};
use crate::context_chips::prompt_type::PromptType;
use crate::context_chips::{git_line_changes_from_chips, ChipResult};

impl AgentInputFooter {
    /// Returns `true` if `DisplayChip`s should be recreated based on updated metadata values.
    ///
    /// This is basically cargo-culted from the equivalent logic in `PromptDisplay`, pared down to
    /// only the chip types that we care about in the AgentView input.
    fn check_if_chip_values_have_changed(
        existing_chips: &[ViewHandle<DisplayChip>],
        new_chips: &[ChipResult],
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        existing_chips.len() != new_chips.len()
            || new_chips.iter().enumerate().any(|(i, chip_result)| {
                let existing_chip = &existing_chips[i];
                existing_chip.read(ctx, |chip, _| {
                    chip.text()
                        != chip_result
                            .value()
                            .map(|v| v.to_string())
                            .unwrap_or_default()
                        || chip.chip_kind() != chip_result.kind()
                        || chip.on_click_values() != chip_result.on_click_values()
                })
            })
    }

    fn create_display_chips(
        &self,
        new_chips: &[ChipResult],
        git_line_changes_info: Option<GitLineChanges>,
        ctx: &mut ViewContext<Self>,
    ) -> Vec<ViewHandle<DisplayChip>> {
        let mut display_chips = Vec::with_capacity(new_chips.len());
        let mut new_chips = new_chips.iter().peekable();
        while let Some(chip_result) = new_chips.next() {
            let next_chip_kind = new_chips
                .peek()
                .map(|chip_result| chip_result.kind().clone());

            let view_handle = ctx.add_typed_action_view(|ctx| {
                let config = self.display_chip_config.clone();
                let mut chip = DisplayChip::new_for_agent_view(
                    chip_result.clone(),
                    next_chip_kind,
                    config,
                    ctx,
                );
                chip.maybe_set_git_line_changes_info(git_line_changes_info.clone());
                chip.update_session_context(self.display_chip_config.session_context.clone(), ctx);
                chip
            });

            ctx.subscribe_to_view(&view_handle, move |_, _, event, ctx| match event {
                PromptDisplayChipEvent::ToggleMenu { open } => {
                    ctx.emit(AgentInputFooterEvent::ToggledChipMenu { open: *open });
                    ctx.notify();
                }
                PromptDisplayChipEvent::TryExecuteCommand(cmd) => {
                    ctx.emit(AgentInputFooterEvent::TryExecuteChipCommand(cmd.clone()));
                    ctx.notify();
                }
                PromptDisplayChipEvent::OpenCodeReview => {
                    ctx.emit(AgentInputFooterEvent::OpenCodeReview);
                    ctx.notify();
                }
                PromptDisplayChipEvent::OpenAIDocument {
                    document_id,
                    document_version,
                } => {
                    ctx.emit(AgentInputFooterEvent::OpenAIDocument {
                        document_id: *document_id,
                        document_version: *document_version,
                    });
                }
                _ => {
                    ctx.notify();
                }
            });

            display_chips.push(view_handle);
        }

        display_chips
    }

    fn update_existing_display_chips(
        display_chips: &[ViewHandle<DisplayChip>],
        git_line_changes_info: Option<GitLineChanges>,
        ctx: &mut ViewContext<Self>,
    ) {
        for chip_view in display_chips {
            chip_view.update(ctx, |chip, ctx| {
                chip.maybe_set_git_line_changes_info(git_line_changes_info.clone());
                ctx.notify();
            });
        }
    }

    /// Updates the display chip views based on a change to the underlying metadata that drives the
    /// prompt, modeled in `PromptType`.
    ///
    /// This is basically cargo-culted from the equivalent logic in `PromptDisplay`, pared down to
    /// only the chip types that we care about in the AgentView input.
    ///
    /// The whole context chip/UDI chip/prompt layer is in need of a big refactor; once the
    /// `FeatureFlag::AgentView` is retired we'll have an opportunity to do a refactor with a smaller
    /// surface area, presumably because we'll be able to first delete a lot of the affected logic
    /// which the UDI and legacy inputs depend on.
    pub(super) fn update_display_chips(
        &mut self,
        model: &ModelHandle<PromptType>,
        ctx: &mut ViewContext<Self>,
    ) {
        let new_left_chips = model
            .as_ref(ctx)
            .agent_view_left_chips(ctx)
            .into_iter()
            .filter(|chip_result| chip_result.value().is_some())
            .collect::<Vec<ChipResult>>();
        let new_right_chips = model
            .as_ref(ctx)
            .agent_view_right_chips(ctx)
            .into_iter()
            .filter(|chip_result| chip_result.value().is_some())
            .collect::<Vec<ChipResult>>();
        let new_chips = model
            .as_ref(ctx)
            .agent_view_chips(ctx)
            .into_iter()
            .filter(|chip| chip.value().is_some())
            .collect::<Vec<ChipResult>>();
        let git_line_changes_info = git_line_changes_from_chips(&new_chips);

        let should_update_left =
            Self::check_if_chip_values_have_changed(&self.left_display_chips, &new_left_chips, ctx);
        let should_update_right = Self::check_if_chip_values_have_changed(
            &self.right_display_chips,
            &new_right_chips,
            ctx,
        );

        if should_update_left {
            self.left_display_chips =
                self.create_display_chips(&new_left_chips, git_line_changes_info.clone(), ctx);
        } else {
            Self::update_existing_display_chips(
                &self.left_display_chips,
                git_line_changes_info.clone(),
                ctx,
            );
        }

        if should_update_right {
            self.right_display_chips =
                self.create_display_chips(&new_right_chips, git_line_changes_info.clone(), ctx);
        } else {
            Self::update_existing_display_chips(
                &self.right_display_chips,
                git_line_changes_info.clone(),
                ctx,
            );
        }

        // Build display chips for the CLI agent footer separately.
        // The CLI selection may include chips not present in the agent view selection.
        let new_cli_chips = model
            .as_ref(ctx)
            .cli_agent_chips(ctx)
            .into_iter()
            .filter(|chip_result| chip_result.value().is_some())
            .collect::<Vec<ChipResult>>();
        let should_update_cli =
            Self::check_if_chip_values_have_changed(&self.cli_display_chips, &new_cli_chips, ctx);
        if should_update_cli {
            self.cli_display_chips =
                self.create_display_chips(&new_cli_chips, git_line_changes_info.clone(), ctx);
        } else {
            Self::update_existing_display_chips(
                &self.cli_display_chips,
                git_line_changes_info,
                ctx,
            );
        }

        ctx.notify();
    }
}
