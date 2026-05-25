use asset_macro::bundled_or_fetched_asset;
use markdown_parser::{FormattedTextFragment, FormattedTextLine};
use warp_core::send_telemetry_from_ctx;
use warpui::assets::asset_cache::AssetSource;
use warpui::{AppContext, SingletonEntity};

use super::{CTAButton, CheckboxConfig, LaunchModalEvent, Slide};
use crate::ai::ambient_agents::telemetry::{CloudAgentTelemetryEvent, CloudModeEntryPoint};
use crate::localization;
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
    fn modal_title(&self, app: &AppContext) -> String {
        text(app, "workspace.launch_modal.oz.modal_title")
    }

    fn modal_subtext_paragraphs(&self, app: &AppContext) -> Vec<FormattedTextLine> {
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text(text(app, "workspace.launch_modal.oz.modal_subtext")),
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

    fn display_text(&self, app: &AppContext) -> Option<String> {
        Some(match self {
            OzLaunchSlide::CloudAgents => text(app, "workspace.launch_modal.oz.cloud_agents.tab"),
            OzLaunchSlide::AgentAutomations => {
                text(app, "workspace.launch_modal.oz.agent_automations.tab")
            }
            OzLaunchSlide::AgentManagement => {
                text(app, "workspace.launch_modal.oz.agent_management.tab")
            }
            OzLaunchSlide::LaunchCredits => {
                text(app, "workspace.launch_modal.oz.launch_credits.tab")
            }
        })
    }

    fn short_label(&self, app: &AppContext) -> String {
        match self {
            OzLaunchSlide::CloudAgents => {
                text(app, "workspace.launch_modal.oz.cloud_agents.short_label")
            }
            OzLaunchSlide::AgentAutomations => text(
                app,
                "workspace.launch_modal.oz.agent_automations.short_label",
            ),
            OzLaunchSlide::AgentManagement => text(
                app,
                "workspace.launch_modal.oz.agent_management.short_label",
            ),
            OzLaunchSlide::LaunchCredits => {
                text(app, "workspace.launch_modal.oz.launch_credits.short_label")
            }
        }
    }

    fn title(&self, app: &AppContext) -> String {
        match self {
            OzLaunchSlide::CloudAgents => text(app, "workspace.launch_modal.oz.cloud_agents.title"),
            OzLaunchSlide::AgentAutomations => {
                text(app, "workspace.launch_modal.oz.agent_automations.title")
            }
            OzLaunchSlide::AgentManagement => {
                text(app, "workspace.launch_modal.oz.agent_management.title")
            }
            OzLaunchSlide::LaunchCredits => {
                text(app, "workspace.launch_modal.oz.launch_credits.title")
            }
        }
    }

    fn title_icon(&self) -> Option<Icon> {
        None
    }

    fn content(&self, app: &AppContext) -> String {
        match self {
            OzLaunchSlide::CloudAgents => {
                text(app, "workspace.launch_modal.oz.cloud_agents.content")
            }
            OzLaunchSlide::AgentAutomations => {
                text(app, "workspace.launch_modal.oz.agent_automations.content")
            }
            OzLaunchSlide::AgentManagement => {
                text(app, "workspace.launch_modal.oz.agent_management.content")
            }
            OzLaunchSlide::LaunchCredits => {
                text(app, "workspace.launch_modal.oz.launch_credits.content")
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

    fn cta_button(&self, app: &AppContext) -> CTAButton<Self> {
        match self {
            OzLaunchSlide::CloudAgents
            | OzLaunchSlide::AgentAutomations
            | OzLaunchSlide::AgentManagement => {
                let next = self.next().expect("Non-final slides should have a next");
                CTAButton::next_slide(
                    next,
                    text(app, "workspace.launch_modal.oz.action.next")
                        .replace("{slide}", &next.short_label(app)),
                )
            }
            OzLaunchSlide::LaunchCredits => CTAButton::custom(
                text(app, "workspace.launch_modal.oz.action.try_it_out"),
                |ctx| {
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
                },
            ),
        }
    }

    fn secondary_cta_button(&self, app: &AppContext) -> Option<CTAButton<Self>> {
        match self {
            OzLaunchSlide::LaunchCredits => Some(CTAButton::close(text(
                app,
                "workspace.launch_modal.oz.action.skip_for_now",
            ))),
            OzLaunchSlide::CloudAgents
            | OzLaunchSlide::AgentAutomations
            | OzLaunchSlide::AgentManagement => None,
        }
    }

    fn checkbox_config(&self, app: &AppContext) -> Option<CheckboxConfig> {
        Some(CheckboxConfig {
            label: text(app, "workspace.launch_modal.oz.checkbox.sync_conversations"),
            description: text(app, "workspace.launch_modal.oz.checkbox.description"),
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

fn text(app: &AppContext, key: &str) -> String {
    localization::text_for_app(app, key)
}

pub fn init(app: &mut warpui::AppContext) {
    super::init::<OzLaunchSlide>(app);
}
