use std::collections::HashSet;

use itertools::Itertools;

use super::action::{AskUserQuestionItem, AskUserQuestionType};
use super::action_result::AskUserQuestionAnswerItem;

/// In-progress answer data for a single question while the user is editing.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct QuestionDraft {
    pub selected_option_indices: HashSet<usize>,
    pub other_text: Option<String>,
    pub is_other_input_active: bool,
}

impl QuestionDraft {
    pub fn has_answer(&self) -> bool {
        !self.selected_option_indices.is_empty()
            || self
                .other_text
                .as_deref()
                .is_some_and(|text| !text.is_empty())
    }

    fn is_empty(&self) -> bool {
        !self.has_answer() && !self.is_other_input_active
    }
}

/// One slot in the editing state's question-aligned draft list.
///
/// Keeping untouched questions distinct from drafts with active UI state lets
/// frontends restore navigation without manufacturing empty answers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum QuestionDraftState {
    #[default]
    Unanswered,
    Answered(QuestionDraft),
}

#[derive(Clone, Copy)]
pub struct AskUserQuestionCurrent<'a> {
    pub question: &'a AskUserQuestionItem,
    pub draft: Option<&'a QuestionDraft>,
}

#[derive(Clone, Copy)]
pub enum AskUserQuestionPhase<'a> {
    Editing,
    Completed {
        answers: &'a [AskUserQuestionAnswerItem],
    },
}

/// Mutable state used only while the questionnaire is accepting answers.
///
/// Each draft index corresponds to the same index in
/// [`AskUserQuestionSession::questions`].
#[derive(Clone, Debug, Eq, PartialEq)]
struct AskUserQuestionEditingState {
    current_question_index: usize,
    drafts: Vec<QuestionDraftState>,
}

impl AskUserQuestionEditingState {
    fn new(draft_count: usize) -> Self {
        Self {
            current_question_index: 0,
            drafts: vec![QuestionDraftState::Unanswered; draft_count],
        }
    }

    fn current_question_index(&self) -> usize {
        self.current_question_index
    }

    fn current_draft(&self) -> Option<&QuestionDraft> {
        self.draft_for_question(self.current_question_index)
    }

    fn draft_for_question(&self, index: usize) -> Option<&QuestionDraft> {
        let QuestionDraftState::Answered(draft) = self.drafts.get(index)? else {
            return None;
        };
        Some(draft)
    }

    fn is_last_question(&self, question_count: usize) -> bool {
        self.current_question_index + 1 >= question_count
    }

    fn update_current_draft(&mut self, update: impl FnOnce(&mut QuestionDraft)) {
        let Some(slot) = self.drafts.get_mut(self.current_question_index) else {
            return;
        };

        let mut draft = match std::mem::take(slot) {
            QuestionDraftState::Unanswered => QuestionDraft::default(),
            QuestionDraftState::Answered(draft) => draft,
        };
        update(&mut draft);
        *slot = if draft.is_empty() {
            QuestionDraftState::Unanswered
        } else {
            QuestionDraftState::Answered(draft)
        };
    }
}

