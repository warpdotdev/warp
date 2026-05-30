//! Logic to convert from / to [`ServerExperiment`].

use std::fmt::{Display, Formatter};

use anyhow::{Ok, Result};
use warp_graphql::experiment::Experiment;

use super::ServerExperiment;

impl Display for ServerExperiment {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            Self::SessionSharingControl => "SESSION_SHARING_CONTROL",
            Self::SessionSharingExperiment => "SESSION_SHARING_EXPERIMENT",
            Self::DisableAgentModeExperiment => "DISABLE_AGENT_MODE_EXPERIMENT",
            Self::EnvVarsEarlyAccessExperiment => "ENV_VARS_EARLY_ACCESS_EXPERIMENT",
            Self::AgentModeAnalyticsExperiment => "AGENT_MODE_ANALYTICS_EXPERIMENT",
            Self::WindowsLaunchExperiment => "WINDOWS_LAUNCH_EXPERIMENT",
            Self::TmuxSshWarpificationControl => "TMUX_SSH_WARPIFICATION_CONTROL",
            Self::TmuxSshWarpificationExperiment => "TMUX_SSH_WARPIFICATION_EXPERIMENT",
            Self::CodebaseContextControl => "CODEBASE_CONTEXT_CONTROL",
            Self::CodebaseContextExperiment => "CODEBASE_CONTEXT_EXPERIMENT",
            Self::SuggestedCodeDiffsControl => "SUGGESTED_CODE_DIFFS_CONTROL",
            Self::SuggestedCodeDiffsExperiment => "SUGGESTED_CODE_DIFFS_EXPERIMENT",
            Self::BuildPlanAutoReloadControl => "BUILD_PLAN_AUTO_RELOAD_CONTROL",
            Self::BuildPlanAutoReloadBannerToggle => "BUILD_PLAN_AUTO_RELOAD_BANNER_TOGGLE",
            Self::BuildPlanAutoReloadPostPurchaseModal => {
                "BUILD_PLAN_AUTO_RELOAD_POST_PURCHASE_MODAL"
            }
            Self::PromptSuggestionsViaMaaControl => "PROMPT_SUGGESTIONS_VIA_MAA_CONTROL",
            Self::PromptSuggestionsViaMaaExperiment => "PROMPT_SUGGESTIONS_VIA_MAA_EXPERIMENT",
            Self::PromptSuggestionsViaMaaOutOfBandExperiment => {
                "PROMPT_SUGGESTIONS_VIA_MAA_OOB_EXPERIMENT"
            }
            Self::FreeUserNoAiControl => "FREE_USER_NO_AI_CONTROL",
            Self::FreeUserNoAiExperiment => "FREE_USER_NO_AI_EXPERIMENT",
            Self::OzMultiHarnessControl => "OZ_MULTI_HARNESS_CONTROL",
            Self::OzMultiHarnessExperiment => "OZ_MULTI_HARNESS_EXPERIMENT",
            Self::SshRemoteServerControl => "SSH_REMOTE_SERVER_CONTROL",
            Self::SshRemoteServerExperiment => "SSH_REMOTE_SERVER_EXPERIMENT",
            #[cfg(test)]
            Self::TestExperiment => "TEST_EXPERIMENT",
        };
        write!(f, "{str}")
    }
}

impl ServerExperiment {
    pub fn from_string(s: String) -> Result<Self> {
        match s.as_str() {
            "SESSION_SHARING_CONTROL" => Ok(Self::SessionSharingControl),
            "SESSION_SHARING_EXPERIMENT" => Ok(Self::SessionSharingExperiment),
            "DISABLE_AGENT_MODE_EXPERIMENT" => Ok(Self::DisableAgentModeExperiment),
            "ENV_VARS_EARLY_ACCESS_EXPERIMENT" => Ok(Self::EnvVarsEarlyAccessExperiment),
            "AGENT_MODE_ANALYTICS_EXPERIMENT" => Ok(Self::AgentModeAnalyticsExperiment),
            "WINDOWS_LAUNCH_EXPERIMENT" => Ok(Self::WindowsLaunchExperiment),
            "TMUX_SSH_WARPIFICATION_CONTROL" => Ok(Self::TmuxSshWarpificationControl),
            "TMUX_SSH_WARPIFICATION_EXPERIMENT" => Ok(Self::TmuxSshWarpificationExperiment),
            "CODEBASE_CONTEXT_EXPERIMENT" => Ok(Self::CodebaseContextExperiment),
            "CODEBASE_CONTEXT_CONTROL" => Ok(Self::CodebaseContextControl),
            "SUGGESTED_CODE_DIFFS_CONTROL" => Ok(Self::SuggestedCodeDiffsControl),
            "SUGGESTED_CODE_DIFFS_EXPERIMENT" => Ok(Self::SuggestedCodeDiffsExperiment),
            "BUILD_PLAN_AUTO_RELOAD_CONTROL" => Ok(Self::BuildPlanAutoReloadControl),
            "BUILD_PLAN_AUTO_RELOAD_BANNER_TOGGLE" => Ok(Self::BuildPlanAutoReloadBannerToggle),
            "BUILD_PLAN_AUTO_RELOAD_POST_PURCHASE_MODAL" => {
                Ok(Self::BuildPlanAutoReloadPostPurchaseModal)
            }
            "PROMPT_SUGGESTIONS_VIA_MAA_CONTROL" => Ok(Self::PromptSuggestionsViaMaaControl),
            "PROMPT_SUGGESTIONS_VIA_MAA_EXPERIMENT" => Ok(Self::PromptSuggestionsViaMaaExperiment),
            "FREE_USER_NO_AI_CONTROL" => Ok(Self::FreeUserNoAiControl),
            "FREE_USER_NO_AI_EXPERIMENT" => Ok(Self::FreeUserNoAiExperiment),
            "OZ_MULTI_HARNESS_CONTROL" => Ok(Self::OzMultiHarnessControl),
            "OZ_MULTI_HARNESS_EXPERIMENT" => Ok(Self::OzMultiHarnessExperiment),
            "SSH_REMOTE_SERVER_CONTROL" => Ok(Self::SshRemoteServerControl),
            "SSH_REMOTE_SERVER_EXPERIMENT" => Ok(Self::SshRemoteServerExperiment),
            s => Err(anyhow::anyhow!(
                "String doesn't match any server experiment variant {s}"
            )),
        }
    }
}

