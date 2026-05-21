use ui_components::Component;
use warp_core::ui::appearance::Appearance;
use warpui::{
    elements::Empty,
    keymap::{macros::*, FixedBinding, Keystroke},
    AppContext, Element, Entity, EventContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext,
};

/// Display strings for keybindings shown in the onboarding callout.
#[derive(Clone, Debug)]
pub struct OnboardingKeybindings {
    /// Display string for toggling between agent/terminal mode (e.g., "⌘I")
    pub toggle_input_mode: String,
    /// Display string for submitting to local agent (e.g., "⌘⏎")
    pub submit_to_local_agent: String,
    /// Display string for submitting to cloud agent (e.g., "⌘⌥⏎")
    pub submit_to_cloud_agent: String,
    /// Display string for returning to terminal mode (e.g., "Esc")
    pub return_to_terminal_mode: String,
}

use crate::{
    callout::model::{
        AgentModalityCalloutState, FinalState, OnboardingCalloutModel, OnboardingCalloutModelEvent,
        OnboardingCalloutState, OnboardingQuery, UniversalInputCalloutState,
    },
    components::onboarding_callout::{self, Button, StepStatus},
    OnboardingIntention,
};

/// Options for rendering a callout.
struct CalloutOptions {
    title: &'static str,
    /// Pre-built text with keybindings already embedded
    text: String,
    step: StepStatus,
    right_button: ButtonOptions,
    /// Optional left button (e.g., "Skip", "Back to terminal")
    left_button: Option<ButtonOptions>,
    /// Optional checkbox for natural language detection
    checkbox: Option<CheckboxOptions>,
}

struct ButtonOptions {
    text: &'static str,
    action: OnboardingCalloutViewAction,
    keystroke: Option<Keystroke>,
}

struct CheckboxOptions {
    label: &'static str,
    checked: bool,
}

fn get_universal_input_callout_options(
    state: UniversalInputCalloutState,
    has_project: bool,
    keybindings: &OnboardingKeybindings,
) -> Option<CalloutOptions> {
    match state {
        UniversalInputCalloutState::MeetInput => Some(CalloutOptions {
            title: "Meet the Warp input",
            text: format!(
                "Your terminal input accepts both terminal commands and agent prompts and automatically detects which you're using. Use {} to lock the input to Agent mode (natural language) or Terminal mode (commands).",
                keybindings.toggle_input_mode
            ),
            step: StepStatus::new(0, 2),
            left_button: None,
            right_button: ButtonOptions {
                text: "Next",
                action: OnboardingCalloutViewAction::NextClicked,
                keystroke: Some(Keystroke::parse("enter").unwrap_or_default()),
            },
            checkbox: None,
        }),
        UniversalInputCalloutState::TalkToAgent => Some(CalloutOptions {
            title: "Talk to the agent",
            text: "You can type in natural language to engage the agent. Submit the query below to start: What tests exist in this repo, how are they structured, and what do they cover?".to_string(),
            step: StepStatus::new(1, 2),
            left_button: if has_project {
                Some(ButtonOptions {
                    text: "Skip",
                    action: OnboardingCalloutViewAction::SkipClicked,
                    keystroke: Some(Keystroke::parse("delete").unwrap_or_default()),
                })
            } else {
                None
            },
            right_button: ButtonOptions {
                text: if has_project { "Submit" } else { "Finish" },
                action: OnboardingCalloutViewAction::NextClicked,
                keystroke: Some(Keystroke::parse("enter").unwrap_or_default()),
            },
            checkbox: None,
        }),
        UniversalInputCalloutState::Off | UniversalInputCalloutState::Complete(_) => None,
    }
}

