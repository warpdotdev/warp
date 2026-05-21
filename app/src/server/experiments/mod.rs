//! This module is responsible for applying
//! server-side experiment state to the client.
//!
//! After adding support for your experiment on the server,
//! you need to define what effect the experiment will
//! have on the client (i.e. see [`ServerExperiment::on_added_to`]).
//!
//! Then, you can use the global [`ServerExperiments`] model
//! to update and query the latest experiment state.
//!
//! See [here](https://www.notion.so/warpdev/Server-side-experiments-dynamic-feature-enablement-c0fb9aed695d4178a19b8830e3269094)
//! for a full guide on the server-side experiment framework.

use crate::features::FeatureFlag;
use crate::terminal::warpify::settings::{SshExtensionInstallMode, WarpifySettings};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::CustomerType;
use settings::Setting;
use warpui::AppContext;
#[cfg(not(test))]
use warpui::SingletonEntity as _;
#[cfg(test)]
use warpui::SingletonEntity;

mod convert;
mod model;

pub use model::{Event as ServerExperimentsEvent, ServerExperiments};

/// The known server-side experiments.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ServerExperiment {
    SessionSharingExperiment,
    SessionSharingControl,
    DisableAgentModeExperiment,
    EnvVarsEarlyAccessExperiment,
    AgentModeAnalyticsExperiment,
    WindowsLaunchExperiment,
    TmuxSshWarpificationControl,
    TmuxSshWarpificationExperiment,
    CodebaseContextExperiment,
    CodebaseContextControl,
    SuggestedCodeDiffsControl,
    SuggestedCodeDiffsExperiment,
    BuildPlanAutoReloadControl,
    BuildPlanAutoReloadBannerToggle,
    BuildPlanAutoReloadPostPurchaseModal,
    PromptSuggestionsViaMaaControl,
    PromptSuggestionsViaMaaExperiment,
    PromptSuggestionsViaMaaOutOfBandExperiment,
    FreeUserNoAiControl,
    FreeUserNoAiExperiment,
    OzMultiHarnessControl,
    OzMultiHarnessExperiment,
    SshRemoteServerControl,
    SshRemoteServerExperiment,
    /// A test-only experiment.
    /// Does not correspond to a real server-side experiment.
    #[cfg(test)]
    TestExperiment,
}

