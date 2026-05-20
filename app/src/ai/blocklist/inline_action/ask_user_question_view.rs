use std::{collections::HashSet, sync::Arc, time::Duration};

use ai::agent::{
    action::{AskUserQuestionItem, AskUserQuestionOption, AskUserQuestionType},
    action_result::{AskUserQuestionAnswerItem, AskUserQuestionResult},
};
use itertools::Itertools;
use warp_core::ui::theme::{color::internal_colors, WarpTheme};
use warpui::{
    elements::{
        new_scrollable::SingleAxisConfig, Border, ChildView, Clipped, ClippedScrollStateHandle,
        ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Expanded, Fill, Flex,
        FormattedTextElement, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement,
        Radius, Text, DEFAULT_UI_LINE_HEIGHT_RATIO,
    },
    keymap::{FixedBinding, Keystroke},
    r#async::{SpawnedFutureHandle, Timer},
    ui_components::components::Coords,
    units::Pixels,
    AppContext, Element, Entity, EntityId, FocusContext, ModelHandle, SingletonEntity,
    TypedActionView, View, ViewContext, ViewHandle,
};

use crate::{
    ai::{
        agent::{
            conversation::AIConversationId, icons::yellow_stop_icon, task::TaskId, AIAgentActionId,
            AIAgentActionResult, AIAgentActionResultType,
        },
        blocklist::{
            action_model::{AIActionStatus, BlocklistAIActionEvent, BlocklistAIActionModel},
            block::{
                compact_agent_input,
                number_shortcut_buttons::{
                    self, NumberShortcutButtonBuilder, NumberShortcutButtons,
                    NumberShortcutButtonsConfig,
                },
                view_impl::{
                    render_autonomy_dropdown_setting_speedbump_footer, CONTENT_HORIZONTAL_PADDING,
                    CONTENT_ITEM_VERTICAL_MARGIN,
                },
            },
            inline_action::{
                inline_action_header::{
                    ExpandedConfig, HeaderConfig, InteractionMode,
                    INLINE_ACTION_HEADER_VERTICAL_PADDING, INLINE_ACTION_HORIZONTAL_PADDING,
                },
                inline_action_icons::{self, icon_size},
                requested_action::CTRL_C_KEYSTROKE,
            },
            BlocklistAIHistoryModel,
        },
        execution_profiles::{profiles::AIExecutionProfilesModel, AskUserQuestionPermission},
    },
    terminal::input::message_bar::{
        common::{render_standard_message, standard_message_bar_height, styles},
        Message, MessageItem,
    },
    ui_components::{blended_colors, icons::Icon},
    view_components::dropdown::{Dropdown, DropdownItem},
    view_components::{
        action_button::{ButtonSize, KeystrokeSource, NakedTheme, PrimaryTheme},
        compactible_action_button::CompactibleActionButton,
    },
    Appearance,
};

const ASK_USER_QUESTION_ACTIVE: &str = "AskUserQuestionActive";

pub(crate) const ASK_USER_QUESTION_AUTO_ADVANCE_DELAY: Duration = Duration::from_millis(300);
pub(crate) const ASK_USER_QUESTION_MAX_CONTAINER_HEIGHT: f32 = 320.;
pub(crate) const ASK_USER_QUESTION_OPTION_BUTTON_VERTICAL_SPACING: f32 = 4.;
pub(crate) const ASK_USER_QUESTION_TEXT_TOP_PADDING: f32 = 16.;
pub(crate) const ASK_USER_QUESTION_TEXT_BOTTOM_PADDING: f32 = 8.;
pub(crate) const ASK_USER_QUESTION_OPTIONS_BOTTOM_PADDING: f32 = 16.;

// Assumes single-line labels; wrapped text will be taller but the container
// caps at ASK_USER_QUESTION_MAX_CONTAINER_HEIGHT and scrolls on overflow.
fn estimated_min_height_for_all_options(max_option_count: usize, monospace_font_size: f32) -> f32 {
    (max_option_count as f32 * (monospace_font_size + 16.))
        + (max_option_count.saturating_sub(1) as f32
            * ASK_USER_QUESTION_OPTION_BUTTON_VERTICAL_SPACING)
        + ASK_USER_QUESTION_OPTIONS_BOTTOM_PADDING
}

fn ask_user_question_text_height(appearance: &Appearance, app: &AppContext) -> f32 {
    app.font_cache().line_height(
        appearance.monospace_font_size(),
        appearance.line_height_ratio(),
    ) + ASK_USER_QUESTION_TEXT_TOP_PADDING
        + ASK_USER_QUESTION_TEXT_BOTTOM_PADDING
}

fn ask_user_question_header_height(appearance: &Appearance, app: &AppContext) -> f32 {
    let title_line_height = app.font_cache().line_height(
        appearance.monospace_font_size(),
        appearance.line_height_ratio(),
    );
    title_line_height
        .max(icon_size(app))
        .max(ButtonSize::InlineActionHeader.button_height(appearance, app))
        + (2. * INLINE_ACTION_HEADER_VERTICAL_PADDING)
}

fn ask_user_question_container_height(
    max_option_count: usize,
    appearance: &Appearance,
    has_nav_footer: bool,
    app: &AppContext,
) -> f32 {
    let mut natural_height = ask_user_question_header_height(appearance, app)
        + ask_user_question_text_height(appearance, app)
        + estimated_min_height_for_all_options(max_option_count, appearance.monospace_font_size());
    if has_nav_footer {
        natural_height += standard_message_bar_height(app) + 1.;
    }
    natural_height.min(ASK_USER_QUESTION_MAX_CONTAINER_HEIGHT)
}

fn ask_user_question_auto_advance_enabled(is_multiselect: bool, is_last_question: bool) -> bool {
    is_last_question || !is_multiselect
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    // Scope these shortcuts to the active ask-user-question block so arrow/submit keys don't leak
    // into surrounding views.
    app.register_fixed_bindings([
        FixedBinding::new(
            "left",
            AskUserQuestionViewAction::NavigatePrev,
            id!(AskUserQuestionView::ui_name()) & id!(ASK_USER_QUESTION_ACTIVE),
        ),
        FixedBinding::new(
            "right",
            AskUserQuestionViewAction::NavigateNext,
            id!(AskUserQuestionView::ui_name()) & id!(ASK_USER_QUESTION_ACTIVE),
        ),
        FixedBinding::new(
            "enter",
            AskUserQuestionViewAction::EnterPressed,
            id!(AskUserQuestionView::ui_name()) & id!(ASK_USER_QUESTION_ACTIVE),
        ),
        FixedBinding::new(
            "numpadenter",
            AskUserQuestionViewAction::EnterPressed,
            id!(AskUserQuestionView::ui_name()) & id!(ASK_USER_QUESTION_ACTIVE),
        ),
    ]);
}