impl TryFrom<Experiment> for ServerExperiment {
    type Error = anyhow::Error;

    fn try_from(value: Experiment) -> Result<Self, Self::Error> {
        match value {
            Experiment::SessionSharingExperiment => Ok(Self::SessionSharingExperiment),
            Experiment::SessionSharingControl => Ok(Self::SessionSharingControl),
            Experiment::BuildPlanAutoReloadControl => Ok(Self::BuildPlanAutoReloadControl),
            Experiment::BuildPlanAutoReloadBannerToggle => {
                Ok(Self::BuildPlanAutoReloadBannerToggle)
            }
            Experiment::BuildPlanAutoReloadPostPurchaseModal => {
                Ok(Self::BuildPlanAutoReloadPostPurchaseModal)
            }
            Experiment::DisableAgentModeExperiment => Ok(Self::DisableAgentModeExperiment),
            Experiment::EnvVarsEarlyAccessExperiment => Ok(Self::EnvVarsEarlyAccessExperiment),
            Experiment::AgentModeAnalyticsExperiment => Ok(Self::AgentModeAnalyticsExperiment),
            Experiment::TmuxSshWarpificationControl => Ok(Self::TmuxSshWarpificationControl),
            Experiment::TmuxSshWarpificationExperiment => Ok(Self::TmuxSshWarpificationExperiment),
            Experiment::WindowsLaunchExperiment => Ok(Self::WindowsLaunchExperiment),
            Experiment::CodebaseContextControl => Ok(Self::CodebaseContextControl),
            Experiment::CodebaseContextExperiment => Ok(Self::CodebaseContextExperiment),
            Experiment::SuggestedCodeDiffsControl => Ok(Self::SuggestedCodeDiffsControl),
            Experiment::SuggestedCodeDiffsExperiment => Ok(Self::SuggestedCodeDiffsExperiment),
            Experiment::PromptSuggestionsViaMaaControl => Ok(Self::PromptSuggestionsViaMaaControl),
            Experiment::PromptSuggestionsViaMaaOob => {
                Ok(Self::PromptSuggestionsViaMaaOutOfBandExperiment)
            }
            Experiment::FreeUserNoAiControl => Ok(Self::FreeUserNoAiControl),
            Experiment::FreeUserNoAiExperiment => Ok(Self::FreeUserNoAiExperiment),
            Experiment::OzMultiHarnessControl => Ok(Self::OzMultiHarnessControl),
            Experiment::OzMultiHarnessExperiment => Ok(Self::OzMultiHarnessExperiment),
            Experiment::SshRemoteServerControl => Ok(Self::SshRemoteServerControl),
            Experiment::SshRemoteServerExperiment => Ok(Self::SshRemoteServerExperiment),
            // Experiments that we no longer support on the client.
            e => Err(anyhow::anyhow!(
                "Server-side enabled experiment '{e:?}' is no longer supported by the client."
            )),
        }
    }
}

#[macro_export]
macro_rules! convert_to_server_experiment {
    ($gql_type:expr) => {{
        let mut acc = Vec::new();
        for a in $gql_type {
            // Note for server experiments we don't currently track on the client.
            // This could be because the client is out of date and we should still
            // apply the experiments the client does track.
            if let Ok(b) = ServerExperiment::try_from(a) {
                acc.push(b);
            }
        }
        Some(acc)
    }};
}