impl ServerExperiment {
    /// When the client is added to an experiment.
    ///
    // TODO: currently, it isn't possible to interact with other
    // application singletons in this function because [`ServerExperiments`]
    // is initialized before most other singletons during app init. We should either:
    // a) remove the `ctx` from this function to prevent reading / updating
    //    other application entities here, which forces those other entities
    //    to subscribe to [`ServerExperiments`] updates instead, or
    // b) continue initializing [`ServerExperiments`] as one of the first
    //    singletons but apply the cached state after initializing all other singletons.
    //    That way, this method would not be called until after all other singletons
    //    have been initialized and can thus be referenced.
    fn on_added_to(&self, _ctx: &mut AppContext) {
        match self {
            Self::SessionSharingExperiment => {
                FeatureFlag::CreatingSharedSessions.set_enabled(true);
            }
            Self::SessionSharingControl => {
                FeatureFlag::CreatingSharedSessions.set_enabled(false);
            }
            Self::DisableAgentModeExperiment => {
                FeatureFlag::AgentMode.set_enabled(false);
            }
            Self::EnvVarsEarlyAccessExperiment => {
                // EnvVars is now always enabled; no-op.
            }
            Self::AgentModeAnalyticsExperiment => {
                FeatureFlag::AgentModeAnalytics.set_enabled(true);
                FeatureFlag::AIRules.set_enabled(true);
                FeatureFlag::SuggestedRules.set_enabled(true);
            }
            Self::WindowsLaunchExperiment => {
                // TODO(alokedesai): Clean this up now that we no longer gate access to the Windows
                // build on an allowlist.
            }
            Self::TmuxSshWarpificationControl => FeatureFlag::SSHTmuxWrapper.set_enabled(false),
            Self::TmuxSshWarpificationExperiment => {
                // Only enable the TMUX-based experience if not on windows. ConPTY doesn't support
                // DCS, which we need in order to use tmux control mode.
                if cfg!(not(windows)) {
                    FeatureFlag::SSHTmuxWrapper.set_enabled(true)
                }
            }
            Self::CodebaseContextExperiment => {
                FeatureFlag::FullSourceCodeEmbedding.set_enabled(true);
                FeatureFlag::CodebaseIndexPersistence.set_enabled(true);
                FeatureFlag::CodebaseIndexSpeedbump.set_enabled(true);
                FeatureFlag::CrossRepoContext.set_enabled(true);
            }
            Self::CodebaseContextControl => {
                FeatureFlag::FullSourceCodeEmbedding.set_enabled(false);
                FeatureFlag::CodebaseIndexPersistence.set_enabled(false);
                FeatureFlag::CodebaseIndexSpeedbump.set_enabled(false);
                FeatureFlag::CrossRepoContext.set_enabled(false);
            }
            Self::SuggestedCodeDiffsExperiment => {}
            Self::SuggestedCodeDiffsControl => {}
            Self::BuildPlanAutoReloadControl => {
                // Control group - disable both experiment flags
                FeatureFlag::BuildPlanAutoReloadBannerToggle.set_enabled(false);
                FeatureFlag::BuildPlanAutoReloadPostPurchaseModal.set_enabled(false);
            }
            Self::BuildPlanAutoReloadBannerToggle => {
                // Experiment variant 1 - enable banner toggle modal
                FeatureFlag::BuildPlanAutoReloadBannerToggle.set_enabled(true);
                FeatureFlag::BuildPlanAutoReloadPostPurchaseModal.set_enabled(false);
            }
            Self::BuildPlanAutoReloadPostPurchaseModal => {
                // Experiment variant 2 - enable post-purchase modal
                FeatureFlag::BuildPlanAutoReloadBannerToggle.set_enabled(false);
                FeatureFlag::BuildPlanAutoReloadPostPurchaseModal.set_enabled(true);
            }
            Self::PromptSuggestionsViaMaaControl => {
                FeatureFlag::PromptSuggestionsViaMAA.set_enabled(false);
            }
            Self::PromptSuggestionsViaMaaOutOfBandExperiment => {
                FeatureFlag::PromptSuggestionsViaMAA.set_enabled(true);
            }
            // The normal experiment arm is no longer used.
            Self::PromptSuggestionsViaMaaExperiment => {}
            Self::FreeUserNoAiControl => {
                FeatureFlag::FreeUserNoAi.set_enabled(false);
            }
            Self::FreeUserNoAiExperiment => {
                FeatureFlag::FreeUserNoAi.set_enabled(true);
            }
            Self::OzMultiHarnessControl => {
                FeatureFlag::AgentHarness.set_enabled(false);
            }
            Self::OzMultiHarnessExperiment => {
                FeatureFlag::AgentHarness.set_enabled(true);
            }
            Self::SshRemoteServerControl => {
                // Remote server binary is not yet supported on Windows.
                if cfg!(not(windows)) {
                    FeatureFlag::SshRemoteServer.set_enabled(true);
                    // Override the default install mode to NeverInstall for users
                    // who haven't explicitly changed it. `load_value` sets the
                    // in-memory value without persisting, so the override is
                    // re-applied from the experiment cache on every launch and
                    // disappears if the user leaves the experiment.
                    WarpifySettings::handle(_ctx).update(_ctx, |settings, ctx| {
                        if !settings
                            .ssh_extension_install_mode
                            .is_value_explicitly_set()
                        {
                            let _ = settings.ssh_extension_install_mode.load_value(
                                SshExtensionInstallMode::NeverInstall,
                                false,
                                ctx,
                            );
                        }
                    });
                }
            }
            Self::SshRemoteServerExperiment => {
                // Remote server binary is not yet supported on Windows.
                if cfg!(not(windows)) {
                    FeatureFlag::SshRemoteServer.set_enabled(true);
                    // Restore the default install mode in case the user was
                    // previously in the control arm (which overrides it to
                    // NeverInstall).
                    WarpifySettings::handle(_ctx).update(_ctx, |settings, ctx| {
                        if !settings
                            .ssh_extension_install_mode
                            .is_value_explicitly_set()
                        {
                            let _ = settings.ssh_extension_install_mode.load_value(
                                SshExtensionInstallMode::default(),
                                false,
                                ctx,
                            );
                        }
                    });
                }
            }
            #[cfg(test)]
            Self::TestExperiment => {
                model::TestModel::handle(_ctx).update(_ctx, |model, _| {
                    model.0 += 1;
                });
            }
        }
    }
}

/// Returns `true` when the user is in the `FreeUserNoAiExperiment` arm **and** is on the
/// free tier. This is the single source of truth for gating any client-side behaviour
/// that should be locked/disabled for users without AI credits.
pub fn is_free_user_no_ai_experiment_active(ctx: &AppContext) -> bool {
    let in_experiment = FeatureFlag::FreeUserNoAi.is_enabled();
    let is_free_tier = UserWorkspaces::handle(ctx)
        .as_ref(ctx)
        .current_team()
        .map(|team| team.billing_metadata.customer_type == CustomerType::Free)
        .unwrap_or(true); // no team = solo free user
    in_experiment && is_free_tier
}
