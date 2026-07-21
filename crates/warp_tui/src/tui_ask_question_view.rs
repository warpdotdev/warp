//! Interactive TUI card for the `AskUserQuestion` agent tool call.

use std::collections::HashSet;
use std::time::Duration;

use warp::tui_export::{
    AIActionStatus, AIAgentActionId, AIAgentActionResultType, AIConversationId,
    AskUserQuestionAction, AskUserQuestionAnswerItem, AskUserQuestionEffect, AskUserQuestionItem,
    AskUserQuestionPhase, AskUserQuestionResult, AskUserQuestionSession, BlocklistAIActionEvent,
    BlocklistAIActionModel, BlocklistAIHistoryModel, OptionFooter, OptionRow, OptionSnapshot,
    OptionSourceStatus,
};
use warpui::SingletonEntity;
use warpui_core::r#async::{SpawnedFutureHandle, Timer};
use warpui_core::elements::tui::{
    Modifier, TuiChildView, TuiContainer, TuiElement, TuiFlex, TuiParentElement, TuiText,
};
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{EditableBinding, FixedBinding};
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::keybindings::{TUI_BINDING_GROUP, is_tui_owned_binding};
use crate::option_selector::{OptionSelectorPage, TuiOptionSelector, TuiOptionSelectorEvent};
use crate::tui_builder::TuiUiBuilder;

const ASK_QUESTION_ACTIVE: &str = "TuiAskQuestionActive";
const AUTO_ADVANCE_DELAY: Duration = Duration::from_millis(300);

