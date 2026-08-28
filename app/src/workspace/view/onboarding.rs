use onboarding::SelectedSettings;
use warp_core::execution_mode::AppExecutionMode;
use warpui::{SingletonEntity as _, ViewContext};

use crate::settings::AISettings;
use crate::terminal::view::{
    AgentOnboardingVersion, OnboardingIntention, OnboardingVersion, TerminalAction,
};
use crate::workspace::Workspace;
use crate::{FeatureFlag, terminal};

/// Configuration for starting the agent onboarding tutorial.
#[derive(Debug, Clone)]
pub enum OnboardingTutorial {
    /// Start tutorial without a project context.
    NoProject { intention: OnboardingIntention },
}

impl OnboardingTutorial {
    /// Extracts the onboarding intention from any tutorial variant.
    pub(crate) fn intention(&self) -> OnboardingIntention {
        match self {
            OnboardingTutorial::NoProject { intention } => *intention,
        }
    }
}

impl From<SelectedSettings> for OnboardingTutorial {
    fn from(settings: SelectedSettings) -> Self {
        let intention = match settings {
            SelectedSettings::AgentDrivenDevelopment { .. } => {
                OnboardingIntention::AgentDrivenDevelopment
            }
            SelectedSettings::Terminal { .. } => OnboardingIntention::Terminal,
        };
        OnboardingTutorial::NoProject { intention }
    }
}

impl Workspace {
    /// Start the agent onboarding tutorial.
    pub(crate) fn start_agent_onboarding_tutorial(
        &mut self,
        tutorial: OnboardingTutorial,
        ctx: &mut ViewContext<Self>,
    ) {
        // Onboarding requires a real user to interact with it; skip when running
        // in a headless mode like the SDK/CLI.
        if !AppExecutionMode::as_ref(ctx).can_show_onboarding() {
            return;
        }

        match tutorial {
            OnboardingTutorial::NoProject { intention } => {
                self.dispatch_tutorial_when_bootstrapped(false, intention, ctx);
            }
        }
    }

    /// Dispatch the onboarding tutorial after the terminal has finished bootstrapping.
    pub(crate) fn dispatch_tutorial_when_bootstrapped(
        &mut self,
        has_project: bool,
        intention: OnboardingIntention,
        ctx: &mut ViewContext<Self>,
    ) {
        // Onboarding requires a real user to interact with it; skip when running
        // in a headless mode like the SDK/CLI.
        if !AppExecutionMode::as_ref(ctx).can_show_onboarding() {
            return;
        }

        // Skip the guided tour when AI is not enabled (e.g. terminal-intent
        // users or users who disabled AI).
        if !*AISettings::as_ref(ctx).is_any_ai_enabled {
            return;
        }

        let Some(terminal_view_handle) = self.active_session_view(ctx) else {
            log::warn!("No active terminal view for onboarding tutorial");
            return;
        };

        let is_bootstrapped =
            terminal_view_handle.read(ctx, |view, _| view.is_login_shell_bootstrapped());

        if is_bootstrapped {
            // Terminal is already bootstrapped, dispatch immediately
            self.dispatch_agent_onboarding_tutorial(has_project, intention, ctx);
        } else {
            // Wait for bootstrapping to complete
            ctx.subscribe_to_view(
                &terminal_view_handle,
                move |me, terminal_view, event, ctx| {
                    if let terminal::Event::SessionBootstrapped = event {
                        me.dispatch_agent_onboarding_tutorial(has_project, intention, ctx);
                        ctx.unsubscribe_to_view(&terminal_view);
                    }
                },
            );
        }
    }

    /// Dispatch the agent onboarding tutorial flow to the active terminal.
    fn dispatch_agent_onboarding_tutorial(
        &self,
        has_project: bool,
        intention: OnboardingIntention,
        ctx: &mut ViewContext<Self>,
    ) {
        let version = OnboardingVersion::Agent(if FeatureFlag::AgentView.is_enabled() {
            AgentOnboardingVersion::AgentModality {
                has_project,
                intention,
            }
        } else {
            AgentOnboardingVersion::UniversalInput { has_project }
        });
        self.dispatch_onboarding(TerminalAction::OnboardingFlow(version), ctx);
    }

    /// Dispatch the onboarding tutorial after a pending command (e.g. worktree
    /// setup) finishes in the active terminal. Subscribes to
    /// `Event::PendingCommandCompleted` on the active terminal view.
    pub(crate) fn dispatch_tutorial_after_setup_commands(
        &mut self,
        intention: OnboardingIntention,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(terminal_view_handle) = self.active_session_view(ctx) else {
            log::warn!("No active terminal view for post-setup onboarding tutorial");
            return;
        };

        // Suppress deferred agent view entry so setup commands run in
        // terminal mode and the tutorial starts in terminal mode.
        terminal_view_handle.update(ctx, |view, _| {
            view.clear_enter_agent_view_after_pending_commands();
        });
        let has_pending_command = terminal_view_handle.read(ctx, |view, ctx| {
            view.has_pending_command_or_awaiting_completion(ctx)
        });
        if !has_pending_command {
            self.dispatch_tutorial_when_bootstrapped(true, intention, ctx);
            return;
        }

        ctx.subscribe_to_view(
            &terminal_view_handle,
            move |me, terminal_view, event, ctx| {
                if let terminal::Event::PendingCommandCompleted = event {
                    // Start the onboarding tutorial now that setup is done.
                    // TODO(roland): We do have a directory in this case so we could consider passing has_project = true
                    // which has an optional /init flow. But the behavior of /init needs to be revisited:
                    // 1. Sends /init as a query which differs in behavior from /init slash command
                    // 2. Sends /init even if not in a git repo - unclear if this should happen (depends on desired behavior from 1)
                    // 3. With no free AI, /init will not work.
                    me.dispatch_agent_onboarding_tutorial(false, intention, ctx);
                    ctx.unsubscribe_to_view(&terminal_view);
                }
            },
        );
    }

    pub(crate) fn should_show_agent_onboarding(&self, ctx: &mut ViewContext<Self>) -> bool {
        // Onboarding requires a real user to interact with it; suppress when
        // running in a headless mode like the SDK/CLI.
        if !AppExecutionMode::as_ref(ctx).can_show_onboarding() {
            return false;
        }
        FeatureFlag::AgentOnboarding.is_enabled()
    }
}
