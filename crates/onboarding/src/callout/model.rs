use crate::telemetry::OnboardingEvent;
use crate::OnboardingIntention;
use warp_core::send_telemetry_from_ctx;
use warpui::{Entity, ModelContext};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinalState {
    /// User submitted the agent query (legacy flow)
    Submit,
    /// User skipped the callout (legacy flow or skip initialization)
    Skip,
    /// User finished the callout without submitting
    Finish,
    /// User chose to initialize the project (AgentModality with project)
    Initialize,
    /// User chose to go back to terminal (AgentModality without project)
    BackToTerminal,
}

impl std::fmt::Display for FinalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FinalState::Submit => write!(f, "submitted"),
            FinalState::Skip => write!(f, "skipped"),
            FinalState::Finish => write!(f, "finished"),
            FinalState::Initialize => write!(f, "initialize"),
            FinalState::BackToTerminal => write!(f, "back_to_terminal"),
        }
    }
}

/// Prompt information for the onboarding callout
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnboardingQuery {
    /// A terminal command that should be executed in shell mode
    TerminalCommand(String),
    /// An agent prompt that should be executed in agent mode
    AgentPrompt(String),
    /// No prompt (empty state)
    None,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum OnboardingCalloutModelEvent {
    StateUpdated,
    Completed(FinalState),
    EnterAgentModality,
    /// Emitted when the user toggles the natural language detection checkbox.
    NaturalLanguageDetectionToggled(bool),
}

/// State for the UniversalInput onboarding flow (non-AgentModality).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum UniversalInputCalloutState {
    #[default]
    Off,
    MeetInput,
    TalkToAgent,
    Complete(FinalState),
}