/// Top-level questionnaire lifecycle state.
///
/// Editing owns the current position and drafts; completion replaces that
/// transient state with the final answer payload rendered by both frontends.
#[derive(Clone, Debug, Eq, PartialEq)]
enum AskUserQuestionState {
    Editing(AskUserQuestionEditingState),
    Completed {
        answers: Vec<AskUserQuestionAnswerItem>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AskUserQuestionAction {
    ToggleOption {
        option_index: usize,
    },
    OpenOtherInput,
    SaveOtherText {
        text: Option<String>,
    },
    NavigatePrev,
    NavigateNext,
    SubmitAnswer {
        highlighted_index: Option<usize>,
        active_other_text: Option<String>,
    },
    Confirm,
    SkipAll,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AskUserQuestionEffect {
    Noop,
    RefreshCurrent,
    FocusOtherInput,
    ShowQuestion,
    ScheduleAutoAdvance,
    Submit(Vec<AskUserQuestionAnswerItem>),
}

/// Frontend-neutral questionnaire state shared by the GUI and TUI.
pub struct AskUserQuestionSession {
    questions: Vec<AskUserQuestionItem>,
    state: AskUserQuestionState,
}

impl AskUserQuestionSession {
    pub fn new(mut questions: Vec<AskUserQuestionItem>) -> Self {
        // Put multi-select questions before single-select so the last question
        // can auto-submit after a single option toggle.
        questions.sort_by_key(|question| !question.is_multiselect());
        Self {
            state: AskUserQuestionState::Editing(AskUserQuestionEditingState::new(questions.len())),
            questions,
        }
    }

    pub fn phase(&self) -> AskUserQuestionPhase<'_> {
        match &self.state {
            AskUserQuestionState::Editing(_) => AskUserQuestionPhase::Editing,
            AskUserQuestionState::Completed { answers } => {
                AskUserQuestionPhase::Completed { answers }
            }
        }
    }

    pub fn is_editing(&self) -> bool {
        matches!(self.state, AskUserQuestionState::Editing(_))
    }

    pub fn questions(&self) -> &[AskUserQuestionItem] {
        &self.questions
    }

    pub fn question_count(&self) -> usize {
        self.questions.len()
    }

    pub fn has_multiple_questions(&self) -> bool {
        self.question_count() > 1
    }

    pub fn current(&self) -> Option<AskUserQuestionCurrent<'_>> {
        let AskUserQuestionState::Editing(editing) = &self.state else {
            return None;
        };

        Some(AskUserQuestionCurrent {
            question: self.questions.get(editing.current_question_index())?,
            draft: editing.current_draft(),
        })
    }

    pub fn draft_for_question(&self, index: usize) -> Option<&QuestionDraft> {
        let AskUserQuestionState::Editing(editing) = &self.state else {
            return None;
        };
        editing.draft_for_question(index)
    }

    pub fn current_question_index(&self) -> usize {
        match &self.state {
            AskUserQuestionState::Editing(editing) => editing.current_question_index(),
            AskUserQuestionState::Completed { .. } => 0,
        }
    }

    pub fn is_last_question(&self) -> bool {
        match &self.state {
            AskUserQuestionState::Editing(editing) => {
                editing.is_last_question(self.questions.len())
            }
            AskUserQuestionState::Completed { .. } => false,
        }
    }

    pub fn apply(&mut self, action: AskUserQuestionAction) -> AskUserQuestionEffect {
        match action {
            AskUserQuestionAction::ToggleOption { option_index } => {
                self.toggle_option(option_index)
            }
            AskUserQuestionAction::OpenOtherInput => self.open_other_input(),
            AskUserQuestionAction::SaveOtherText { text } => self.save_other_text(text),
            AskUserQuestionAction::NavigatePrev => self.navigate_prev(),
            AskUserQuestionAction::NavigateNext => self.navigate_next(),
            AskUserQuestionAction::SubmitAnswer {
                highlighted_index,
                active_other_text,
            } => self.submit_answer(highlighted_index, active_other_text),
            AskUserQuestionAction::Confirm => self.confirm(),
            AskUserQuestionAction::SkipAll => self.skip_all(),
        }
    }

    fn editing_state_mut(&mut self) -> Option<&mut AskUserQuestionEditingState> {
        let AskUserQuestionState::Editing(editing) = &mut self.state else {
            return None;
        };
        Some(editing)
    }

    fn toggle_option(&mut self, option_index: usize) -> AskUserQuestionEffect {
        let Some((is_multiselect, auto_advance_enabled)) = self.current().map(|current| {
            let is_multiselect = current.question.is_multiselect();
            (
                is_multiselect,
                ask_user_question_auto_advance_enabled(is_multiselect, self.is_last_question()),
            )
        }) else {
            return AskUserQuestionEffect::Noop;
        };

        let Some(editing) = self.editing_state_mut() else {
            return AskUserQuestionEffect::Noop;
        };

        let mut should_auto_advance_after_toggle = false;
        editing.update_current_draft(|draft| {
            if is_multiselect {
                if !draft.selected_option_indices.insert(option_index) {
                    draft.selected_option_indices.remove(&option_index);
                }
                should_auto_advance_after_toggle =
                    auto_advance_enabled && !draft.selected_option_indices.is_empty();
                return;
            }

            if draft.selected_option_indices.contains(&option_index) {
                draft.selected_option_indices.clear();
                draft.other_text = None;
                draft.is_other_input_active = false;
                return;
            }

            draft.selected_option_indices.clear();
            draft.selected_option_indices.insert(option_index);
            draft.other_text = None;
            draft.is_other_input_active = false;
            should_auto_advance_after_toggle = auto_advance_enabled;
        });

        if should_auto_advance_after_toggle {
            AskUserQuestionEffect::ScheduleAutoAdvance
        } else {
            AskUserQuestionEffect::RefreshCurrent
        }
    }

    fn open_other_input(&mut self) -> AskUserQuestionEffect {
        let Some(is_multiselect) = self
            .current()
            .map(|current| current.question.is_multiselect())
        else {
            return AskUserQuestionEffect::Noop;
        };

        let Some(editing) = self.editing_state_mut() else {
            return AskUserQuestionEffect::Noop;
        };

        editing.update_current_draft(|draft| {
            if !is_multiselect {
                draft.selected_option_indices.clear();
            }
            draft.is_other_input_active = true;
        });
        AskUserQuestionEffect::FocusOtherInput
    }

    fn save_other_text(&mut self, text: Option<String>) -> AskUserQuestionEffect {
        let Some(auto_advance_enabled) = self.current().map(|current| {
            ask_user_question_auto_advance_enabled(
                current.question.is_multiselect(),
                self.is_last_question(),
            )
        }) else {
            return AskUserQuestionEffect::Noop;
        };
        let Some(editing) = self.editing_state_mut() else {
            return AskUserQuestionEffect::Noop;
        };

        editing.update_current_draft(|draft| {
            draft.other_text = text;
            draft.is_other_input_active = false;
        });
        if editing
            .current_draft()
            .is_some_and(|draft| draft.other_text.is_some())
        {
            if auto_advance_enabled {
                AskUserQuestionEffect::ScheduleAutoAdvance
            } else {
                AskUserQuestionEffect::RefreshCurrent
            }
        } else {
            AskUserQuestionEffect::RefreshCurrent
        }
    }

    fn navigate_prev(&mut self) -> AskUserQuestionEffect {
        let Some(editing) = self.editing_state_mut() else {
            return AskUserQuestionEffect::Noop;
        };
        if editing.current_question_index == 0 {
            return AskUserQuestionEffect::Noop;
        }

        editing.current_question_index -= 1;
        AskUserQuestionEffect::ShowQuestion
    }

    fn navigate_next(&mut self) -> AskUserQuestionEffect {
        let question_count = self.questions.len();
        let Some(editing) = self.editing_state_mut() else {
            return AskUserQuestionEffect::Noop;
        };
        if editing.is_last_question(question_count) {
            return AskUserQuestionEffect::Noop;
        }

        editing.current_question_index += 1;
        AskUserQuestionEffect::ShowQuestion
    }

    fn submit_answer(
        &mut self,
        highlighted_index: Option<usize>,
        active_other_text: Option<String>,
    ) -> AskUserQuestionEffect {
        let Some((supports_other, option_count)) = self.current().map(|current| {
            (
                current.question.supports_other(),
                current
                    .question
                    .multiple_choice_options()
                    .map_or(0, |options| options.len()),
            )
        }) else {
            return AskUserQuestionEffect::Noop;
        };

        if supports_other && highlighted_index == Some(option_count) {
            return self.open_other_input();
        }

        if let Some(option_index) = highlighted_index.filter(|index| *index < option_count) {
            let _ = self.toggle_option(option_index);
            return self.advance_after_answer();
        }

        if self
            .current()
            .and_then(|current| current.draft)
            .is_some_and(|draft| draft.is_other_input_active)
        {
            let _ = self.save_other_text(active_other_text);
        }

        self.advance_after_answer()
    }

    fn advance_after_answer(&mut self) -> AskUserQuestionEffect {
        if self
            .current()
            .and_then(|current| current.draft)
            .is_some_and(QuestionDraft::has_answer)
        {
            AskUserQuestionEffect::ScheduleAutoAdvance
        } else {
            self.confirm()
        }
    }

    fn confirm(&mut self) -> AskUserQuestionEffect {
        let question_count = self.questions.len();
        let drafts = {
            let Some(editing) = self.editing_state_mut() else {
                return AskUserQuestionEffect::Noop;
            };
            if !editing.is_last_question(question_count) {
                editing.current_question_index += 1;
                return AskUserQuestionEffect::ShowQuestion;
            }

            editing.drafts.clone()
        };
        let answers = Self::build_answers(&self.questions, &drafts);

        self.state = AskUserQuestionState::Completed {
            answers: answers.clone(),
        };
        AskUserQuestionEffect::Submit(answers)
    }

    fn skip_all(&mut self) -> AskUserQuestionEffect {
        let drafts = {
            let Some(editing) = self.editing_state_mut() else {
                return AskUserQuestionEffect::Noop;
            };
            for draft in &mut editing.drafts {
                *draft = QuestionDraftState::Unanswered;
            }

            editing.drafts.clone()
        };
        let answers = Self::build_answers(&self.questions, &drafts);

        self.state = AskUserQuestionState::Completed {
            answers: answers.clone(),
        };
        AskUserQuestionEffect::Submit(answers)
    }

    fn build_answers(
        questions: &[AskUserQuestionItem],
        drafts: &[QuestionDraftState],
    ) -> Vec<AskUserQuestionAnswerItem> {
        questions
            .iter()
            .enumerate()
            .map(|(index, question)| Self::build_answer(question, drafts.get(index)))
            .collect_vec()
    }

    fn build_answer(
        question: &AskUserQuestionItem,
        draft: Option<&QuestionDraftState>,
    ) -> AskUserQuestionAnswerItem {
        let Some(QuestionDraftState::Answered(draft)) = draft else {
            return AskUserQuestionAnswerItem::Skipped {
                question_id: question.question_id.clone(),
            };
        };

        let selected_options = match &question.question_type {
            AskUserQuestionType::MultipleChoice { options, .. } => draft
                .selected_option_indices
                .iter()
                .copied()
                .sorted_unstable()
                .filter_map(|index| options.get(index).map(|option| option.label.clone()))
                .collect_vec(),
        };
        let other_text = draft.other_text.clone().unwrap_or_default();

        if selected_options.is_empty() && other_text.is_empty() {
            AskUserQuestionAnswerItem::Skipped {
                question_id: question.question_id.clone(),
            }
        } else {
            AskUserQuestionAnswerItem::Answered {
                question_id: question.question_id.clone(),
                selected_options,
                other_text,
            }
        }
    }
}

fn ask_user_question_auto_advance_enabled(is_multiselect: bool, is_last_question: bool) -> bool {
    is_last_question || !is_multiselect
}

#[cfg(test)]
#[path = "ask_user_question_session_tests.rs"]
mod tests;
