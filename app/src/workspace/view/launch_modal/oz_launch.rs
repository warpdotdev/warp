use asset_macro::bundled_or_fetched_asset;
use markdown_parser::{FormattedTextFragment, FormattedTextLine};
use warp_core::send_telemetry_from_ctx;
use warpui::assets::asset_cache::AssetSource;
use warpui::{AppContext, SingletonEntity};

use super::{CTAButton, CheckboxConfig, LaunchModalEvent, Slide};
use crate::ai::ambient_agents::telemetry::{CloudAgentTelemetryEvent, CloudModeEntryPoint};
use crate::terminal::view::OnboardingIntention;
use crate::ui_components::icons::Icon;
use crate::workspace::action::WorkspaceAction;
use crate::workspace::view::OnboardingTutorial;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{AdminEnablementSetting, UgcCollectionEnablementSetting};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OzLaunchSlide {
    CloudAgents,
    AgentAutomations,
    AgentManagement,
    LaunchCredits,
}

impl Slide for OzLaunchSlide {
    fn modal_title(&self) -> String {
        "Introducing Oz".to_string()
    }

    fn modal_subtext_paragraphs(&self) -> Vec<FormattedTextLine> {
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text(
                "Infinitely scalable coding agent — run in local sessions or in the cloud.",
            ),
        ])]
    }

    fn first() -> Self {
        OzLaunchSlide::CloudAgents
    }

    fn next(&self) -> Option<Self> {
        match self {
            OzLaunchSlide::CloudAgents => Some(OzLaunchSlide::AgentAutomations),
            OzLaunchSlide::AgentAutomations => Some(OzLaunchSlide::AgentManagement),
            OzLaunchSlide::AgentManagement => Some(OzLaunchSlide::LaunchCredits),
            OzLaunchSlide::LaunchCredits => None,
        }
    }

    fn prev(&self) -> Option<Self> {
        match self {
            OzLaunchSlide::CloudAgents => None,
            OzLaunchSlide::AgentAutomations => Some(OzLaunchSlide::CloudAgents),
            OzLaunchSlide::AgentManagement => Some(OzLaunchSlide::AgentAutomations),
            OzLaunchSlide::LaunchCredits => Some(OzLaunchSlide::AgentManagement),
        }
    }

    fn display_text(&self) -> Option<&'static str> {
        Some(match self {
            OzLaunchSlide::CloudAgents => "Cloud agents",
            OzLaunchSlide::AgentAutomations => "Agent automations",
            OzLaunchSlide::AgentManagement => "Agent management",
            OzLaunchSlide::LaunchCredits => "A little gift",
        })
    }

    fn short_label(&self) -> &'static str {
        match self {
            OzLaunchSlide::CloudAgents => "Cloud agents",
            OzLaunchSlide::AgentAutomations => "Agent automations",
            OzLaunchSlide::AgentManagement => "Agent management",
            OzLaunchSlide::LaunchCredits => "Launch credits",
        }
    }

    fn title(&self) -> &'static str {
        match self {
            OzLaunchSlide::CloudAgents => "Break out of your laptop with cloud agents",
            OzLaunchSlide::AgentAutomations => {
                "Orchestrate agents, turning Skills into automations"
            }
            OzLaunchSlide::AgentManagement => "Track local and cloud agents seamlessly",
            OzLaunchSlide::LaunchCredits => {
                "1,000 free cloud agent credits when you upgrade to Warp Build"
            }
        }
    }

    fn title_icon(&self) -> Option<Icon> {
        None
    }

    fn content(&self) -> &'static str {
        match self {
            OzLaunchSlide::CloudAgents => {
                "Use cloud agents to run many agents in parallel, keep agents working when you close your laptop, or start agents programmatically. Plus, you can check on their work through the web."
            }
            OzLaunchSlide::AgentAutomations => {
                "Oz agents can be defined using the standard Skills format. You can use the built in scheduler to setup agents to run autonomously at set intervals, or use the Oz SDK or API to programmatically start and manage Oz agents."
            }
            OzLaunchSlide::AgentManagement => {
                "View all of your agents across local and cloud sessions in the Warp app or at [oz.warp.dev](https://oz.warp.dev). Join live agent sessions, continue tasks locally, and steer agents with one click."
            }
            OzLaunchSlide::LaunchCredits => {
                "Upgrade to Build this month and receive 1,000 extra credits to try using Oz. Credits are only eligible for Oz runs in Warp-hosted cloud environments."
            }
        }
    }

    fn image(&self) -> AssetSource {
        // TODO: Replace with new images once provided.
        match self {
            OzLaunchSlide::CloudAgents => {
                bundled_or_fetched_asset!("png/oz_cloud_agents.png")
            }
            OzLaunchSlide::AgentAutomations => {
                bundled_or_fetched_asset!("png/oz_agent_automations.png")
            }
            OzLaunchSlide::AgentManagement => {
                bundled_or_fetched_asset!("png/oz_agent_management.png")
            }
            OzLaunchSlide::LaunchCredits => {
                bundled_or_fetched_asset!("png/oz_launch_credits.png")
            }
        }
    }

    fn all() -> Vec<Self> {
        vec![
            OzLaunchSlide::CloudAgents,
            OzLaunchSlide::AgentAutomations,
            OzLaunchSlide::AgentManagement,
            OzLaunchSlide::LaunchCredits,
        ]
    }

    fn cta_button(&self) -> CTAButton<Self> {
        match self {
            OzLaunchSlide::CloudAgents
            | OzLaunchSlide::AgentAutomations
            | OzLaunchSlide::AgentManagement => {
                let next = self.next().expect("Non-final slides should have a next");
                CTAButton::next_slide(next, format!("Next: {}", next.short_label()))
            }
            OzLaunchSlide::LaunchCredits => CTAButton::custom("Try it out", |ctx| {
                send_telemetry_from_ctx!(
                    CloudAgentTelemetryEvent::EnteredCloudMode {
                        entry_point: CloudModeEntryPoint::OzLaunchModal,
                    },
                    ctx
                );
                ctx.emit(LaunchModalEvent::Close);
                ctx.dispatch_typed_action(&WorkspaceAction::StartAgentOnboardingTutorial(
                    OnboardingTutorial::NoProject {
                        intention: OnboardingIntention::AgentDrivenDevelopment,
                    },
                ));
                ctx.dispatch_typed_action(&WorkspaceAction::AddAmbientAgentTab);
            }),
        }
    }

    fn secondary_cta_button(&self) -> Option<CTAButton<Self>> {
        match self {
            OzLaunchSlide::LaunchCredits => Some(CTAButton::close("Skip for now")),
            OzLaunchSlide::CloudAgents
            | OzLaunchSlide::AgentAutomations
            | OzLaunchSlide::AgentManagement => None,
        }
    }

    fn checkbox_config(&self) -> Option<CheckboxConfig> {
        Some(CheckboxConfig {
            label: "Sync conversations to cloud",
            description: "Agent conversations stored in the cloud can be shared with anyone with one click, and allow conversations to be continued across devices and on logout.",
        })
    }

    fn should_show_checkbox(&self, app: &AppContext) -> bool {
        let cloud_storage_setting =
            UserWorkspaces::as_ref(app).get_cloud_conversation_storage_enablement_setting();
        let ugc_setting = UserWorkspaces::as_ref(app).get_ugc_collection_enablement_setting();

        // Show checkbox only when user has control over cloud storage AND UGC is not force-enabled.
        matches!(
            cloud_storage_setting,
            AdminEnablementSetting::RespectUserSetting
        ) && !matches!(ugc_setting, UgcCollectionEnablementSetting::Enable)
    }

    fn on_close(&self, ctx: &mut warpui::ViewContext<super::LaunchModal<Self>>) {
        ctx.dispatch_typed_action(&WorkspaceAction::StartAgentOnboardingTutorial(
            OnboardingTutorial::NoProject {
                intention: OnboardingIntention::AgentDrivenDevelopment,
            },
        ));
    }
}

pub fn init(app: &mut warpui::AppContext) {
    super::init::<OzLaunchSlide>(app);
}
