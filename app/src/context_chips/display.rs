use std::path::PathBuf;
use std::sync::Arc;

use warp_core::features::FeatureFlag;
use warpui::elements::{
    ChildView, Clipped, Container, CrossAxisAlignment, Element, Flex, MainAxisAlignment,
    MainAxisSize, ParentElement, Wrap,
};
use warpui::{
    AppContext, Entity, EntityId, FocusContext, ModelHandle, SingletonEntity, TypedActionView,
    View, ViewContext, ViewHandle,
};

use super::display_chip::{DisplayChip, DisplayChipConfig, PromptDisplayChipEvent};
use super::prompt_type::PromptType;
use super::{git_line_changes_from_chips, ChipResult, ContextChipKind};
use crate::ai::blocklist::agent_view::AgentViewController;
use crate::ai::blocklist::{
    BlocklistAIContextModel, BlocklistAIHistoryEvent, BlocklistAIHistoryModel,
    BlocklistAIInputEvent, BlocklistAIInputModel,
};
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentVersion};
use crate::completer::SessionContext;
use crate::context_chips::display_chip::{DisplayChipAction, PromptChipShellCommand};
use crate::settings::InputSettings;
use crate::terminal::input::MenuPositioningProvider;
use crate::terminal::model_events::ModelEventDispatcher;

/// Enum introduced to abstract over the different row types we use for the prompt display,
/// between the non-UDI and UDI cases.
enum RowBuilder {
    Wrap(Wrap),
    Flex(Flex),
}

impl RowBuilder {
    fn add_child(&mut self, child: Box<dyn Element>) {
        match self {
            RowBuilder::Wrap(w) => w.add_child(child),
            RowBuilder::Flex(f) => f.add_child(child),
        }
    }

    fn finish(self) -> Box<dyn Element> {
        match self {
            RowBuilder::Wrap(w) => w.finish(),
            RowBuilder::Flex(f) => f.finish(),
        }
    }
}

/// A view for displaying the prompt.
pub struct PromptDisplay {
    prompt: ModelHandle<PromptType>,
    display_chips: Vec<ViewHandle<DisplayChip>>,
    ai_input_model: ModelHandle<BlocklistAIInputModel>,
    ai_context_model: ModelHandle<BlocklistAIContextModel>,
    terminal_view_id: EntityId,
    menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
    session_context: Option<SessionContext>,
    current_repo_path: Option<PathBuf>,
    model_events: ModelHandle<ModelEventDispatcher>,

    /// Whether the pane this prompt belongs to is currently focused.
    pane_is_focused: bool,

    /// Whether this terminal is viewing a shared session.
    is_shared_session_viewer: bool,

    agent_view_controller: ModelHandle<AgentViewController>,
}

const PROMPT_CHIP_DISPLAY_ID: &str = "PromptChipDisplay";

#[derive(Debug, Clone)]
pub enum PromptDisplayAction {
    SelectGitBranch { value: String },
}

pub enum PromptDisplayEvent {
    OpenFile(String),
    OpenTextFileInCodeEditor(String),
    ToggleMenu {
        open: bool,
    },
    OpenCodeReview,
    OpenConversationHistory,
    OpenCommandPaletteFiles,
    RunAgentQuery(String),
    TryExecuteCommand(PromptChipShellCommand),
    OpenAIDocument {
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
    },
}

impl PromptDisplay {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        prompt: ModelHandle<PromptType>,
        ai_input_model: ModelHandle<BlocklistAIInputModel>,
        ai_context_model: ModelHandle<BlocklistAIContextModel>,
        terminal_view_id: EntityId,
        menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
        session_context: Option<SessionContext>,
        current_repo_path: Option<PathBuf>,
        model_events: ModelHandle<ModelEventDispatcher>,
        agent_view_controller: ModelHandle<AgentViewController>,
        is_shared_session_viewer: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.observe(&prompt, |me, _, ctx| me.handle_prompt_change(ctx));

        // Subscribe to AI input model changes to trigger re-render when input mode changes
        ctx.subscribe_to_model(&ai_input_model, |_me, _model, event, ctx| {
            match event {
                BlocklistAIInputEvent::InputTypeChanged { .. }
                | BlocklistAIInputEvent::LockChanged { .. } => {
                    // Trigger re-render to update chip visibility based on new input mode
                    ctx.notify();
                }
            }
        });