/// Registers controls that must win over the surrounding terminal session
/// while an ask-question card is the active blocker.
pub(crate) fn init(app: &mut AppContext) {
    let predicate = id!(TuiAskQuestionView::ui_name()) & id!(ASK_QUESTION_ACTIVE);
    app.register_fixed_bindings([FixedBinding::new(
        "ctrl-c",
        TuiAskQuestionViewAction::SkipAll,
        predicate.clone(),
    )
    .with_group(TUI_BINDING_GROUP)]);
    app.register_editable_bindings([
        EditableBinding::new(
            "tui:ask-question:confirm",
            "Select or confirm the highlighted answer",
            TuiAskQuestionViewAction::Enter,
        )
        .with_context_predicate(predicate.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("enter"),
        EditableBinding::new(
            "tui:ask-question:previous",
            "Show the previous question",
            TuiAskQuestionViewAction::Previous,
        )
        .with_context_predicate(predicate.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("left"),
        EditableBinding::new(
            "tui:ask-question:next",
            "Show the next question",
            TuiAskQuestionViewAction::Next,
        )
        .with_context_predicate(predicate.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("right"),
        EditableBinding::new(
            "tui:ask-question:next",
            "Show the next question",
            TuiAskQuestionViewAction::Next,
        )
        .with_context_predicate(predicate)
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("tab"),
    ]);
    app.register_tui_binding_validator::<TuiAskQuestionView>(is_tui_owned_binding);
}

#[derive(Clone, Debug)]
pub(super) enum TuiAskQuestionViewAction {
    Enter,
    Previous,
    Next,
    SkipAll,
}

pub(super) enum TuiAskQuestionViewEvent {
    LayoutChanged,
}

pub(super) struct TuiAskQuestionView {
    action_model: ModelHandle<BlocklistAIActionModel>,
    conversation_id: AIConversationId,
    action_id: AIAgentActionId,
    source_questions: Vec<AskUserQuestionItem>,
    session: AskUserQuestionSession,
    selector: ViewHandle<TuiOptionSelector>,
    auto_advance: Option<SpawnedFutureHandle>,
    pending_auto_advance_question_index: Option<usize>,
}

impl TuiAskQuestionView {
    pub(super) fn new(
        action_model: ModelHandle<BlocklistAIActionModel>,
        conversation_id: AIConversationId,
        action_id: AIAgentActionId,
        questions: Vec<AskUserQuestionItem>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let session = AskUserQuestionSession::new(questions.clone());
        let selector = ctx.add_typed_action_tui_view(TuiOptionSelector::new);
        let mut view = Self {
            action_model: action_model.clone(),
            conversation_id,
            action_id,
            source_questions: questions,
            session,
            selector: selector.clone(),
            auto_advance: None,
            pending_auto_advance_question_index: None,
        };
        view.show_current_question(ctx);

        ctx.subscribe_to_view(&selector, |me, _, event, ctx| {
            me.handle_selector_event(event, ctx);
        });
        ctx.subscribe_to_model(&action_model, |me, _, event, ctx| {
            if event.action_id() != &me.action_id {
                return;
            }
            if matches!(
                event,
                BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(_)
            ) {
                ctx.focus(&me.selector);
            }
            if matches!(event, BlocklistAIActionEvent::FinishedAction { .. }) {
                me.abort_auto_advance();
            }
            me.invalidate_layout(ctx);
        });
        if view.is_waiting_on_answers(ctx) {
            ctx.focus(&selector);
        }
        view
    }

    fn is_waiting_on_answers(&self, app: &AppContext) -> bool {
        self.action_model
            .as_ref(app)
            .get_action_status(&self.action_id)
            .is_some_and(|status| status.is_blocked())
    }

    pub(super) fn matches_action(
        &self,
        action_id: &AIAgentActionId,
        questions: &[AskUserQuestionItem],
    ) -> bool {
        &self.action_id == action_id && self.source_questions == questions
    }

    /// Restored conversations can predate persisted action results. Match the
    /// GUI fallback by treating a missing result as skipped once the owning
    /// conversation and all of its actions are terminal.
    fn should_restore_as_skipped(&self, app: &AppContext) -> bool {
        BlocklistAIHistoryModel::as_ref(app)
            .conversation(&self.conversation_id)
            .is_some_and(|conversation| !conversation.status().is_in_progress())
            && !self
                .action_model
                .as_ref(app)
                .has_unfinished_actions_for_conversation(self.conversation_id)
    }

    fn show_current_question(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(current) = self.session.current() else {
            return;
        };
        let rows = current
            .question
            .multiple_choice_options()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(index, option)| OptionRow {
                id: index.to_string(),
                label: option.label.clone(),
                harness: None,
                badge: None,
                disabled_reason: None,
            })
            .collect();
        let footer = current
            .question
            .supports_other()
            .then(|| OptionFooter::CustomText {
                label: "Other…".to_owned(),
            });
        let selected_id = current.draft.and_then(|draft| {
            draft.other_text.clone().or_else(|| {
                draft
                    .selected_option_indices
                    .iter()
                    .min()
                    .map(usize::to_string)
            })
        });
        let snapshot = OptionSnapshot {
            rows,
            selected_id,
            status: OptionSourceStatus::Ready,
            footer,
        };
        let selected_ids = current
            .draft
            .map(|draft| {
                draft
                    .selected_option_indices
                    .iter()
                    .map(usize::to_string)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let show_markers = current.question.is_multiselect();
        self.selector.update(ctx, |selector, ctx| {
            selector.set_page(
                OptionSelectorPage {
                    header: None,
                    snapshot,
                    searchable: false,
                    row_shortcuts: Default::default(),
                },
                ctx,
            );
            selector.set_question_state(selected_ids, show_markers, ctx);
        });
        self.invalidate_layout(ctx);
    }

    fn refresh_selection(&self, ctx: &mut ViewContext<Self>) {
        let Some(current) = self.session.current() else {
            return;
        };
        let selected_ids = current
            .draft
            .map(|draft| {
                draft
                    .selected_option_indices
                    .iter()
                    .map(usize::to_string)
                    .collect()
            })
            .unwrap_or_default();
        self.selector.update(ctx, |selector, ctx| {
            selector.set_question_state(selected_ids, current.question.is_multiselect(), ctx);
        });
    }

    fn handle_selector_event(
        &mut self,
        event: &TuiOptionSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TuiOptionSelectorEvent::Confirmed { id } => {
                let Ok(option_index) = id.parse::<usize>() else {
                    return;
                };
                self.abort_auto_advance();
                let effect = self
                    .session
                    .apply(AskUserQuestionAction::ToggleOption { option_index });
                self.handle_effect(effect, ctx);
            }
            TuiOptionSelectorEvent::CustomTextSubmitted { value } => {
                self.abort_auto_advance();
                let effect = self.session.apply(AskUserQuestionAction::SaveOtherText {
                    text: Some(value.clone()),
                });
                self.handle_effect(effect, ctx);
            }
            TuiOptionSelectorEvent::CustomTextOpened => {
                self.abort_auto_advance();
                let _ = self.session.apply(AskUserQuestionAction::OpenOtherInput);
                self.invalidate_layout(ctx);
            }
            TuiOptionSelectorEvent::LayoutInvalidated => self.invalidate_layout(ctx),
            TuiOptionSelectorEvent::RetryRequested | TuiOptionSelectorEvent::Dismissed => {}
        }
    }

    fn commit_active_other_text(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self
            .selector
            .read(ctx, |selector, ctx| selector.active_custom_text(ctx));
        let Some(text) = text else {
            return;
        };
        let _ = self
            .session
            .apply(AskUserQuestionAction::SaveOtherText { text: Some(text) });
    }

    fn abort_auto_advance(&mut self) {
        self.pending_auto_advance_question_index = None;
        if let Some(handle) = self.auto_advance.take() {
            handle.abort();
        }
    }

    /// Briefly leaves the chosen answer visible before advancing to the next
    /// question or submitting the questionnaire. Further input cancels the
    /// task, and the captured index prevents a stale task from advancing a
    /// different question.
    fn schedule_auto_advance(&mut self, ctx: &mut ViewContext<Self>) {
        let question_index = self.session.current_question_index();
        self.abort_auto_advance();
        self.pending_auto_advance_question_index = Some(question_index);
        self.auto_advance = Some(ctx.spawn(
            async move {
                Timer::after(AUTO_ADVANCE_DELAY).await;
                question_index
            },
            |me, question_index, ctx| {
                me.auto_advance = None;
                if me.pending_auto_advance_question_index == Some(question_index)
                    && me.session.current_question_index() == question_index
                {
                    let effect = me.session.apply(AskUserQuestionAction::Confirm);
                    me.handle_effect(effect, ctx);
                }
            },
        ));
    }

    fn handle_effect(&mut self, effect: AskUserQuestionEffect, ctx: &mut ViewContext<Self>) {
        match effect {
            AskUserQuestionEffect::Noop => {}
            AskUserQuestionEffect::RefreshCurrent => self.refresh_selection(ctx),
            AskUserQuestionEffect::FocusOtherInput => {
                self.selector
                    .update(ctx, |selector, ctx| selector.confirm_selected(ctx));
            }
            AskUserQuestionEffect::ShowQuestion => self.show_current_question(ctx),
            AskUserQuestionEffect::ScheduleAutoAdvance => {
                self.refresh_selection(ctx);
                self.schedule_auto_advance(ctx);
            }
            AskUserQuestionEffect::Submit(answers) => self.submit_answers(answers, ctx),
        }
        self.invalidate_layout(ctx);
    }

    fn submit_answers(
        &mut self,
        answers: Vec<AskUserQuestionAnswerItem>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.abort_auto_advance();
        let action_id = self.action_id.clone();
        let conversation_id = self.conversation_id;
        self.action_model.update(ctx, |action_model, ctx| {
            action_model
                .ask_user_question_executor(ctx)
                .update(ctx, |executor, _| executor.complete(answers.clone()));
            action_model.execute_action(&action_id, conversation_id, ctx);
        });
    }

    fn invalidate_layout(&self, ctx: &mut ViewContext<Self>) {
        ctx.emit(TuiAskQuestionViewEvent::LayoutChanged);
        ctx.notify();
    }

    fn render_active(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        let Some(current) = self.session.current() else {
            return self.render_unavailable(app);
        };
        let index = self.session.current_question_index() + 1;
        let total = self.session.question_count();
        let position = TuiText::from_spans([
            ("← ".to_owned(), builder.muted_text_style()),
            (format!("{index} "), builder.primary_text_style()),
            (format!("of {total} "), builder.muted_text_style()),
            ("→".to_owned(), builder.muted_text_style()),
        ])
        .finish();
        let header = TuiFlex::row()
            .child(
                TuiText::from_spans([
                    ("■ ".to_owned(), builder.attention_glyph_style()),
                    (
                        "Agent questions".to_owned(),
                        builder.primary_text_style().add_modifier(Modifier::BOLD),
                    ),
                ])
                .finish(),
            )
            .flex_child(TuiFlex::row().finish())
            .child(position)
            .finish();
        let mut question = current.question.question.clone();
        if current.question.is_multiselect() {
            question.push_str(" (select all that apply)");
        }
        let body = TuiFlex::column()
            .child(header)
            .child(TuiText::new(" ").finish())
            .child(
                TuiText::new(question)
                    .with_style(builder.primary_text_style().add_modifier(Modifier::BOLD))
                    .finish(),
            )
            .child(TuiChildView::new(&self.selector).finish())
            .child(TuiText::new(" ").finish())
            .child(
                TuiText::from_spans([
                    ("Enter or number ".to_owned(), builder.primary_text_style()),
                    ("to select ".to_owned(), builder.muted_text_style()),
                    ("Tab or ← → ".to_owned(), builder.primary_text_style()),
                    ("to navigate ".to_owned(), builder.muted_text_style()),
                    ("Ctrl + C ".to_owned(), builder.primary_text_style()),
                    ("to cancel question".to_owned(), builder.muted_text_style()),
                ])
                .truncate()
                .finish(),
            )
            .finish();
        TuiContainer::new(body)
            .with_padding(1)
            .with_border_style(builder.accent_border_style())
            .with_background(builder.question_surface_background())
            .finish()
    }

    fn render_unavailable(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        TuiText::from_spans([
            ("■ ".to_owned(), builder.muted_text_style()),
            (
                "Questions unavailable".to_owned(),
                builder.muted_text_style(),
            ),
        ])
        .finish()
    }

    fn render_finished_result(
        &self,
        result: &AskUserQuestionResult,
        app: &AppContext,
    ) -> Box<dyn TuiElement> {
        match result {
            AskUserQuestionResult::Success { answers } => self.render_answers(answers, app),
            AskUserQuestionResult::SkippedByAutoApprove { .. } => {
                self.render_summary("Questions skipped due to auto-approve", false, app)
            }
            AskUserQuestionResult::Error(_) | AskUserQuestionResult::Cancelled => {
                self.render_summary("Questions skipped", false, app)
            }
        }
    }

    fn render_answers(
        &self,
        answers: &[AskUserQuestionAnswerItem],
        app: &AppContext,
    ) -> Box<dyn TuiElement> {
        let answered = answers.iter().filter(|answer| !answer.is_skipped()).count();
        let total = answers.len();
        let label = if answered == 0 {
            "Questions skipped".to_owned()
        } else if answered == total && total == 1 {
            "Answered question".to_owned()
        } else if answered == total {
            format!("Answered all {total} questions")
        } else {
            format!("Answered {answered} of {total} questions")
        };
        let builder = TuiUiBuilder::from_app(app);
        let mut content = TuiFlex::column();
        content.add_child(self.render_summary(&label, answered > 0, app));
        for question in self.session.questions() {
            let answer = answers
                .iter()
                .find(|answer| match answer {
                    AskUserQuestionAnswerItem::Answered { question_id, .. }
                    | AskUserQuestionAnswerItem::Skipped { question_id } => {
                        question_id == &question.question_id
                    }
                })
                .map(AskUserQuestionAnswerItem::display_text)
                .unwrap_or_else(|| "Skipped".to_owned());
            content.add_child(
                TuiContainer::new(
                    TuiFlex::column()
                        .child(
                            TuiText::new(format!("Q: {}", question.question))
                                .with_style(builder.primary_text_style())
                                .finish(),
                        )
                        .child(
                            TuiText::new(format!("A: {answer}"))
                                .with_style(builder.muted_text_style())
                                .finish(),
                        )
                        .finish(),
                )
                .with_padding_left(2)
                .finish(),
            );
        }
        content.finish()
    }

    fn render_summary(
        &self,
        label: &str,
        succeeded: bool,
        app: &AppContext,
    ) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        let glyph_style = if succeeded {
            builder.success_glyph_style()
        } else {
            builder.muted_text_style()
        };
        TuiText::from_spans([
            (if succeeded { "✓ " } else { "■ " }.to_owned(), glyph_style),
            (label.to_owned(), builder.muted_text_style()),
        ])
        .finish()
    }
}

#[cfg(test)]
#[path = "tui_ask_question_view_tests.rs"]
mod tests;

impl Entity for TuiAskQuestionView {
    type Event = TuiAskQuestionViewEvent;
}

impl TuiView for TuiAskQuestionView {
    fn ui_name() -> &'static str {
        "TuiAskQuestionView"
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        vec![self.selector.id()]
    }

    fn keymap_context(&self, app: &AppContext) -> warpui_core::keymap::Context {
        let mut context = Self::default_keymap_context();
        if self.session.is_editing()
            && self.session.current().is_some()
            && self.is_waiting_on_answers(app)
        {
            context.set.insert(ASK_QUESTION_ACTIVE);
        }
        context
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let status = self
            .action_model
            .as_ref(app)
            .get_action_status(&self.action_id);
        if let Some(AIActionStatus::Finished(result)) = status.as_ref() {
            if let AIAgentActionResultType::AskUserQuestion(result) = &result.result {
                return self.render_finished_result(result, app);
            }
            return self.render_unavailable(app);
        }
        if status.is_none() && self.should_restore_as_skipped(app) {
            return self.render_summary("Questions skipped", false, app);
        }
        match self.session.phase() {
            AskUserQuestionPhase::Completed { answers } => self.render_answers(answers, app),
            AskUserQuestionPhase::Editing if self.is_waiting_on_answers(app) => {
                self.render_active(app)
            }
            AskUserQuestionPhase::Editing => {
                let builder = TuiUiBuilder::from_app(app);
                TuiText::from_spans([
                    ("○ ".to_owned(), builder.muted_text_style()),
                    ("Agent questions".to_owned(), builder.muted_text_style()),
                ])
                .finish()
            }
        }
    }
}

impl TypedActionView for TuiAskQuestionView {
    type Action = TuiAskQuestionViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        if !self.session.is_editing() || !self.is_waiting_on_answers(ctx) {
            return;
        }
        self.abort_auto_advance();
        match action {
            TuiAskQuestionViewAction::Enter => {
                let (highlighted_index, active_other_text) =
                    self.selector.read(ctx, |selector, ctx| {
                        (
                            selector.highlighted_question_index(),
                            selector.active_custom_text(ctx),
                        )
                    });
                let effect = self.session.apply(AskUserQuestionAction::SubmitAnswer {
                    highlighted_index,
                    active_other_text,
                });
                self.handle_effect(effect, ctx);
            }
            TuiAskQuestionViewAction::Previous => {
                self.commit_active_other_text(ctx);
                let effect = self.session.apply(AskUserQuestionAction::NavigatePrev);
                self.handle_effect(effect, ctx);
            }
            TuiAskQuestionViewAction::Next => {
                self.commit_active_other_text(ctx);
                let effect = self.session.apply(AskUserQuestionAction::NavigateNext);
                self.handle_effect(effect, ctx);
            }
            TuiAskQuestionViewAction::SkipAll => {
                let effect = self.session.apply(AskUserQuestionAction::SkipAll);
                self.handle_effect(effect, ctx);
            }
        }
    }
}