fn get_agent_modality_callout_options(
    state: AgentModalityCalloutState,
    intention: OnboardingIntention,
    has_project: bool,
    initial_natural_language_detection_enabled: bool,
    natural_language_detection_enabled: bool,
    keybindings: &OnboardingKeybindings,
) -> Option<CalloutOptions> {
    let total_steps = match intention {
        OnboardingIntention::Terminal => 1,
        OnboardingIntention::AgentDrivenDevelopment => 2,
    };

    match state {
        AgentModalityCalloutState::TerminalMode => {
            let is_final_step = intention == OnboardingIntention::Terminal;
            // Show different callout content based on initial NL detection state
            if initial_natural_language_detection_enabled {
                // NL detection was already enabled - show simpler "overrides" callout without checkbox
                Some(CalloutOptions {
                    title: "Welcome to terminal mode",
                    text: format!(
                        "Run commands here, just like a regular terminal. If you type a question or task using natural language, Warp can suggest opening it in agent mode. You can always override using {}.",
                        keybindings.toggle_input_mode
                    ),
                    step: StepStatus::new(0, total_steps),
                    left_button: None,
                    right_button: ButtonOptions {
                        text: if is_final_step { "Finish" } else { "Next" },
                        action: OnboardingCalloutViewAction::NextClicked,
                        keystroke: Some(Keystroke::parse("enter").unwrap_or_default()),
                    },
                    checkbox: None,
                })
            } else {
                // NL detection was disabled - show full explanation with checkbox to enable
                Some(CalloutOptions {
                    title: "You’re in terminal mode",
                    text: format!(
                        "Run commands here, just like a regular terminal. If you type a question or task using natural language, Warp can suggest opening it in agent mode. You can always override using {}.",
                        keybindings.toggle_input_mode
                    ),
                    step: StepStatus::new(0, total_steps),
                    left_button: None,
                    right_button: ButtonOptions {
                        text: if is_final_step { "Finish" } else { "Next" },
                        action: OnboardingCalloutViewAction::NextClicked,
                        keystroke: Some(Keystroke::parse("enter").unwrap_or_default()),
                    },
                    checkbox: Some(CheckboxOptions {
                        label: "Enable Natural Language Detection",
                        checked: natural_language_detection_enabled,
                    }),
                })
            }
        }
        AgentModalityCalloutState::AgentMode => {
            if has_project {
                Some(CalloutOptions {
                    title: "You're in agent mode",
                    text: "Agent mode gives your questions and tasks their own conversation, so you can ask follow-ups without leaving your terminal workflow.\n\nSubmit the query below to have the agent initialize this project, or ⊗ to clear the input and start your own!".to_string(),
                    step: StepStatus::new(1, total_steps),
                    left_button: Some(ButtonOptions {
                        text: "Skip initialization",
                        action: OnboardingCalloutViewAction::SkipClicked,
                        keystroke: Some(Keystroke::parse("delete").unwrap_or_default()),
                    }),
                    right_button: ButtonOptions {
                        text: "Initialize",
                        action: OnboardingCalloutViewAction::NextClicked,
                        keystroke: Some(Keystroke::parse("enter").unwrap_or_default()),
                    },
                    checkbox: None,
                })
            } else {
                Some(CalloutOptions {
                    title: "You're in agent mode",
                    text: format!(
                        "Agent mode gives your questions and tasks their own conversation, so you can ask follow-ups without leaving your terminal workflow. Press {} to return to terminal mode at any point.",
                        keybindings.return_to_terminal_mode
                    ),
                    step: StepStatus::new(1, total_steps),
                    left_button: Some(ButtonOptions {
                        text: "Back to terminal",
                        action: OnboardingCalloutViewAction::BackToTerminalClicked,
                        keystroke: Some(Keystroke::parse("escape").unwrap_or_default()),
                    }),
                    right_button: ButtonOptions {
                        text: "Finish",
                        action: OnboardingCalloutViewAction::NextClicked,
                        keystroke: Some(Keystroke::parse("enter").unwrap_or_default()),
                    },
                    checkbox: None,
                })
            }
        }
        AgentModalityCalloutState::Off | AgentModalityCalloutState::Complete(_) => None,
    }
}

#[derive(Clone, Debug)]
pub enum OnboardingCalloutViewAction {
    NextClicked,
    SkipClicked,
    BackToTerminalClicked,
    ToggleCheckbox,
}