        // Subscribe todo list updates to refresh the todo list chip visibility
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |me, _, event, ctx| {
                if let BlocklistAIHistoryEvent::UpdatedTodoList {
                    terminal_surface_id,
                } = event
                {
                    if *terminal_surface_id != me.terminal_view_id {
                        return;
                    }
                    ctx.notify();
                }
            },
        );

        ctx.subscribe_to_model(&agent_view_controller, |_, _, _, ctx| {
            ctx.notify();
        });

        Self {
            prompt,
            display_chips: vec![],
            ai_input_model,
            ai_context_model,
            terminal_view_id,
            menu_positioning_provider,
            session_context,
            current_repo_path,
            model_events,
            agent_view_controller,
            pane_is_focused: true,
            is_shared_session_viewer,
        }
    }

    pub fn has_open_chip_menu(&self, app: &AppContext) -> bool {
        self.display_chips
            .iter()
            .any(|chip| chip.as_ref(app).display_chip_kind().has_open_menu())
    }

    fn check_if_chip_values_have_changed(
        &mut self,
        new_chips: &[ChipResult],
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        self.display_chips.len() != new_chips.len()
            || new_chips.iter().enumerate().any(|(i, chip_result)| {
                let existing_chip = &self.display_chips[i];
                existing_chip.read(ctx, |chip, _| {
                    chip.value() != chip_result.value.as_ref()
                        || chip.chip_kind() != &chip_result.kind
                        || chip.on_click_values() != chip_result.on_click_values.as_slice()
                })
            })
    }

    fn handle_prompt_change(&mut self, ctx: &mut ViewContext<Self>) {
        let new_chips = self.collect_chips(ctx);

        let should_update = self.check_if_chip_values_have_changed(&new_chips, ctx);

        if should_update {
            self.reset_chips(&new_chips, ctx);
        }
        ctx.notify();
    }

    /// Collects the current chips from the prompt model, filtering out chips with no value.
    fn collect_chips(&self, ctx: &AppContext) -> Vec<ChipResult> {
        self.prompt
            .as_ref(ctx)
            .chips(ctx)
            .into_iter()
            .filter(|chip| chip.value.is_some())
            .collect()
    }

    fn reset_chips(&mut self, new_chips: &[ChipResult], ctx: &mut ViewContext<Self>) {
        let git_line_changes_info = git_line_changes_from_chips(new_chips);

        self.display_chips.clear();
        let mut display_chips = new_chips.iter().peekable();
        while let Some(chip_result) = display_chips.next() {
            let next_chip_kind = display_chips
                .peek()
                .map(|chip_result| chip_result.kind.clone());

            let is_shared_session_viewer = self.is_shared_session_viewer;

            let view_handle = ctx.add_typed_action_view(|ctx| {
                let mut chip = DisplayChip::new(
                    ctx,
                    chip_result.clone(),
                    next_chip_kind,
                    DisplayChipConfig {
                        ai_input_model: self.ai_input_model.clone(),
                        ai_context_model: self.ai_context_model.clone(),
                        terminal_view_id: self.terminal_view_id,
                        menu_positioning_provider: self.menu_positioning_provider.clone(),
                        session_context: self.session_context.clone(),
                        current_repo_path: self.current_repo_path.clone(),
                        model_events: self.model_events.clone(),
                        is_shared_session_viewer,
                        agent_view_controller: self.agent_view_controller.clone(),
                        ambient_agent_view_model: None,
                    },
                );
                chip.maybe_set_git_line_changes_info(git_line_changes_info.clone());
                chip
            });

            ctx.subscribe_to_view(&view_handle, move |_, _, event, ctx| match event {
                PromptDisplayChipEvent::OpenFile(value) => {
                    ctx.emit(PromptDisplayEvent::OpenFile(value.clone()));
                    ctx.notify();
                }
                PromptDisplayChipEvent::OpenTextFileInCodeEditor(value) => {
                    ctx.emit(PromptDisplayEvent::OpenTextFileInCodeEditor(value.clone()));
                    ctx.notify();
                }
                PromptDisplayChipEvent::ToggleMenu { open } => {
                    ctx.emit(PromptDisplayEvent::ToggleMenu { open: *open });
                    ctx.notify();
                }
                PromptDisplayChipEvent::OpenCodeReview => {
                    ctx.emit(PromptDisplayEvent::OpenCodeReview);
                    ctx.notify();
                }
                PromptDisplayChipEvent::OpenConversationHistory => {
                    ctx.emit(PromptDisplayEvent::OpenConversationHistory);
                    ctx.notify();
                }
                PromptDisplayChipEvent::OpenCommandPaletteFiles => {
                    ctx.emit(PromptDisplayEvent::OpenCommandPaletteFiles);
                    ctx.notify();
                }
                PromptDisplayChipEvent::RunAgentQuery(query) => {
                    ctx.emit(PromptDisplayEvent::RunAgentQuery(query.clone()));
                    ctx.notify();
                }
                PromptDisplayChipEvent::TryExecuteCommand(cmd) => {
                    ctx.emit(PromptDisplayEvent::TryExecuteCommand(cmd.clone()));
                    ctx.notify();
                }
                PromptDisplayChipEvent::OpenAIDocument {
                    document_id,
                    document_version,
                } => {
                    ctx.emit(PromptDisplayEvent::OpenAIDocument {
                        document_id: *document_id,
                        document_version: *document_version,
                    });
                    ctx.notify();
                }
            });

            self.display_chips.push(view_handle.clone());
        }
    }

    pub fn on_pane_focus_changed(&mut self, focused: bool, ctx: &mut ViewContext<Self>) {
        self.pane_is_focused = focused;
        let new_chips = self.collect_chips(ctx);
        if self.check_if_chip_values_have_changed(&new_chips, ctx) {
            self.reset_chips(&new_chips, ctx);
        }
        ctx.notify();
    }

    /// Update the session context and propagate it to all display chips
    pub fn update_session_context(
        &mut self,
        session_context: Option<SessionContext>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.session_context = session_context.clone();

        // Update all existing display chips with the new session context
        for chip_view in &self.display_chips {
            chip_view.update(ctx, |chip, chip_ctx| {
                chip.update_session_context(session_context.clone(), chip_ctx);
            });
        }
    }

    /// Update whether this terminal is viewing a shared session
    pub fn update_shared_session_viewer_status(
        &mut self,
        is_viewer: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.is_shared_session_viewer != is_viewer {
            self.is_shared_session_viewer = is_viewer;

            // Re-render chips to show/hide the shared session viewer-specific chips
            let new_chips = self.collect_chips(ctx);
            self.reset_chips(&new_chips, ctx);
            ctx.notify();
        }
    }

    /// The current prompt text.
    pub fn text(&self, ctx: &AppContext) -> String {
        self.prompt.as_ref(ctx).prompt_as_string(ctx)
    }

    #[cfg(feature = "integration_tests")]
    pub fn git_branch(&self, ctx: &AppContext) -> Option<String> {
        self.prompt.read(ctx, |prompt, ctx| {
            prompt
                .chips(ctx)
                .iter()
                .find(|chip_result| matches!(chip_result.kind, ContextChipKind::ShellGitBranch))
                .and_then(|chip_result| chip_result.value.as_ref().map(|v| v.to_string()))
        })
    }

    pub fn close_all_chip_menus(&mut self, ctx: &mut ViewContext<Self>) {
        for chip_view in &self.display_chips {
            chip_view.update(ctx, |chip, chip_ctx| {
                chip.handle_action(&DisplayChipAction::CloseMenu, chip_ctx);
            });
        }
        ctx.notify();
    }

    /// Update the current repository path and rebuild chips.
    pub fn update_repo_path(&mut self, repo_path: Option<PathBuf>, ctx: &mut ViewContext<Self>) {
        self.current_repo_path = repo_path;
        let new_chips = self.collect_chips(ctx);
        if self.check_if_chip_values_have_changed(&new_chips, ctx) {
            self.reset_chips(&new_chips, ctx);
        }
        ctx.notify();
    }
}