/// View-level interactions for the ask-user-question UI (buttons, keyboard, and text input).
#[derive(Clone, Debug)]
pub enum AskUserQuestionViewAction {
    OptionToggled { option_index: usize },
    SelectionConfirmed,
    SkipAll,
    FreeTextSubmitted { text: String },
    OtherSelected,
    NavigateNext,
    NavigatePrev,
    ToggleExpanded,
    EnterPressed,
    SetPermission(AskUserQuestionPermission),
}

/// Emitted when local questionnaire state changes and the parent needs to refresh.
#[derive(Clone, Debug)]
pub enum AskUserQuestionViewEvent {
    Updated,
    /// Fired when the user selects a new permission from the speedbump dropdown.
    SpeedbumpPermissionChanged(AskUserQuestionPermission),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
/// In-progress answer data for a single question while the user is editing.
/// Tracks selected choices plus optional free-text "Other" input.
pub(crate) struct QuestionDraft {
    pub selected_option_indices: HashSet<usize>,
    pub other_text: Option<String>,
    pub is_other_input_active: bool,
}

impl QuestionDraft {
    fn has_answer(&self) -> bool {
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
/// Per-question draft slot used by the questionnaire state machine.
/// `Unanswered` is intentionally distinct from an empty `Answered` draft.
enum QuestionDraftState {
    #[default]
    Unanswered,
    Answered(QuestionDraft),
}

/// Snapshot of the currently visible question and its optional draft.
#[derive(Clone, Copy)]
struct AskUserQuestionCurrent<'a> {
    question: &'a AskUserQuestionItem,
    draft: Option<&'a QuestionDraft>,
}

/// Rendering phase for this questionnaire block.
#[derive(Clone, Copy)]
enum AskUserQuestionPhase<'a> {
    Editing,
    Completed {
        answers: &'a [AskUserQuestionAnswerItem],
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Editing-phase state for the questionnaire.
/// Holds the active question index and one draft slot per question.
struct AskUserQuestionEditingState {
    current_question_index: usize,
    drafts: Vec<QuestionDraftState>,
}

impl AskUserQuestionEditingState {
    // Helpers for reading/updating the active question cursor and per-question draft slots.
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

        // Store unanswered questions as a distinct state instead of an empty draft so later logic
        // can tell the difference between "no answer yet" and "there is answer state to render".
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

#[derive(Clone, Debug, Eq, PartialEq)]
/// Lifecycle state for the questionnaire flow.
/// `Editing` is mutable local draft state; `Completed` is the frozen submitted summary.
enum AskUserQuestionState {
    Editing(AskUserQuestionEditingState),
    Completed {
        answers: Vec<AskUserQuestionAnswerItem>,
    },
}

/// State-machine inputs derived from UI interactions.
#[derive(Clone, Debug, Eq, PartialEq)]
enum AskUserQuestionAction {
    ToggleOption {
        option_index: usize,
    },
    OpenOtherInput,
    SaveOtherText {
        text: Option<String>,
    },
    NavigatePrev,
    NavigateNext,
    PressEnter {
        highlighted_index: Option<usize>,
        active_other_text: Option<String>,
    },
    Confirm,
    SkipAll,
}

/// State-machine outputs that tell the view which follow-up UI work to do.
#[derive(Clone, Debug, Eq, PartialEq)]
enum AskUserQuestionEffect {
    Noop,
    RefreshCurrent,
    FocusOtherInput,
    ShowQuestion,
    ScheduleAutoAdvance,
    Submit(Vec<AskUserQuestionAnswerItem>),
}

/// Derived render state for controls that depend on the active question/draft.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AskUserQuestionViewState {
    show_other_input: bool,
}

fn ask_user_question_view_state(
    current: Option<AskUserQuestionCurrent<'_>>,
) -> AskUserQuestionViewState {
    AskUserQuestionViewState {
        show_other_input: current
            .and_then(|current| current.draft)
            .is_some_and(|draft| draft.is_other_input_active),
    }
}

/// Determines whether question-control rebuilds preserve or reset button selection.
#[derive(Clone, Copy)]
enum AskUserQuestionRebuildMode {
    PreserveSelection,
    ResetSelection,
}

/// Concrete child views used for question interaction.
struct AskUserQuestionInteractiveViews {
    buttons: ViewHandle<NumberShortcutButtons>,
    text_input: Option<ViewHandle<compact_agent_input::CompactAgentInput>>,
}

/// Header state for the collapsed/expanded completion summary.
struct AskUserQuestionCompletionState {
    label: String,
    status_icon: warpui::elements::Icon,
}

/// Local questionnaire state machine used by the view.
/// UI events are translated into actions here, which update internal state and return effects for
/// follow-up view work (focus, refresh, navigation, submit).
struct AskUserQuestionSession {
    questions: Vec<AskUserQuestionItem>,
    state: AskUserQuestionState,
}

/// Owns questionnaire prompts and applies state transitions independently of persisted action
/// status, returning effects for the view to execute.
impl AskUserQuestionSession {
    fn new(mut questions: Vec<AskUserQuestionItem>) -> Self {
        // Put multi-select questions before single-select so the last question
        // can auto-submit after a single option toggle.
        questions.sort_by_key(|q| !q.is_multiselect());
        Self {
            state: AskUserQuestionState::Editing(AskUserQuestionEditingState::new(questions.len())),
            questions,
        }
    }

    fn phase(&self) -> AskUserQuestionPhase<'_> {
        match &self.state {
            AskUserQuestionState::Editing(_) => AskUserQuestionPhase::Editing,
            AskUserQuestionState::Completed { answers } => {
                AskUserQuestionPhase::Completed { answers }
            }
        }
    }

    fn is_editing(&self) -> bool {
        matches!(self.state, AskUserQuestionState::Editing(_))
    }

    fn questions(&self) -> &[AskUserQuestionItem] {
        &self.questions
    }

    fn question_count(&self) -> usize {
        self.questions.len()
    }

    fn has_multiple_questions(&self) -> bool {
        self.question_count() > 1
    }

    fn current(&self) -> Option<AskUserQuestionCurrent<'_>> {
        let AskUserQuestionState::Editing(editing) = &self.state else {
            return None;
        };

        Some(AskUserQuestionCurrent {
            question: self.questions.get(editing.current_question_index())?,
            draft: editing.current_draft(),
        })
    }