/// State for the AgentModality onboarding flow.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum AgentModalityCalloutState {
    #[default]
    Off,
    /// Step 1: terminal input with natural language support.
    TerminalMode,
    /// Step 2: "Agent Mode" (Agent intention only).
    AgentMode,
    /// Terminal state
    Complete(FinalState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OnboardingCalloutState {
    UniversalInput(UniversalInputCalloutState),
    AgentModality(AgentModalityCalloutState),
}

pub(super) struct OnboardingCalloutModel {
    state: OnboardingCalloutState,
    intention: OnboardingIntention,
    has_project: bool,
    /// The initial value of natural language detection when onboarding started.
    /// Used to determine which callout variant to show.
    initial_natural_language_detection_enabled: bool,
    /// The current value of natural language detection (may change via checkbox toggle).
    natural_language_detection_enabled: bool,
}

impl OnboardingCalloutModel {
    /// Create a new model for UniversalInput onboarding flow.
    pub fn new_universal_input(
        has_project: bool,
        initial_natural_language_detection_enabled: bool,
    ) -> Self {
        Self {
            state: OnboardingCalloutState::UniversalInput(UniversalInputCalloutState::default()),
            intention: OnboardingIntention::AgentDrivenDevelopment,
            has_project,
            initial_natural_language_detection_enabled,
            natural_language_detection_enabled: initial_natural_language_detection_enabled,
        }
    }

    /// Create a new model for AgentModality onboarding flow.
    pub fn new_agent_modality(
        has_project: bool,
        intention: OnboardingIntention,
        initial_natural_language_detection_enabled: bool,
    ) -> Self {
        Self {
            state: OnboardingCalloutState::AgentModality(AgentModalityCalloutState::default()),
            intention,
            has_project,
            initial_natural_language_detection_enabled,
            natural_language_detection_enabled: initial_natural_language_detection_enabled,
        }
    }

    pub fn has_project(&self) -> bool {
        self.has_project
    }

    pub fn intention(&self) -> OnboardingIntention {
        self.intention
    }

    pub fn initial_natural_language_detection_enabled(&self) -> bool {
        self.initial_natural_language_detection_enabled
    }

    pub fn natural_language_detection_enabled(&self) -> bool {
        self.natural_language_detection_enabled
    }

    pub fn toggle_natural_language_detection(&mut self, ctx: &mut ModelContext<Self>) {
        self.natural_language_detection_enabled = !self.natural_language_detection_enabled;
        ctx.emit(
            OnboardingCalloutModelEvent::NaturalLanguageDetectionToggled(
                self.natural_language_detection_enabled,
            ),
        );
        ctx.emit(OnboardingCalloutModelEvent::StateUpdated);
        ctx.notify();
    }

    pub fn next(&mut self, ctx: &mut ModelContext<Self>) {
        send_telemetry_from_ctx!(OnboardingEvent::CalloutNext, ctx);
        match &self.state {
            OnboardingCalloutState::UniversalInput(universal_input_state) => {
                self.next_universal_input(*universal_input_state, ctx);
            }
            OnboardingCalloutState::AgentModality(modality_state) => {
                self.next_agent_modality(*modality_state, ctx);
            }
        }
    }

    fn next_universal_input(
        &mut self,
        state: UniversalInputCalloutState,
        ctx: &mut ModelContext<Self>,
    ) {
        let next_state = match state {
            UniversalInputCalloutState::Off => Some(UniversalInputCalloutState::MeetInput),
            UniversalInputCalloutState::MeetInput => Some(UniversalInputCalloutState::TalkToAgent),
            UniversalInputCalloutState::TalkToAgent => {
                Some(UniversalInputCalloutState::Complete(FinalState::Submit))
            }
            UniversalInputCalloutState::Complete(_) => None,
        };
        if let Some(next_state) = next_state {
            self.set_state(OnboardingCalloutState::UniversalInput(next_state), ctx);
        }
    }

    fn next_agent_modality(
        &mut self,
        state: AgentModalityCalloutState,
        ctx: &mut ModelContext<Self>,
    ) {
        let (next_state, emit_enter_agent_modality) = match state {
            AgentModalityCalloutState::Off => {
                (Some(AgentModalityCalloutState::TerminalMode), false)
            }
            AgentModalityCalloutState::TerminalMode => {
                // For Terminal intention, finish here
                // For Agent intention, continue to IntroducingAgentExperience
                match self.intention {
                    OnboardingIntention::Terminal => (
                        Some(AgentModalityCalloutState::Complete(FinalState::Finish)),
                        false,
                    ),
                    OnboardingIntention::AgentDrivenDevelopment => {
                        // Signal to enter agent modality when showing the agent experience slide
                        (Some(AgentModalityCalloutState::AgentMode), true)
                    }
                }
            }
            AgentModalityCalloutState::AgentMode => {
                // For Agent with project: Initialize
                // For Agent without project: Finish
                let final_state = if self.has_project {
                    FinalState::Initialize
                } else {
                    FinalState::Finish
                };
                (
                    Some(AgentModalityCalloutState::Complete(final_state)),
                    false,
                )
            }
            AgentModalityCalloutState::Complete(_) => (None, false),
        };
        if let Some(next_state) = next_state {
            self.set_state(OnboardingCalloutState::AgentModality(next_state), ctx);
        }
        if emit_enter_agent_modality {
            ctx.emit(OnboardingCalloutModelEvent::EnterAgentModality);
        }
    }

    pub fn skip(&mut self, ctx: &mut ModelContext<Self>) {
        match &self.state {
            OnboardingCalloutState::UniversalInput(UniversalInputCalloutState::TalkToAgent) => {
                self.set_state(
                    OnboardingCalloutState::UniversalInput(UniversalInputCalloutState::Complete(
                        FinalState::Skip,
                    )),
                    ctx,
                );
            }
            OnboardingCalloutState::AgentModality(AgentModalityCalloutState::AgentMode)
                if self.has_project =>
            {
                // Skip initialization
                self.set_state(
                    OnboardingCalloutState::AgentModality(AgentModalityCalloutState::Complete(
                        FinalState::Skip,
                    )),
                    ctx,
                );
            }
            _ => log::error!(
                "Skip action called in an unskippable state: {:?}",
                self.state
            ),
        }
    }

    pub fn finish(&mut self, ctx: &mut ModelContext<Self>) {
        match &self.state {
            OnboardingCalloutState::UniversalInput(UniversalInputCalloutState::TalkToAgent) => {
                self.set_state(
                    OnboardingCalloutState::UniversalInput(UniversalInputCalloutState::Complete(
                        FinalState::Finish,
                    )),
                    ctx,
                );
            }
            OnboardingCalloutState::AgentModality(AgentModalityCalloutState::TerminalMode) => {
                // Terminal intention finishes here
                self.set_state(
                    OnboardingCalloutState::AgentModality(AgentModalityCalloutState::Complete(
                        FinalState::Finish,
                    )),
                    ctx,
                );
            }
            OnboardingCalloutState::AgentModality(AgentModalityCalloutState::AgentMode)
                if !self.has_project =>
            {
                // Agent without project finishes here
                self.set_state(
                    OnboardingCalloutState::AgentModality(AgentModalityCalloutState::Complete(
                        FinalState::Finish,
                    )),
                    ctx,
                );
            }
            _ => log::error!("Finish action called in an invalid state: {:?}", self.state),
        }
    }

    /// Handle "Back to terminal" action (ESC in IntroducingAgentExperience).
    pub fn back_to_terminal(&mut self, ctx: &mut ModelContext<Self>) {
        match &self.state {
            OnboardingCalloutState::AgentModality(AgentModalityCalloutState::AgentMode) => {
                self.set_state(
                    OnboardingCalloutState::AgentModality(AgentModalityCalloutState::Complete(
                        FinalState::BackToTerminal,
                    )),
                    ctx,
                );
            }
            _ => log::error!(
                "BackToTerminal action called in an invalid state: {:?}",
                self.state
            ),
        }
    }

    pub fn is_onboarding_active(&self) -> bool {
        match &self.state {
            OnboardingCalloutState::UniversalInput(state) => !matches!(
                state,
                UniversalInputCalloutState::Off | UniversalInputCalloutState::Complete(_)
            ),
            OnboardingCalloutState::AgentModality(state) => !matches!(
                state,
                AgentModalityCalloutState::Off | AgentModalityCalloutState::Complete(_)
            ),
        }
    }

    pub fn state(&self) -> OnboardingCalloutState {
        self.state
    }

    fn send_callout_displayed_telemetry(
        new_state: OnboardingCalloutState,
        ctx: &mut ModelContext<Self>,
    ) {
        let callout_name = match new_state {
            OnboardingCalloutState::UniversalInput(UniversalInputCalloutState::MeetInput) => {
                Some("meet_input")
            }
            OnboardingCalloutState::UniversalInput(UniversalInputCalloutState::TalkToAgent) => {
                Some("talk_to_agent")
            }
            OnboardingCalloutState::AgentModality(AgentModalityCalloutState::TerminalMode) => {
                Some("natural_language_support")
            }
            OnboardingCalloutState::AgentModality(AgentModalityCalloutState::AgentMode) => {
                Some("introducing_agent_experience")
            }
            _ => None,
        };
        if let Some(callout) = callout_name {
            send_telemetry_from_ctx!(
                OnboardingEvent::CalloutDisplayed {
                    callout: callout.to_string(),
                },
                ctx
            );
        }
    }

    fn set_state(&mut self, new_state: OnboardingCalloutState, ctx: &mut ModelContext<Self>) {
        if self.state != new_state {
            self.state = new_state;
            Self::send_callout_displayed_telemetry(new_state, ctx);
            ctx.emit(OnboardingCalloutModelEvent::StateUpdated);

            // Check for completion
            let final_state = match new_state {
                OnboardingCalloutState::UniversalInput(UniversalInputCalloutState::Complete(
                    fs,
                )) => Some(fs),
                OnboardingCalloutState::AgentModality(AgentModalityCalloutState::Complete(fs)) => {
                    Some(fs)
                }
                _ => None,
            };

            if let Some(final_state) = final_state {
                send_telemetry_from_ctx!(
                    OnboardingEvent::CalloutCompleted {
                        completion_type: final_state.to_string(),
                    },
                    ctx
                );
                ctx.emit(OnboardingCalloutModelEvent::Completed(final_state));
            }
        }
    }

    /// Returns a prompt string to populate a command based on current state
    pub fn prompt_string(&self) -> String {
        match self.prompt() {
            OnboardingQuery::TerminalCommand(text) | OnboardingQuery::AgentPrompt(text) => text,
            OnboardingQuery::None => String::new(),
        }
    }

    /// Returns the prompt information including type for the current state
    pub fn prompt(&self) -> OnboardingQuery {
        match &self.state {
            OnboardingCalloutState::UniversalInput(state) => {
                self.prompt_for_universal_input(*state)
            }
            OnboardingCalloutState::AgentModality(state) => self.prompt_for_agent_modality(*state),
        }
    }

    fn prompt_for_universal_input(&self, state: UniversalInputCalloutState) -> OnboardingQuery {
        match state {
            UniversalInputCalloutState::Off
            | UniversalInputCalloutState::Complete(FinalState::Skip)
            | UniversalInputCalloutState::Complete(FinalState::Finish) => OnboardingQuery::None,
            UniversalInputCalloutState::MeetInput => {
                OnboardingQuery::TerminalCommand("git status".to_string())
            }
            UniversalInputCalloutState::TalkToAgent
            | UniversalInputCalloutState::Complete(FinalState::Submit) => {
                OnboardingQuery::AgentPrompt(
                    "What tests exist in this repo, how are they structured, and what do they cover?"
                        .to_string(),
                )
            }
            UniversalInputCalloutState::Complete(_) => OnboardingQuery::None,
        }
    }

    fn prompt_for_agent_modality(&self, state: AgentModalityCalloutState) -> OnboardingQuery {
        match state {
            AgentModalityCalloutState::Off => OnboardingQuery::None,
            AgentModalityCalloutState::TerminalMode => {
                OnboardingQuery::TerminalCommand("Run a command...".to_string())
            }
            AgentModalityCalloutState::AgentMode => {
                if self.has_project {
                    OnboardingQuery::AgentPrompt("/init".to_string())
                } else {
                    OnboardingQuery::AgentPrompt("Tell the agent what to build...".to_string())
                }
            }
            // All completion states should return None so the input gets cleared
            AgentModalityCalloutState::Complete(_) => OnboardingQuery::None,
        }
    }

    pub fn start_onboarding(&mut self, ctx: &mut ModelContext<Self>) {
        log::info!(
            "start_onboarding called with current state: {:?}",
            self.state
        );
        match &self.state {
            OnboardingCalloutState::UniversalInput(_) => {
                log::info!("Transitioning to UniversalInput::MeetInput");
                self.set_state(
                    OnboardingCalloutState::UniversalInput(UniversalInputCalloutState::MeetInput),
                    ctx,
                );
            }
            OnboardingCalloutState::AgentModality(_) => {
                log::info!("Transitioning to AgentModality::NaturalLanguageSupport");
                self.set_state(
                    OnboardingCalloutState::AgentModality(AgentModalityCalloutState::TerminalMode),
                    ctx,
                );
            }
        }
    }
}

impl Entity for OnboardingCalloutModel {
    type Event = OnboardingCalloutModelEvent;
}