impl Entity for PromptDisplay {
    type Event = PromptDisplayEvent;
}

impl TypedActionView for PromptDisplay {
    type Action = PromptDisplayAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            PromptDisplayAction::SelectGitBranch { value } => {
                ctx.emit(PromptDisplayEvent::TryExecuteCommand(
                    PromptChipShellCommand::GitCheckout {
                        branch_name: value.clone(),
                    },
                ));
            }
        }
    }
}

impl View for PromptDisplay {
    fn ui_name() -> &'static str {
        "PromptDisplay"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            // Try to focus any open menu in the display chips
            for chip_view in &self.display_chips {
                let menu_focused =
                    chip_view.update(ctx, |chip, chip_ctx| chip.try_focus_open_menu(chip_ctx));
                if menu_focused {
                    return;
                }
            }
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let should_render_udi_chips = InputSettings::as_ref(app)
            .is_universal_developer_input_enabled(app)
            || FeatureFlag::AgentView.is_enabled();
        let mut row = if should_render_udi_chips {
            RowBuilder::Wrap(
                Wrap::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_alignment(MainAxisAlignment::Start)
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_run_spacing(super::spacing::UDI_ROW_RUN_SPACING),
            )
        } else {
            RowBuilder::Flex(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_constrain_horizontal_bounds_to_parent(true)
                    .with_main_axis_size(MainAxisSize::Min),
            )
        };

        self.display_chips.iter().for_each(|display_chip| {
            let chip = display_chip.as_ref(app);
            // AgentPlanAndTodoList is only shown in the agent input footer
            if matches!(chip.chip_kind(), ContextChipKind::AgentPlanAndTodoList) {
                return;
            }
            if chip.should_render(app) {
                row.add_child(ChildView::new(display_chip).finish());
            }
        });

        // This is a hack to apply horizontal clipping without vertical clipping (for padding).
        Container::new(
            Clipped::new(
                Container::new(row.finish())
                    .with_vertical_margin(4.)
                    .finish(),
            )
            .finish(),
        )
        .with_vertical_margin(-4.)
        .finish()
    }
}