pub fn init(app: &mut AppContext) {
    app.register_fixed_bindings([
        FixedBinding::new(
            "enter",
            OnboardingCalloutViewAction::NextClicked,
            id!(OnboardingCalloutView::ui_name()),
        ),
        FixedBinding::new(
            "numpadenter",
            OnboardingCalloutViewAction::NextClicked,
            id!(OnboardingCalloutView::ui_name()),
        ),
        FixedBinding::new(
            "backspace",
            OnboardingCalloutViewAction::SkipClicked,
            id!(OnboardingCalloutView::ui_name()),
        ),
        FixedBinding::new(
            "escape",
            OnboardingCalloutViewAction::BackToTerminalClicked,
            id!(OnboardingCalloutView::ui_name()),
        ),
    ]);
}

#[derive(Clone, Debug)]
pub enum OnboardingCalloutViewEvent {
    StateUpdated,
    Completed {
        final_state: FinalState,
    },
    /// Signals that the terminal should enter agent modality (agent view).
    EnterAgentModality,
    /// Emitted when the user toggles the natural language detection checkbox.
    NaturalLanguageDetectionToggled(bool),
}

/// A view that renders the onboarding callout UI component based on the current model state
pub struct OnboardingCalloutView {
    /// Reference to the model that manages onboarding state
    model: ModelHandle<OnboardingCalloutModel>,
    /// The UI component that renders the actual callout
    callout_component: onboarding_callout::OnboardingCallout,
    /// Display strings for keybindings shown in the callout
    keybindings: OnboardingKeybindings,
}

impl OnboardingCalloutView {
    /// Create a new view for the UniversalInput onboarding flow.
    pub fn new_universal_input(
        has_project: bool,
        initial_natural_language_detection_enabled: bool,
        keybindings: OnboardingKeybindings,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let model = ctx.add_model(|_ctx| {
            OnboardingCalloutModel::new_universal_input(
                has_project,
                initial_natural_language_detection_enabled,
            )
        });
        Self::with_model(model, keybindings, ctx)
    }

    /// Create a new view for the AgentModality onboarding flow.
    pub fn new_agent_modality(
        has_project: bool,
        intention: OnboardingIntention,
        initial_natural_language_detection_enabled: bool,
        keybindings: OnboardingKeybindings,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let model = ctx.add_model(|_ctx| {
            OnboardingCalloutModel::new_agent_modality(
                has_project,
                intention,
                initial_natural_language_detection_enabled,
            )
        });
        Self::with_model(model, keybindings, ctx)
    }

    fn with_model(
        model: ModelHandle<OnboardingCalloutModel>,
        keybindings: OnboardingKeybindings,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        // Re-emit model updates as view events so parents can subscribe to the view.
        ctx.subscribe_to_model(&model, |_me, _model, event, ctx| match event {
            OnboardingCalloutModelEvent::StateUpdated => {
                ctx.emit(OnboardingCalloutViewEvent::StateUpdated);
                ctx.notify();
            }
            OnboardingCalloutModelEvent::Completed(final_state) => {
                ctx.emit(OnboardingCalloutViewEvent::Completed {
                    final_state: *final_state,
                });
                ctx.notify();
            }
            OnboardingCalloutModelEvent::EnterAgentModality => {
                ctx.emit(OnboardingCalloutViewEvent::EnterAgentModality);
                ctx.notify();
            }
            OnboardingCalloutModelEvent::NaturalLanguageDetectionToggled(enabled) => {
                ctx.emit(OnboardingCalloutViewEvent::NaturalLanguageDetectionToggled(
                    *enabled,
                ));
                ctx.notify();
            }
        });

        Self {
            model,
            callout_component: onboarding_callout::OnboardingCallout::default(),
            keybindings,
        }
    }

    pub fn has_project(&self, app: &AppContext) -> bool {
        self.model.as_ref(app).has_project()
    }

