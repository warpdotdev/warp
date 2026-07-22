//! Session-wide projection of the active TUI interaction that blocks normal input.

use std::collections::HashMap;

use warp::tui_export::{
    AIActionStatus, AIAgentActionId, BlocklistAIActionEvent, BlocklistAIActionModel,
};
use warpui_core::elements::tui::{TuiChildView, TuiElement};
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle, ViewHandle};

use crate::handoff_block::TuiHandoffBlock;
use crate::orchestration_block::TuiOrchestrationBlock;
use crate::terminal_session_view::TuiTerminalSessionView;
use crate::tui_ask_question_view::TuiAskQuestionView;
use crate::tui_permission_prompt::TuiPermissionPrompt;

/// Where the active interaction is rendered while it suppresses the normal input stack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TuiBlockingInteractionPlacement {
    /// The interaction remains inside its owning transcript block.
    Transcript,
    /// The interaction replaces the normal session input stack.
    InputArea,
}

/// An existing interactive view that can own the session while awaiting user input.
#[derive(Clone)]
pub(crate) enum TuiBlockingInteraction {
    AskQuestion(ViewHandle<TuiAskQuestionView>),
    Permission(ViewHandle<TuiPermissionPrompt>),
    Orchestration(ViewHandle<TuiOrchestrationBlock>),
    Handoff(ViewHandle<TuiHandoffBlock>),
}

impl TuiBlockingInteraction {
    fn kind(&self) -> TuiBlockingInteractionKind {
        match self {
            Self::AskQuestion(_) => TuiBlockingInteractionKind::AskQuestion,
            Self::Permission(_) => TuiBlockingInteractionKind::Permission,
            Self::Orchestration(_) => TuiBlockingInteractionKind::Orchestration,
            Self::Handoff(_) => TuiBlockingInteractionKind::Handoff,
        }
    }

    fn view_id(&self) -> EntityId {
        match self {
            Self::AskQuestion(view) => view.id(),
            Self::Permission(view) => view.id(),
            Self::Orchestration(view) => view.id(),
            Self::Handoff(view) => view.id(),
        }
    }

    fn is_active(&self, ctx: &AppContext) -> bool {
        match self {
            Self::AskQuestion(view) => view.as_ref(ctx).is_awaiting_answers(ctx),
            Self::Permission(view) => view.as_ref(ctx).is_active(ctx),
            Self::Orchestration(view) => view.as_ref(ctx).is_awaiting_confirmation(ctx),
            Self::Handoff(view) => view.as_ref(ctx).is_active(),
        }
    }

    pub(crate) fn focus(&self, ctx: &mut warpui_core::ViewContext<TuiTerminalSessionView>) {
        match self {
            Self::AskQuestion(view) => {
                view.update(ctx, |view, ctx| view.focus(ctx));
            }
            Self::Permission(view) => {
                view.update(ctx, |view, ctx| view.focus(ctx));
            }
            Self::Orchestration(view) => ctx.focus(view),
            Self::Handoff(view) => {
                view.update(ctx, |view, ctx| view.focus(ctx));
            }
        }
    }