    fn current_question_index(&self) -> usize {
        match &self.state {
            AskUserQuestionState::Editing(editing) => editing.current_question_index(),
            AskUserQuestionState::Completed { .. } => 0,
        }
    }

    fn is_last_question(&self) -> bool {
        match &self.state {
            AskUserQuestionState::Editing(editing) => {
                editing.is_last_question(self.questions.len())
            }
            AskUserQuestionState::Completed { .. } => false,
        }
    }

    fn max_option_count(&self) -> usize {
        self.questions
            .iter()
            .map(AskUserQuestionItem::numbered_option_count)
            .max()
            .unwrap_or(1)
    }

    // Centralize all state transitions so the view layer only maps UI events to actions and then
    // applies the returned effect.
    fn apply(&mut self, action: AskUserQuestionAction) -> AskUserQuestionEffect {
        match action {
            AskUserQuestionAction::ToggleOption { option_index } => {
                self.toggle_option(option_index)
            }
            AskUserQuestionAction::OpenOtherInput => self.open_other_input(),
            AskUserQuestionAction::SaveOtherText { text } => self.save_other_text(text),
            AskUserQuestionAction::NavigatePrev => self.navigate_prev(),
            AskUserQuestionAction::NavigateNext => self.navigate_next(),
            AskUserQuestionAction::PressEnter {
                highlighted_index,
                active_other_text,
            } => self.press_enter(highlighted_index, active_other_text),
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
        let Some((is_multi_select, auto_advance_enabled)) = self.current().map(|current| {
            let is_multi_select = current.question.is_multiselect();
            (
                is_multi_select,
                ask_user_question_auto_advance_enabled(is_multi_select, self.is_last_question()),
            )
        }) else {
            return AskUserQuestionEffect::Noop;
        };

        let Some(editing) = self.editing_state_mut() else {
            return AskUserQuestionEffect::Noop;
        };

        let mut should_auto_advance_after_toggle = false;
        editing.update_current_draft(|draft| {
            if is_multi_select {
                // Multiselect behaves like a checklist: toggling one option should not affect any
                // of the other selected options, and only the last question is allowed to auto-advance.
                if !draft.selected_option_indices.insert(option_index) {
                    draft.selected_option_indices.remove(&option_index);
                }
                should_auto_advance_after_toggle =
                    auto_advance_enabled && !draft.selected_option_indices.is_empty();
                return;
            }
            // Single-select behaves like a radio group, except clicking the selected option again
            // clears the answer entirely.
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
        let Some(is_multi_select) = self
            .current()
            .map(|current| current.question.is_multiselect())
        else {
            return AskUserQuestionEffect::Noop;
        };

        let Some(editing) = self.editing_state_mut() else {
            return AskUserQuestionEffect::Noop;
        };

        editing.update_current_draft(|draft| {
            if !is_multi_select {
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

    fn press_enter(
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
            return self.enter_submit_effect();
        }

        if self
            .current()
            .and_then(|current| current.draft)
            .is_some_and(|draft| draft.is_other_input_active)
        {
            let _ = self.save_other_text(active_other_text);
        }

        self.enter_submit_effect()
    }

    fn enter_submit_effect(&mut self) -> AskUserQuestionEffect {
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
        // The executor expects one answer entry per question, so unanswered drafts and drafts that
        // collapse back to "no actual content" are both normalized to Skipped here.
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

/// Stateful inline-action view that renders questionnaire UI and coordinates with the action model.
pub(crate) struct AskUserQuestionView {
    action_model: ModelHandle<BlocklistAIActionModel>,
    conversation_id: AIConversationId,
    action_id: AIAgentActionId,
    session: AskUserQuestionSession,
    buttons: ViewHandle<NumberShortcutButtons>,
    text_input: Option<ViewHandle<compact_agent_input::CompactAgentInput>>,
    options_scroll_state: ClippedScrollStateHandle,
    auto_advance_timer_handle: Option<SpawnedFutureHandle>,
    pending_auto_advance_question_index: Option<usize>,
    is_expanded: bool,
    prev_nav_mouse_state: MouseStateHandle,
    next_nav_mouse_state: MouseStateHandle,
    toggle_mouse_state: MouseStateHandle,
    skip_button: CompactibleActionButton,
    next_button: CompactibleActionButton,
    /// Settings link handle for the Ask-User-Question autonomy speedbump footer,
    /// set by `AIBlock` when the speedbump is seeded for this action.
    speedbump_settings_link_handle: Option<MouseStateHandle>,
    /// Lazily created dropdown for the speedbump footer; owned here so the
    /// view handle (and its event subscription) survives re-renders.
    speedbump_dropdown: Option<ViewHandle<Dropdown<AskUserQuestionViewAction>>>,
}

impl AskUserQuestionView {
    pub fn new(
        action_model: ModelHandle<BlocklistAIActionModel>,
        conversation_id: AIConversationId,
        action_id: AIAgentActionId,
        questions: Vec<AskUserQuestionItem>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let session = AskUserQuestionSession::new(questions);
        let options_scroll_state = ClippedScrollStateHandle::new();
        let AskUserQuestionInteractiveViews {
            buttons,
            text_input,
        } = Self::build_interactive_views(
            session.current(),
            None,
            None,
            options_scroll_state.clone(),
            ctx,
        );
        let skip_button = CompactibleActionButton::new(
            "Skip all".to_string(),
            Some(KeystrokeSource::Fixed(CTRL_C_KEYSTROKE.clone())),
            ButtonSize::InlineActionHeader,
            AskUserQuestionViewAction::SkipAll,
            Icon::X,
            Arc::new(NakedTheme),
            ctx,
        );
        let next_button = CompactibleActionButton::new(
            "Next".to_string(),
            Some(KeystrokeSource::Fixed(
                Keystroke::parse("enter").expect("keystroke should parse"),
            )),
            ButtonSize::InlineActionHeader,
            AskUserQuestionViewAction::SelectionConfirmed,
            Icon::CornerDownLeft,
            Arc::new(PrimaryTheme),
            ctx,
        );

        let view = Self {
            action_model: action_model.clone(),
            conversation_id,
            action_id,
            session,
            buttons,
            text_input,
            options_scroll_state,
            auto_advance_timer_handle: None,
            pending_auto_advance_question_index: None,
            is_expanded: false,
            prev_nav_mouse_state: MouseStateHandle::default(),
            next_nav_mouse_state: MouseStateHandle::default(),
            toggle_mouse_state: MouseStateHandle::default(),
            skip_button,
            next_button,
            speedbump_settings_link_handle: None,
            speedbump_dropdown: None,
        };

        ctx.subscribe_to_model(&action_model, |me, _, event, ctx| {
            if event.action_id() != me.action_id() {
                return;
            }

            if matches!(event, BlocklistAIActionEvent::FinishedAction { .. }) {
                me.abort_auto_advance();
                me.text_input = None;
            }

            ctx.emit(AskUserQuestionViewEvent::Updated);
            ctx.notify();
        });

        view
    }

    pub fn action_id(&self) -> &AIAgentActionId {
        &self.action_id
    }

    pub fn is_editing(&self) -> bool {
        self.session.is_editing()
    }

    /// Sets the settings-link mouse handle used in the speedbump footer.
    /// The footer is only actually drawn during the completed/finished render paths.
    pub fn set_speedbump_settings_link(
        &mut self,
        settings_link_handle: Option<MouseStateHandle>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.speedbump_settings_link_handle = settings_link_handle;
        ctx.notify();
    }

    /// Creates the dropdown view for the speedbump footer. No-op if already initialized.
    pub fn init_speedbump_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        if self.speedbump_dropdown.is_some() {
            return;
        }
        let view = ctx.add_typed_action_view(|ctx| {
            let dropdown_font_size = Appearance::as_ref(ctx).monospace_font_size() - 1.;
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_font_size(dropdown_font_size, ctx);
            dropdown.set_padding(
                Coords {
                    top: 2.,
                    bottom: 2.,
                    left: 8.,
                    right: 8.,
                },
                ctx,
            );
            dropdown.set_vertical_margin(0., ctx);
            dropdown.set_top_bar_height(24., ctx);
            let permissions = [
                AskUserQuestionPermission::Never,
                AskUserQuestionPermission::AskExceptInAutoApprove,
                AskUserQuestionPermission::AlwaysAsk,
            ];
            dropdown.set_items(
                permissions
                    .into_iter()
                    .map(|p| {
                        DropdownItem::new(p.label(), AskUserQuestionViewAction::SetPermission(p))
                    })
                    .collect(),
                ctx,
            );
            dropdown
        });
        self.speedbump_dropdown = Some(view);
    }

    /// Updates the dropdown's selected item to match the active execution profile.
    pub fn refresh_speedbump_dropdown_selection(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(dropdown) = self.speedbump_dropdown.clone() else {
            return;
        };
        let permission = AIExecutionProfilesModel::as_ref(ctx)
            .active_profile(Some(terminal_view_id), ctx)
            .data()
            .ask_user_question;
        dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_name(permission.label(), ctx);
        });
    }

    /// Recover completed/cancelled status even if the live action entry is gone, so restored
    /// conversations still render deterministically.
    fn action_status(&self, app: &AppContext) -> Option<AIActionStatus> {
        let action_model = self.action_model.as_ref(app);
        if let Some(status) = action_model.get_action_status(self.action_id()) {
            return Some(status);
        }

        let should_restore_as_cancelled = BlocklistAIHistoryModel::as_ref(app)
            .conversation(&self.conversation_id)
            .is_some_and(|conversation| !conversation.status().is_in_progress())
            && !action_model.has_unfinished_actions_for_conversation(self.conversation_id);

        should_restore_as_cancelled.then(|| {
            AIActionStatus::Finished(Arc::new(AIAgentActionResult {
                result: AIAgentActionResultType::AskUserQuestion(AskUserQuestionResult::Cancelled),
                task_id: TaskId::new("fake-id".to_owned()),
                id: self.action_id.clone(),
            }))
        })
    }

    /// True when the action model reports this inline action is blocked waiting on user answers.
    fn is_waiting_on_user_answers(&self, app: &AppContext) -> bool {
        self.action_status(app)
            .is_some_and(|status| status.is_blocked())
    }

    pub fn should_render_inline(&self, app: &AppContext) -> bool {
        matches!(self.session.phase(), AskUserQuestionPhase::Completed { .. })
            || self.is_waiting_on_user_answers(app)
            || matches!(self.action_status(app), Some(AIActionStatus::Finished(_)))
    }

    pub fn matches_action(
        &self,
        action_id: &AIAgentActionId,
        questions: &[AskUserQuestionItem],
    ) -> bool {
        self.action_id() == action_id && self.session.questions() == questions
    }

    /// Rebuild numbered options and inline input from the current session state whenever the
    /// visible question or draft mode changes.
    fn build_interactive_views(
        current: Option<AskUserQuestionCurrent<'_>>,
        existing_text_input: Option<ViewHandle<compact_agent_input::CompactAgentInput>>,
        selected_button_index: Option<usize>,
        options_scroll_state: ClippedScrollStateHandle,
        ctx: &mut ViewContext<Self>,
    ) -> AskUserQuestionInteractiveViews {
        let view_state = ask_user_question_view_state(current);
        let text_input = if view_state.show_other_input {
            existing_text_input.or_else(|| {
                Some(Self::create_text_input(
                    current
                        .and_then(|current| current.draft)
                        .and_then(|draft| draft.other_text.as_deref()),
                    ctx,
                ))
            })
        } else {
            None
        };
        let buttons = Self::build_question_buttons(current, text_input.as_ref());
        AskUserQuestionInteractiveViews {
            buttons: ctx.add_typed_action_view(|ctx| {
                NumberShortcutButtons::new_with_config(
                    buttons,
                    selected_button_index,
                    NumberShortcutButtonsConfig::new()
                        .with_keyboard_navigation()
                        .with_enter_to_activate(false)
                        .with_scroll_state(options_scroll_state),
                    ctx,
                )
            }),
            text_input,
        }
    }

    fn build_question_buttons(
        current: Option<AskUserQuestionCurrent<'_>>,
        other_text_input: Option<&ViewHandle<compact_agent_input::CompactAgentInput>>,
    ) -> Vec<NumberShortcutButtonBuilder> {
        let Some(current) = current else {
            return Vec::new();
        };

        match &current.question.question_type {
            AskUserQuestionType::MultipleChoice {
                options,
                supports_other,
                ..
            } => Self::build_multiple_choice_question_buttons(
                options,
                *supports_other,
                current.draft,
                other_text_input,
            ),
        }
    }

    fn build_multiple_choice_question_buttons(
        options: &[AskUserQuestionOption],
        supports_other: bool,
        draft: Option<&QuestionDraft>,
        other_text_input: Option<&ViewHandle<compact_agent_input::CompactAgentInput>>,
    ) -> Vec<NumberShortcutButtonBuilder> {
        let mut buttons = options
            .iter()
            .enumerate()
            .map(|(index, option)| {
                let is_checked =
                    draft.is_some_and(|draft| draft.selected_option_indices.contains(&index));
                number_shortcut_buttons::numbered_shortcut_button(
                    index + 1,
                    option.label.clone(),
                    is_checked,
                    option.recommended,
                    true,
                    MouseStateHandle::default(),
                    AskUserQuestionViewAction::OptionToggled {
                        option_index: index,
                    },
                )
            })
            .collect_vec();

        if let Some(other_button) = Self::build_other_question_button(
            options.len() + 1,
            supports_other,
            draft,
            other_text_input,
        ) {
            buttons.push(other_button);
        }

        buttons
    }

    fn build_other_question_button(
        number: usize,
        supports_other: bool,
        draft: Option<&QuestionDraft>,
        other_text_input: Option<&ViewHandle<compact_agent_input::CompactAgentInput>>,
    ) -> Option<NumberShortcutButtonBuilder> {
        if !supports_other {
            return None;
        }

        if draft.is_some_and(|draft| draft.is_other_input_active) {
            if let Some(input) = other_text_input {
                return Some(number_shortcut_buttons::inline_input_shortcut_button(
                    number,
                    input.clone(),
                    MouseStateHandle::default(),
                ));
            }
        }

        let accepted_text = draft
            .and_then(|draft| draft.other_text.as_deref())
            .filter(|text| !text.is_empty())
            .map(str::to_owned);

        Some(number_shortcut_buttons::numbered_shortcut_button(
            number,
            accepted_text
                .clone()
                .unwrap_or_else(|| "Other...".to_string()),
            accepted_text.is_some(),
            false,
            true,
            MouseStateHandle::default(),
            AskUserQuestionViewAction::OtherSelected,
        ))
    }

    fn create_text_input(
        initial_text: Option<&str>,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<compact_agent_input::CompactAgentInput> {
        let initial_text = initial_text.map(String::from);
        let input = ctx.add_view(move |ctx| {
            let input = compact_agent_input::CompactAgentInput::new(ctx);
            input.set_placeholder_text("Type your answer and press Enter", ctx);
            if let Some(initial_text) = initial_text.as_deref() {
                input.set_text(initial_text, ctx);
            }
            input
        });
        ctx.subscribe_to_view(&input, |_, text_input, event, ctx| match event {
            compact_agent_input::CompactAgentInputEvent::Submit(text) => {
                ctx.dispatch_typed_action_deferred(AskUserQuestionViewAction::FreeTextSubmitted {
                    text: text.clone(),
                });
            }
            compact_agent_input::CompactAgentInputEvent::Escape => {
                // Esc only dismisses the inline "Other" editor when it is still empty; once the
                // user has typed something, we keep the input open so text is not discarded
                // implicitly.
                let has_text = text_input.read(ctx, |input, ctx| {
                    input.editor().read(ctx, |editor, ctx| {
                        !editor.buffer_text(ctx).trim().is_empty()
                    })
                });
                if !has_text {
                    ctx.dispatch_typed_action_deferred(
                        AskUserQuestionViewAction::FreeTextSubmitted {
                            text: String::new(),
                        },
                    );
                }
            }
        });
        input
    }

    /// Keep button/input views synchronized with session state after each transition. Navigation
    /// resets selection, while in-question updates preserve it for keyboard continuity.
    fn rebuild_current_question(
        &mut self,
        rebuild_mode: AskUserQuestionRebuildMode,
        ctx: &mut ViewContext<Self>,
    ) {
        let selected_button_index = match rebuild_mode {
            AskUserQuestionRebuildMode::PreserveSelection => self
                .buttons
                .read(ctx, |buttons, _| buttons.selected_button_index()),
            AskUserQuestionRebuildMode::ResetSelection => None,
        };
        let current = self.session.current();
        let AskUserQuestionInteractiveViews {
            buttons,
            text_input,
        } = Self::build_interactive_views(
            current,
            self.text_input.clone(),
            selected_button_index,
            self.options_scroll_state.clone(),
            ctx,
        );

        self.buttons = buttons;
        self.text_input = text_input;
    }

    fn abort_auto_advance(&mut self) {
        self.pending_auto_advance_question_index = None;
        if let Some(handle) = self.auto_advance_timer_handle.take() {
            handle.abort();
        }
    }

    fn schedule_auto_advance_for_current_question(&mut self, ctx: &mut ViewContext<Self>) {
        let question_index = self.session.current_question_index();
        self.abort_auto_advance();
        self.pending_auto_advance_question_index = Some(question_index);
        // Single-select answers advance automatically after a short delay, but only if the user is
        // still on the same question when the timer fires. Any navigation or edit clears the
        // pending question index and turns the timer into a no-op.
        let handle = ctx.spawn(
            async move {
                Timer::after(ASK_USER_QUESTION_AUTO_ADVANCE_DELAY).await;
                question_index
            },
            |me, question_index, ctx| {
                me.auto_advance_timer_handle = None;
                if me.pending_auto_advance_question_index == Some(question_index)
                    && me.session.current_question_index() == question_index
                {
                    let effect = me.session.apply(AskUserQuestionAction::Confirm);
                    me.handle_session_effect(effect, ctx);
                    ctx.emit(AskUserQuestionViewEvent::Updated);
                    ctx.notify();
                }
            },
        );
        self.auto_advance_timer_handle = Some(handle);
    }

    fn submit_answers(
        &mut self,
        answers: Vec<AskUserQuestionAnswerItem>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.abort_auto_advance();
        let action_id = self.action_id().clone();
        let conversation_id = self.conversation_id;
        // Completing the executor only stages the result; execute_action drives the action model
        // to publish final status/events for this inline block.
        self.action_model.update(ctx, |action_model, ctx| {
            action_model
                .ask_user_question_executor(ctx)
                .update(ctx, |executor, _| {
                    executor.complete(answers.clone());
                });
            action_model.execute_action(&action_id, conversation_id, ctx);
        });
    }

    fn read_active_other_text(&self, ctx: &AppContext) -> Option<String> {
        self.text_input
            .as_ref()
            .map(|text_input| {
                text_input.read(ctx, |input, ctx| {
                    input
                        .editor()
                        .read(ctx, |editor, ctx| editor.buffer_text(ctx))
                        .trim()
                        .to_owned()
                })
            })
            .filter(|text| !text.is_empty())
    }

    fn commit_active_other_text(&mut self, ctx: &mut ViewContext<Self>) {
        if self.text_input.is_none() {
            return;
        }

        let text = self.read_active_other_text(ctx);
        let _ = self
            .session
            .apply(AskUserQuestionAction::SaveOtherText { text });
        self.rebuild_current_question(AskUserQuestionRebuildMode::PreserveSelection, ctx);
    }

    /// Translate state-machine effects into concrete UI work (refresh/focus/scroll/submit).
    fn handle_session_effect(
        &mut self,
        effect: AskUserQuestionEffect,
        ctx: &mut ViewContext<Self>,
    ) {
        match effect {
            AskUserQuestionEffect::Noop => {}
            AskUserQuestionEffect::RefreshCurrent => {
                self.rebuild_current_question(AskUserQuestionRebuildMode::PreserveSelection, ctx);
            }
            AskUserQuestionEffect::FocusOtherInput => {
                self.rebuild_current_question(AskUserQuestionRebuildMode::PreserveSelection, ctx);
                if let Some(text_input) = &self.text_input {
                    ctx.focus(text_input);
                }
            }
            AskUserQuestionEffect::ShowQuestion => {
                self.rebuild_current_question(AskUserQuestionRebuildMode::ResetSelection, ctx);
                self.options_scroll_state.scroll_to(Pixels::zero());
                ctx.focus(&self.buttons);
            }
            AskUserQuestionEffect::ScheduleAutoAdvance => {
                self.rebuild_current_question(AskUserQuestionRebuildMode::PreserveSelection, ctx);
                self.schedule_auto_advance_for_current_question(ctx);
            }
            AskUserQuestionEffect::Submit(answers) => {
                self.text_input = None;
                self.submit_answers(answers, ctx);
            }
        }
    }

    fn render_active(&self, appearance: &Appearance, app: &AppContext) -> Option<Box<dyn Element>> {
        let theme = appearance.theme();
        let current = self.session.current()?;
        let mut question_text = current.question.question.clone();
        if current.question.is_multiselect() {
            question_text.push_str(" (select all that apply)");
        }
        let has_nav_footer = self.session.has_multiple_questions();
        let container_height = ask_user_question_container_height(
            self.session.max_option_count(),
            appearance,
            has_nav_footer,
            app,
        );

        let mut content = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        let mut header_right = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        header_right.add_child(ChildView::new(self.skip_button.expanded_button()).finish());
        header_right.add_child(
            Container::new(ChildView::new(self.next_button.expanded_button()).finish())
                .with_margin_left(8.)
                .finish(),
        );
        content.add_child(
            HeaderConfig::new("Agent questions", app)
                .with_icon(yellow_stop_icon(appearance))
                .with_corner_radius_override(CornerRadius::with_top(Radius::Pixels(8.)))
                .render_header(app, Some(header_right.finish())),
        );
        content.add_child(
            Expanded::new(
                1.,
                self.render_question_body(&question_text, appearance, theme),
            )
            .finish(),
        );

        if has_nav_footer {
            content.add_child(self.render_nav_footer(appearance, theme, app));
        }

        let border_color = blended_colors::neutral_4(theme);
        Some(
            wrap_with_content_item_spacing(
                ConstrainedBox::new(content.finish())
                    .with_height(container_height)
                    .finish(),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_background_color(theme.background().into_solid())
            .with_border(Border::all(1.).with_border_fill(border_color))
            .finish(),
        )
    }

    fn render_unavailable(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        wrap_with_agent_output_item_spacing(
            HeaderConfig::new("Questions unavailable".to_string(), app)
                .with_icon(inline_action_icons::reverted_icon(appearance))
                .render(app),
            app,
        )
        .finish()
    }

    fn render_completed_answers(
        &self,
        answers: &[AskUserQuestionAnswerItem],
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let AskUserQuestionCompletionState { label, status_icon } =
            ask_user_question_completion_state(answers, appearance);
        self.render_completed(
            self.session.questions(),
            Some(answers),
            label,
            status_icon,
            appearance,
            app,
        )
    }

    fn render_finished(
        &self,
        ask_result: &AskUserQuestionResult,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let (answers, label, status_icon) = match ask_result {
            AskUserQuestionResult::Success { answers } => {
                let AskUserQuestionCompletionState { label, status_icon } =
                    ask_user_question_completion_state(answers, appearance);
                (Some(answers.as_slice()), label, status_icon)
            }
            AskUserQuestionResult::Error(_) | AskUserQuestionResult::Cancelled => (
                None,
                "Questions skipped".to_string(),
                inline_action_icons::reverted_icon(appearance),
            ),
            AskUserQuestionResult::SkippedByAutoApprove { .. } => (
                None,
                "Questions skipped due to auto-approve".to_string(),
                inline_action_icons::reverted_icon(appearance),
            ),
        };

        self.render_completed(
            self.session.questions(),
            answers,
            label,
            status_icon,
            appearance,
            app,
        )
    }

    fn render_completed(
        &self,
        questions: &[AskUserQuestionItem],
        answers: Option<&[AskUserQuestionAnswerItem]>,
        label: String,
        status_icon: warpui::elements::Icon,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let speedbump_footer = self
            .speedbump_settings_link_handle
            .clone()
            .filter(|_| self.speedbump_dropdown.is_some())
            .and_then(|handle| self.render_speedbump_footer(handle, appearance, app));
        let has_speedbump_footer = speedbump_footer.is_some();

        let mut header_config = HeaderConfig::new(label, app)
            .with_icon(status_icon)
            .with_interaction_mode(InteractionMode::ManuallyExpandable(
                ExpandedConfig::new(self.is_expanded, self.toggle_mouse_state.clone())
                    .with_toggle_callback(|ctx| {
                        ctx.dispatch_typed_action(AskUserQuestionViewAction::ToggleExpanded);
                    }),
            ));
        if has_speedbump_footer {
            header_config = header_config
                .with_corner_radius_override(CornerRadius::with_top(Radius::Pixels(8.)));
        }
        let header = header_config.render(app);

        // Collapsed state: just the header — but still draw the speedbump footer if it's
        // present, since the autonomy nudge must stay visible even if the user collapses
        // their answers.
        if !self.is_expanded {
            if let Some(footer) = speedbump_footer {
                let mut wrapper =
                    Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
                wrapper.add_child(header);
                wrapper.add_child(footer);
                return wrap_with_agent_output_item_spacing(wrapper.finish(), app).finish();
            }
            return wrap_with_agent_output_item_spacing(header, app).finish();
        }

        let mut wrapper = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        wrapper.add_child(header);
        wrapper.add_child(render_answers(
            questions,
            answers,
            appearance,
            has_speedbump_footer,
        ));
        if let Some(footer) = speedbump_footer {
            wrapper.add_child(footer);
            return wrap_with_agent_output_item_spacing(wrapper.finish(), app).finish();
        }
        wrap_with_agent_output_item_spacing(wrapper.finish(), app)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish()
    }

    fn render_speedbump_footer(
        &self,
        settings_link_handle: MouseStateHandle,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let theme = appearance.theme();
        let dropdown = self.speedbump_dropdown.as_ref()?;
        let row = render_autonomy_dropdown_setting_speedbump_footer(
            "Allow the agent to ask questions:",
            dropdown,
            settings_link_handle,
            app,
        );
        Some(
            Container::new(row)
                .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
                .with_vertical_padding(4.)
                .with_background(theme.surface_1())
                .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
                .finish(),
        )
    }

    fn render_question_text(
        question_text: &str,
        appearance: &Appearance,
        theme: &WarpTheme,
    ) -> Box<dyn Element> {
        let text_color = theme.foreground().into();
        Container::new(render_text_with_markdown_support(
            question_text,
            appearance.monospace_font_size(),
            text_color,
            appearance,
        ))
        .with_padding_top(ASK_USER_QUESTION_TEXT_TOP_PADDING)
        .with_padding_bottom(ASK_USER_QUESTION_TEXT_BOTTOM_PADDING)
        .finish()
    }

    fn render_options_list(&self) -> Box<dyn Element> {
        Container::new(ChildView::new(&self.buttons).finish())
            .with_padding_bottom(ASK_USER_QUESTION_OPTIONS_BOTTOM_PADDING)
            .finish()
    }

    fn render_question_body(
        &self,
        question_text: &str,
        appearance: &Appearance,
        theme: &WarpTheme,
    ) -> Box<dyn Element> {
        let body = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(Self::render_question_text(question_text, appearance, theme))
            .with_child(self.render_options_list())
            .finish();

        let scrollable = warpui::elements::NewScrollable::vertical(
            SingleAxisConfig::Clipped {
                handle: self.options_scroll_state.clone(),
                child: body,
            },
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            Fill::None,
        )
        .finish();

        Container::new(Clipped::new(scrollable).finish())
            .with_margin_left(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_margin_right(12.)
            .finish()
    }

    fn render_nav_footer(
        &self,
        appearance: &Appearance,
        theme: &WarpTheme,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let counter = format!(
            "{}/{}",
            self.session.current_question_index() + 1,
            self.session.question_count()
        );
        let counter_text = Text::new(counter, appearance.ui_font_family(), styles::font_size(app))
            .with_color(theme.foreground().into())
            .finish();

        let left_key = Keystroke::parse("left").expect("keystroke should parse");
        let right_key = Keystroke::parse("right").expect("keystroke should parse");

        let nav_message = Message::new(vec![
            MessageItem::clickable(
                vec![MessageItem::keystroke(left_key), MessageItem::text("prev")],
                |ctx| {
                    ctx.dispatch_typed_action(AskUserQuestionViewAction::NavigatePrev);
                },
                self.prev_nav_mouse_state.clone(),
            ),
            MessageItem::text(" / "),
            MessageItem::clickable(
                vec![MessageItem::keystroke(right_key), MessageItem::text("next")],
                |ctx| {
                    ctx.dispatch_typed_action(AskUserQuestionViewAction::NavigateNext);
                },
                self.next_nav_mouse_state.clone(),
            ),
        ]);

        let footer_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(counter_text)
            .with_child(
                Container::new(render_standard_message(nav_message, app))
                    .with_margin_left(8.)
                    .finish(),
            )
            .finish();

        Container::new(footer_row)
            .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_vertical_padding(8.)
            .with_border(Border::top(1.).with_border_fill(blended_colors::neutral_4(theme)))
            .finish()
    }
}

impl Entity for AskUserQuestionView {
    type Event = AskUserQuestionViewEvent;
}

impl View for AskUserQuestionView {
    fn ui_name() -> &'static str {
        "AskUserQuestionView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let action_status = self.action_status(app);
        // While the user is answering, render from the local session draft. Once the action
        // finishes, render from the persisted action result so reopened views reflect what was
        // actually submitted.
        if let Some(AIActionStatus::Finished(result)) = action_status.as_ref() {
            if let AIAgentActionResultType::AskUserQuestion(ask_result) = &result.result {
                return self.render_finished(ask_result, appearance, app);
            }

            return Flex::column().finish();
        }

        match self.session.phase() {
            AskUserQuestionPhase::Completed { answers } => {
                self.render_completed_answers(answers, appearance, app)
            }
            AskUserQuestionPhase::Editing if !self.is_waiting_on_user_answers(app) => {
                Flex::column().finish()
            }
            AskUserQuestionPhase::Editing => self
                .render_active(appearance, app)
                .unwrap_or_else(|| self.render_unavailable(appearance, app)),
        }
    }

    fn keymap_context(&self, app: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();
        // These context flags are what activate the fixed bindings registered in init().
        if matches!(self.session.phase(), AskUserQuestionPhase::Editing)
            && self.is_waiting_on_user_answers(app)
            && self.session.current().is_some()
        {
            context.set.insert(ASK_USER_QUESTION_ACTIVE);
        }
        context
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if !focus_ctx.is_self_focused()
            || !self.is_waiting_on_user_answers(ctx)
            || self.session.current().is_none()
        {
            return;
        }
        if let Some(text_input) = &self.text_input {
            ctx.focus(text_input);
        } else if self.session.is_editing() {
            ctx.focus(&self.buttons);
        }
    }
}

impl TypedActionView for AskUserQuestionView {
    type Action = AskUserQuestionViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        // Editing actions only run while this inline block is still the active user blocker. The
        // completion summary stays interactive via ToggleExpanded and SetPermission after completion.
        if !matches!(
            action,
            AskUserQuestionViewAction::ToggleExpanded | AskUserQuestionViewAction::SetPermission(_)
        ) && (!self.session.is_editing()
            || !self.is_waiting_on_user_answers(ctx)
            || self.session.current().is_none())
        {
            return;
        }

        match action {
            AskUserQuestionViewAction::OptionToggled { option_index } => {
                self.abort_auto_advance();
                let effect = self.session.apply(AskUserQuestionAction::ToggleOption {
                    option_index: *option_index,
                });
                self.handle_session_effect(effect, ctx);
            }
            AskUserQuestionViewAction::SelectionConfirmed => {
                self.abort_auto_advance();
                self.commit_active_other_text(ctx);
                let effect = self.session.apply(AskUserQuestionAction::Confirm);
                self.handle_session_effect(effect, ctx);
            }
            AskUserQuestionViewAction::SkipAll => {
                self.abort_auto_advance();
                let effect = self.session.apply(AskUserQuestionAction::SkipAll);
                self.handle_session_effect(effect, ctx);
            }
            AskUserQuestionViewAction::FreeTextSubmitted { text } => {
                self.abort_auto_advance();
                let effect = self.session.apply(AskUserQuestionAction::SaveOtherText {
                    text: (!text.trim().is_empty()).then(|| text.trim().to_owned()),
                });
                self.handle_session_effect(effect, ctx);
            }
            AskUserQuestionViewAction::OtherSelected => {
                self.abort_auto_advance();
                let effect = self.session.apply(AskUserQuestionAction::OpenOtherInput);
                self.handle_session_effect(effect, ctx);
            }
            AskUserQuestionViewAction::NavigateNext => {
                self.abort_auto_advance();
                self.commit_active_other_text(ctx);
                let effect = self.session.apply(AskUserQuestionAction::NavigateNext);
                self.handle_session_effect(effect, ctx);
            }
            AskUserQuestionViewAction::NavigatePrev => {
                self.abort_auto_advance();
                self.commit_active_other_text(ctx);
                let effect = self.session.apply(AskUserQuestionAction::NavigatePrev);
                self.handle_session_effect(effect, ctx);
            }
            AskUserQuestionViewAction::ToggleExpanded => {
                self.is_expanded = !self.is_expanded;
            }
            AskUserQuestionViewAction::SetPermission(permission) => {
                ctx.emit(AskUserQuestionViewEvent::SpeedbumpPermissionChanged(
                    *permission,
                ));
            }
            AskUserQuestionViewAction::EnterPressed => {
                self.abort_auto_advance();
                let highlighted_index = self
                    .buttons
                    .read(ctx, |buttons, _| buttons.selected_button_index());
                let active_other_text = self.read_active_other_text(ctx);
                let effect = self.session.apply(AskUserQuestionAction::PressEnter {
                    highlighted_index,
                    active_other_text,
                });
                self.handle_session_effect(effect, ctx);
            }
        }

        ctx.emit(AskUserQuestionViewEvent::Updated);
        ctx.notify();
    }
}

fn ask_user_question_completion_state(
    answers: &[AskUserQuestionAnswerItem],
    appearance: &Appearance,
) -> AskUserQuestionCompletionState {
    let answered_count = answers.iter().filter(|answer| !answer.is_skipped()).count();
    let total = answers.len();

    if answered_count == 0 {
        AskUserQuestionCompletionState {
            label: "Questions skipped".to_string(),
            status_icon: inline_action_icons::reverted_icon(appearance),
        }
    } else {
        let label = if answered_count == total {
            if total == 1 {
                "Answered question".to_string()
            } else {
                format!("Answered all {total} questions")
            }
        } else {
            format!(
                "Answered {answered_count} of {total} question{}",
                if total == 1 { "" } else { "s" }
            )
        };
        AskUserQuestionCompletionState {
            label,
            status_icon: inline_action_icons::green_check_icon(appearance),
        }
    }
}

fn render_answers(
    questions: &[AskUserQuestionItem],
    answers: Option<&[AskUserQuestionAnswerItem]>,
    appearance: &Appearance,
    flatten_bottom: bool,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let font_size = appearance.monospace_font_size();
    let text_color = blended_colors::text_main(theme, theme.surface_2());
    let muted_color = internal_colors::neutral_5(theme);

    let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    for (index, question) in questions.iter().enumerate() {
        let answer = answers.and_then(|answers| answers.get(index));
        let question_text = format!("Q: {}", question.question);
        let question_label =
            render_text_with_markdown_support(&question_text, font_size, text_color, appearance);
        let answer_text = format!(
            "A: {}",
            answer
                .map(AskUserQuestionAnswerItem::display_text)
                .unwrap_or_else(|| "Skipped".to_string())
        );
        let answer_label =
            render_text_with_markdown_support(&answer_text, font_size, muted_color, appearance);

        let mut item = Flex::column();
        item.add_child(question_label);
        item.add_child(Container::new(answer_label).with_margin_top(2.).finish());

        let mut item_container = Container::new(item.finish())
            .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_vertical_padding(8.);
        if index > 0 {
            item_container = item_container
                .with_border(Border::top(1.).with_border_fill(blended_colors::neutral_2(theme)));
        }
        content.add_child(item_container.finish());
    }

    let mut container = Container::new(content.finish()).with_background(theme.surface_2());
    if !flatten_bottom {
        container = container.with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)));
    }
    container.finish()
}

fn wrap_with_content_item_spacing(element: Box<dyn Element>) -> Container {
    Container::new(element)
        .with_margin_left(CONTENT_HORIZONTAL_PADDING)
        .with_margin_right(CONTENT_HORIZONTAL_PADDING)
        .with_margin_bottom(CONTENT_ITEM_VERTICAL_MARGIN)
}

fn wrap_with_agent_output_item_spacing(element: Box<dyn Element>, app: &AppContext) -> Container {
    let left_margin = CONTENT_HORIZONTAL_PADDING + icon_size(app) + 16.;
    Container::new(element)
        .with_margin_left(left_margin)
        .with_margin_right(CONTENT_HORIZONTAL_PADDING)
        .with_margin_bottom(CONTENT_ITEM_VERTICAL_MARGIN)
}

/// Renders text as markdown using `FormattedTextElement`, falling back to plain `Text` if
/// markdown parsing fails.
pub(crate) fn render_text_with_markdown_support(
    text: &str,
    font_size: f32,
    text_color: pathfinder_color::ColorU,
    appearance: &Appearance,
) -> Box<dyn Element> {
    if let Ok(formatted_text) = markdown_parser::parse_markdown(text) {
        FormattedTextElement::new(
            formatted_text,
            font_size,
            appearance.ui_font_family(),
            appearance.monospace_font_family(),
            text_color,
            Default::default(),
        )
        .with_line_height_ratio(DEFAULT_UI_LINE_HEIGHT_RATIO)
        .finish()
    } else {
        Text::new(text.to_string(), appearance.ui_font_family(), font_size)
            .soft_wrap(true)
            .with_color(text_color)
            .finish()
    }
}

#[cfg(test)]
#[path = "ask_user_question_view_tests.rs"]
mod tests;
