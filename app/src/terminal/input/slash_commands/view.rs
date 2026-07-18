use ai::skills::SkillReference;
use warpui::elements::ChildView;
use warpui::{AppContext, Element, Entity, ModelHandle, View, ViewContext, ViewHandle};

use crate::ai::blocklist::agent_view::AgentViewController;
use crate::search::slash_command_menu::SlashCommandId;
use crate::server::ids::SyncId;
use crate::terminal::input::buffer_model::InputBufferModel;
use crate::terminal::input::inline_menu::{InlineMenuEvent, InlineMenuPositioner, InlineMenuView};
use crate::terminal::input::slash_command_model::{SlashCommandEntryState, SlashCommandModel};
use crate::terminal::input::slash_commands::{
    build_slash_command_mixer, slash_command_query, AcceptSlashCommandOrSavedPrompt,
    GuiSlashCommandDataSource, GuiZeroStateDataSource, SlashCommandMixer, UpdatedActiveCommands,
};
use crate::terminal::input::suggestions_mode_model::{
    InputSuggestionsModeEvent, InputSuggestionsModeModel,
};

#[derive(Debug, Clone, Copy)]
pub enum CloseReason {
    NoResults,
    ManualDismissal,
}

impl CloseReason {
    pub fn is_manual_dismissal(&self) -> bool {
        matches!(self, Self::ManualDismissal)
    }

    pub fn is_no_results(&self) -> bool {
        matches!(self, Self::NoResults)
    }
}

/// Events emitted by the slash commands menu
#[derive(Debug, Clone)]
pub enum SlashCommandsEvent {
    Close(CloseReason),
    SelectedSavedPrompt {
        id: SyncId,
    },
    /// `cmd_or_ctrl_enter` is true if accepted via Cmd/Ctrl+Enter (vs Enter/click).
    SelectedStaticCommand {
        id: SlashCommandId,
        cmd_or_ctrl_enter: bool,
    },
    /// A skill was selected from the menu. Contains the skill name (for buffer insertion)
    /// and path/bundled_skill_id (for execution context).
    SelectedSkill {
        reference: SkillReference,
        name: String,
    },
}

/// Wrapper around `InlineMenuView` specialized for slash commands.
///
/// This view:
/// - Creates and owns the slash command data sources
/// - Sets up the mixer with those sources
/// - Maps `InlineMenuEvent<SelectItem>` to `SlashCommandsEvent`
/// - Subscribes to `SlashCommandModel` for query updates
pub struct InlineSlashCommandView {
    menu_view: ViewHandle<InlineMenuView<AcceptSlashCommandOrSavedPrompt>>,
    suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
    mixer: ModelHandle<SlashCommandMixer>,
    input_buffer_model: ModelHandle<InputBufferModel>,
}