    pub fn start_onboarding(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.start_onboarding(ctx);
        });
        ctx.notify();
    }

    pub fn is_onboarding_active(&self, app: &AppContext) -> bool {
        self.model.as_ref(app).is_onboarding_active()
    }

    pub fn prompt_string(&self, app: &AppContext) -> String {
        self.model.as_ref(app).prompt_string()
    }

    pub fn prompt(&self, app: &AppContext) -> OnboardingQuery {
        self.model.as_ref(app).prompt()
    }

    /// Returns true if the callout should be positioned above the zero state.
    /// For the agent experience state, always position relative to the input box instead.
    pub fn should_position_above_zero_state(&self, app: &AppContext) -> bool {
        !matches!(
            self.model.as_ref(app).state(),
            OnboardingCalloutState::AgentModality(AgentModalityCalloutState::AgentMode)
        )
    }

    fn get_callout_options(&self, app: &AppContext) -> Option<CalloutOptions> {
        let model = self.model.as_ref(app);
        match model.state() {
            OnboardingCalloutState::UniversalInput(state) => {
                get_universal_input_callout_options(state, model.has_project(), &self.keybindings)
            }
            OnboardingCalloutState::AgentModality(state) => get_agent_modality_callout_options(
                state,
                model.intention(),
                model.has_project(),
                model.initial_natural_language_detection_enabled(),
                model.natural_language_detection_enabled(),
                &self.keybindings,
            ),
        }
    }
}

impl Entity for OnboardingCalloutView {
    type Event = OnboardingCalloutViewEvent;
}

impl View for OnboardingCalloutView {
    fn ui_name() -> &'static str {
        "OnboardingCalloutView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let model = self.model.as_ref(app);

        // Check if onboarding is active and render appropriate callout based on state
        if !model.is_onboarding_active() {
            return Empty::new().finish();
        }

        let Some(options) = self.get_callout_options(app) else {
            log::warn!(
                "Onboarding callout view: onboarding is active but state has no callout options: {:?}",
                model.state()
            );
            return Empty::new().finish();
        };

        let right_button = Button {
            text: options.right_button.text.into(),
            keystroke: options.right_button.keystroke,
            handler: Box::new(move |ctx: &mut EventContext, _app_ctx: &AppContext, _pos| {
                ctx.dispatch_typed_action(options.right_button.action.clone());
            }),
        };

        let left_button = options.left_button.map(|left_opts| Button {
            text: left_opts.text.into(),
            keystroke: left_opts.keystroke,
            handler: Box::new(move |ctx: &mut EventContext, _app_ctx: &AppContext, _pos| {
                ctx.dispatch_typed_action(left_opts.action.clone());
            }),
        });

        let checkbox = options
            .checkbox
            .map(|checkbox_opts| onboarding_callout::Checkbox {
                label: checkbox_opts.label.into(),
                checked: checkbox_opts.checked,
                handler: Box::new(|ctx: &mut EventContext, _app_ctx: &AppContext, _pos| {
                    ctx.dispatch_typed_action(OnboardingCalloutViewAction::ToggleCheckbox);
                }),
            });

        // Render the callout component with data from the model state
        self.callout_component.render(
            appearance,
            onboarding_callout::Params {
                title: options.title.to_string().into(),
                text: options.text.into(),
                step: options.step,
                right_button,
                options: onboarding_callout::Options {
                    left_button,
                    checkbox,
                },
            },
        )
    }
}

impl TypedActionView for OnboardingCalloutView {
    type Action = OnboardingCalloutViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            OnboardingCalloutViewAction::NextClicked => {
                self.model.update(ctx, |model, ctx| {
                    // Handle special cases for UniversalInput flow
                    if let OnboardingCalloutState::UniversalInput(
                        UniversalInputCalloutState::TalkToAgent,
                    ) = model.state()
                    {
                        if !model.has_project() {
                            model.finish(ctx);
                            return;
                        }
                    }
                    model.next(ctx);
                });
                ctx.notify();
            }
            OnboardingCalloutViewAction::SkipClicked => {
                self.model.update(ctx, |model, ctx| {
                    model.skip(ctx);
                });
                ctx.notify();
            }
            OnboardingCalloutViewAction::BackToTerminalClicked => {
                self.model.update(ctx, |model, ctx| {
                    model.back_to_terminal(ctx);
                });
                ctx.notify();
            }
            OnboardingCalloutViewAction::ToggleCheckbox => {
                self.model.update(ctx, |model, ctx| {
                    model.toggle_natural_language_detection(ctx);
                });
                ctx.notify();
            }
        }
    }
}

impl SingletonEntity for OnboardingCalloutView {}
