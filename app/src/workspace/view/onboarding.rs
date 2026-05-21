use crate::pane_group::{NewTerminalOptions, PanesLayout};
use crate::settings::AISettings;
use crate::terminal;
use crate::terminal::view::{
    AgentOnboardingVersion, OnboardingIntention, OnboardingVersion, TerminalAction,
};
use crate::workspace::Workspace;
use crate::FeatureFlag;
use onboarding::{ProjectOnboardingSettings, SelectedSettings};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use warp_core::execution_mode::AppExecutionMode;
use warpui::{SingletonEntity as _, ViewContext};

/// Configuration for starting the agent onboarding tutorial.
#[derive(Debug, Clone)]
pub enum OnboardingTutorial {
    /// Start tutorial without a project context.
    NoProject { intention: OnboardingIntention },
    /// Start tutorial with a project path, but don't run init.
    Project {
        path: PathBuf,
        intention: OnboardingIntention,
    },
    /// Start tutorial with a project path and run init flow first.
    InitProject {
        path: PathBuf,
        intention: OnboardingIntention,
    },
}

impl OnboardingTutorial {
    /// Extracts the onboarding intention from any tutorial variant.
    pub(crate) fn intention(&self) -> OnboardingIntention {
        match self {
            OnboardingTutorial::NoProject { intention }
            | OnboardingTutorial::Project { intention, .. }
            | OnboardingTutorial::InitProject { intention, .. } => *intention,
        }
    }
}

impl From<SelectedSettings> for OnboardingTutorial {
    fn from(settings: SelectedSettings) -> Self {
        match settings {
            SelectedSettings::AgentDrivenDevelopment {
                project_settings, ..
            } => match project_settings {
                ProjectOnboardingSettings::Project {
                    selected_local_folder,
                    initialize_projects_automatically,
                } => {
                    let path = PathBuf::from(selected_local_folder);
                    // When AgentView is enabled, /init comes at the end of the tutorial.
                    if !FeatureFlag::AgentView.is_enabled() && initialize_projects_automatically {
                        OnboardingTutorial::InitProject {
                            path,
                            intention: OnboardingIntention::AgentDrivenDevelopment,
                        }
                    } else {
                        OnboardingTutorial::Project {
                            path,
                            intention: OnboardingIntention::AgentDrivenDevelopment,
                        }
                    }
                }
                ProjectOnboardingSettings::NoProject => OnboardingTutorial::NoProject {
                    intention: OnboardingIntention::AgentDrivenDevelopment,
                },
            },
            SelectedSettings::Terminal { .. } => OnboardingTutorial::NoProject {
                intention: OnboardingIntention::Terminal,
            },
        }
    }
}

impl Workspace {
    /// Start the agent onboarding tutorial.
    ///
    /// Depending on the variant of `tutorial`, this will either:
    /// - `NoProject`: Start the tutorial immediately without any project context
    /// - `Project`: Change to the project directory and start the tutorial
    /// - `InitProject`: Open the repository, wait for init to complete, then start the tutorial
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
            OnboardingTutorial::InitProject {
                ref path,
                intention,
            } => {
                // Open the repository - this will create a new terminal and trigger init
                let Some(path_str) = path.to_str() else {
                    log::error!("Failed to convert path to string: {path:?}");
                    return;
                };
                self.handle_open_repository(path_str, ctx);

                // Subscribe to the terminal view to wait for init completion
                if let Some(terminal_view_handle) = self.active_session_view(ctx) {
                    ctx.subscribe_to_view(
                        &terminal_view_handle,
                        move |me, terminal_view, event, ctx| {
                            if let terminal::Event::OnboardingInitCompleted = event {
                                // Init flow is complete, now start the tutorial
                                me.dispatch_agent_onboarding_tutorial(true, intention, ctx);
                                ctx.unsubscribe_to_view(&terminal_view);
                            }
                        },
                    );
                }
            }
            OnboardingTutorial::Project {
                ref path,
                intention,
            } => {
                // Create a new terminal in the project directory
                self.add_tab_with_pane_layout(
                    PanesLayout::SingleTerminal(Box::new(NewTerminalOptions {
                        initial_directory: Some(path.clone()),
                        hide_homepage: true,
                        ..Default::default()
                    })),
                    Arc::new(HashMap::new()),
                    None,
                    ctx,
                );
                self.dispatch_tutorial_when_bootstrapped(true, intention, ctx);
            }
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

        // With new onboarding, skip the guided tour when AI is not enabled
        // (e.g. terminal-intent users or users who disabled AI).
        if FeatureFlag::OpenWarpNewSettingsModes.is_enabled()
            && !*AISettings::as_ref(ctx).is_any_ai_enabled
        {
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