impl InlineSlashCommandView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        slash_command_model: &ModelHandle<SlashCommandModel>,
        positioner: &ModelHandle<InlineMenuPositioner>,
        slash_commands_source: ModelHandle<GuiSlashCommandDataSource>,
        suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        input_buffer_model: ModelHandle<InputBufferModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(
            &slash_commands_source,
            |me, _, _: &UpdatedActiveCommands, ctx| {
                me.mixer.update(ctx, |mixer, ctx| {
                    // Auto-rerun queries if set of active commands changed.
                    if let Some(query) = mixer.current_query().cloned() {
                        mixer.run_query(query, ctx);
                    }
                });
            },
        );
        let zero_state_source = GuiZeroStateDataSource::new(&slash_commands_source);
        let mixer = ctx.add_model(|ctx| {
            build_slash_command_mixer(slash_commands_source.clone(), zero_state_source, ctx)
        });

        let menu_view = ctx.add_typed_action_view(|ctx| {
            InlineMenuView::new(
                mixer.clone(),
                positioner.clone(),
                &suggestions_mode_model,
                agent_view_controller,
                ctx,
            )
        });

        ctx.subscribe_to_view(&menu_view, |me, _, event, ctx| match event {
            InlineMenuEvent::AcceptedItem {
                item,
                cmd_or_ctrl_shift_enter,
            } => {
                me.handle_selection(item, *cmd_or_ctrl_shift_enter, ctx);
            }
            InlineMenuEvent::NoResults => {
                if me.suggestions_mode_model.as_ref(ctx).is_slash_commands() {
                    ctx.emit(SlashCommandsEvent::Close(CloseReason::NoResults));
                }
            }
            InlineMenuEvent::Dismissed => {
                ctx.emit(SlashCommandsEvent::Close(CloseReason::ManualDismissal));
            }
            InlineMenuEvent::SelectedItem { .. } | InlineMenuEvent::TabChanged => (),
        });

        ctx.subscribe_to_model(slash_command_model, |me, model, _, ctx| {
            // If the inline menu isn't open, don't keep re-running search as the user types.
            //
            // This prevents expensive searching (e.g. saved prompts) when the menu has been
            // closed (such as after selecting a command and typing an argument).
            if !me.suggestions_mode_model.as_ref(ctx).is_slash_commands() {
                return;
            }

            match model.as_ref(ctx).state().clone() {
                SlashCommandEntryState::None
                | SlashCommandEntryState::Composing { .. }
                | SlashCommandEntryState::SlashCommand(_) => {
                    me.run_query_for_current_slash_filter(ctx);
                }
                _ => (),
            }
        });

        ctx.subscribe_to_model(&suggestions_mode_model, |me, _, event, ctx| {
            let InputSuggestionsModeEvent::ModeChanged { .. } = event;
            if me.suggestions_mode_model.as_ref(ctx).is_closed() {
                me.mixer.update(ctx, |mixer, ctx| {
                    mixer.reset_results(ctx);
                });
                return;
            }

            // If the menu reopened while the buffer still contains a slash query,
            // ensure we run a query so the menu isn't showing stale/empty results.
            if me.suggestions_mode_model.as_ref(ctx).is_slash_commands() {
                me.run_query_for_current_slash_filter(ctx);
            }
        });

        Self {
            menu_view,
            mixer,
            suggestions_mode_model,
            input_buffer_model,
        }
    }

    fn run_query_for_current_slash_filter(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(filter) = self
            .input_buffer_model
            .as_ref(ctx)
            .current_value()
            .strip_prefix('/')
            .map(ToOwned::to_owned)
        else {
            return;
        };

        self.mixer.update(ctx, move |mixer, ctx| {
            if mixer.current_query().is_some_and(|q| q.text == filter) {
                return;
            }
            mixer.run_query(slash_command_query(&filter), ctx);
        });
    }

    fn handle_selection(
        &mut self,
        item: &AcceptSlashCommandOrSavedPrompt,
        cmd_or_ctrl_enter: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        match item {
            AcceptSlashCommandOrSavedPrompt::SlashCommand { id } => {
                ctx.emit(SlashCommandsEvent::SelectedStaticCommand {
                    id: *id,
                    cmd_or_ctrl_enter,
                });
            }
            AcceptSlashCommandOrSavedPrompt::SavedPrompt { id } => {
                ctx.emit(SlashCommandsEvent::SelectedSavedPrompt { id: *id });
            }
            AcceptSlashCommandOrSavedPrompt::Skill { name, reference } => {
                ctx.emit(SlashCommandsEvent::SelectedSkill {
                    reference: reference.clone(),
                    name: name.clone(),
                });
            }
        }
    }

    pub fn select_up(&self, ctx: &mut ViewContext<Self>) {
        self.menu_view.update(ctx, |v, ctx| v.select_up(ctx));
    }

    pub fn select_down(&self, ctx: &mut ViewContext<Self>) {
        self.menu_view.update(ctx, |v, ctx| v.select_down(ctx));
    }

    pub fn accept_selected_item(&self, cmd_or_ctrl_enter: bool, ctx: &mut ViewContext<Self>) {
        self.menu_view
            .update(ctx, |v, ctx| v.accept_selected_item(cmd_or_ctrl_enter, ctx));
    }

    pub fn result_count(&self, app: &AppContext) -> usize {
        self.mixer.as_ref(app).results().len()
    }
}

impl View for InlineSlashCommandView {
    fn ui_name() -> &'static str {
        "InlineSlashCommandView"
    }

    fn render(&self, _app: &warpui::AppContext) -> Box<dyn Element> {
        ChildView::new(&self.menu_view).finish()
    }
}

impl Entity for InlineSlashCommandView {
    type Event = SlashCommandsEvent;
}