    fn render(&self) -> Box<dyn TuiElement> {
        match self {
            Self::AskQuestion(view) => TuiChildView::new(view).finish(),
            Self::Permission(view) => TuiChildView::new(view).finish(),
            Self::Orchestration(view) => TuiChildView::new(view).finish(),
            Self::Handoff(view) => TuiChildView::new(view).finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TuiBlockingInteractionKind {
    AskQuestion,
    Permission,
    Orchestration,
    Handoff,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TuiBlockingInteractionOwner {
    Action(AIAgentActionId),
    Session,
}

/// Stable identity used to suppress redundant notifications and focus transfer.
#[derive(Clone, Debug, Eq, PartialEq)]
struct TuiBlockingInteractionIdentity {
    owner: TuiBlockingInteractionOwner,
    kind: TuiBlockingInteractionKind,
    view_id: EntityId,
    placement: TuiBlockingInteractionPlacement,
}

#[derive(Clone)]
struct TuiBlockingInteractionRegistration {
    interaction: TuiBlockingInteraction,
    placement: TuiBlockingInteractionPlacement,
}

/// One resolved interaction snapshot consumed by focus and rendering.
#[derive(Clone)]
pub(crate) struct TuiBlockingInteractionSnapshot {
    identity: TuiBlockingInteractionIdentity,
    interaction: TuiBlockingInteraction,
    placement: TuiBlockingInteractionPlacement,
}

impl TuiBlockingInteractionSnapshot {
    pub(crate) fn view_id(&self) -> EntityId {
        self.identity.view_id
    }
    pub(crate) fn placement(&self) -> TuiBlockingInteractionPlacement {
        self.placement
    }

    pub(crate) fn focus(&self, ctx: &mut warpui_core::ViewContext<TuiTerminalSessionView>) {
        self.interaction.focus(ctx);
    }

    pub(crate) fn render(&self) -> Box<dyn TuiElement> {
        self.interaction.render()
    }
}

/// Emitted only when the resolved active interaction identity changes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TuiBlockingInteractionEvent;

/// Projects the authoritative action queue and optional session interaction into one active view.
pub(crate) struct TuiBlockingInteractionModel {
    action_model: ModelHandle<BlocklistAIActionModel>,
    action_interactions: HashMap<AIAgentActionId, TuiBlockingInteractionRegistration>,
    session_interaction: Option<TuiBlockingInteractionRegistration>,
    active: Option<TuiBlockingInteractionSnapshot>,
}

impl TuiBlockingInteractionModel {
    pub(crate) fn new(
        action_model: ModelHandle<BlocklistAIActionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(
            &action_model,
            |model, _, _: &BlocklistAIActionEvent, ctx| model.refresh(ctx),
        );
        Self {
            action_model,
            action_interactions: HashMap::new(),
            session_interaction: None,
            active: None,
        }
    }

    pub(crate) fn active(&self) -> Option<TuiBlockingInteractionSnapshot> {
        self.active.clone()
    }

    pub(crate) fn has_session_interaction(&self) -> bool {
        self.session_interaction.is_some()
    }

    #[cfg(test)]
    pub(crate) fn handoff_for_test(&self) -> Option<ViewHandle<TuiHandoffBlock>> {
        match self
            .session_interaction
            .as_ref()
            .map(|registration| &registration.interaction)
        {
            Some(TuiBlockingInteraction::Handoff(view)) => Some(view.clone()),
            Some(
                TuiBlockingInteraction::AskQuestion(_)
                | TuiBlockingInteraction::Permission(_)
                | TuiBlockingInteraction::Orchestration(_),
            )
            | None => None,
        }
    }

    pub(crate) fn register_action(
        &mut self,
        action_id: AIAgentActionId,
        interaction: TuiBlockingInteraction,
        placement: TuiBlockingInteractionPlacement,
        ctx: &mut ModelContext<Self>,
    ) {
        self.action_interactions.insert(
            action_id,
            TuiBlockingInteractionRegistration {
                interaction,
                placement,
            },
        );
        self.refresh(ctx);
    }

    pub(crate) fn unregister_action(
        &mut self,
        action_id: &AIAgentActionId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.action_interactions.remove(action_id);
        self.refresh(ctx);
    }

    pub(crate) fn set_session_interaction(
        &mut self,
        interaction: Option<(TuiBlockingInteraction, TuiBlockingInteractionPlacement)>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.session_interaction =
            interaction.map(
                |(interaction, placement)| TuiBlockingInteractionRegistration {
                    interaction,
                    placement,
                },
            );
        self.refresh(ctx);
    }

    pub(crate) fn refresh(&mut self, ctx: &mut ModelContext<Self>) {
        let active = self.resolve_active(ctx);
        let identity = active.as_ref().map(|snapshot| &snapshot.identity);
        let previous_identity = self.active.as_ref().map(|snapshot| &snapshot.identity);
        if identity == previous_identity {
            return;
        }
        self.active = active;
        ctx.emit(TuiBlockingInteractionEvent);
        ctx.notify();
    }

    fn resolve_active(&self, ctx: &AppContext) -> Option<TuiBlockingInteractionSnapshot> {
        if let Some(registration) = self
            .session_interaction
            .as_ref()
            .filter(|registration| registration.interaction.is_active(ctx))
        {
            return Some(Self::snapshot(
                TuiBlockingInteractionOwner::Session,
                registration,
            ));
        }

        let action_model = self.action_model.as_ref(ctx);
        let action = action_model.get_pending_action(ctx)?;
        if !matches!(
            action_model.get_action_status(&action.id),
            Some(AIActionStatus::Blocked)
        ) {
            return None;
        }
        let registration = self.action_interactions.get(&action.id)?;
        registration.interaction.is_active(ctx).then(|| {
            Self::snapshot(
                TuiBlockingInteractionOwner::Action(action.id.clone()),
                registration,
            )
        })
    }

    fn snapshot(
        owner: TuiBlockingInteractionOwner,
        registration: &TuiBlockingInteractionRegistration,
    ) -> TuiBlockingInteractionSnapshot {
        let interaction = registration.interaction.clone();
        TuiBlockingInteractionSnapshot {
            identity: TuiBlockingInteractionIdentity {
                owner,
                kind: interaction.kind(),
                view_id: interaction.view_id(),
                placement: registration.placement,
            },
            interaction,
            placement: registration.placement,
        }
    }
}

impl Entity for TuiBlockingInteractionModel {
    type Event = TuiBlockingInteractionEvent;
}

#[cfg(test)]
#[path = "blocking_interaction_tests.rs"]
mod tests;
