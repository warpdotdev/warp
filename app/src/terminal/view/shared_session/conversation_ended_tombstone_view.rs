use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_management::telemetry::{AgentManagementTelemetryEvent, ArtifactType};
use crate::ai::ambient_agents::AmbientAgentTask;
use crate::ai::ambient_agents::{
    conversation_output_status_from_conversation, AmbientAgentTaskId, AmbientConversationStatus,
};
use crate::ai::artifacts::{Artifact, ArtifactButtonsRow, ArtifactButtonsRowEvent};
use crate::ai::blocklist::{format_credits, BlocklistAIHistoryModel};
use crate::appearance::Appearance;
use crate::server::ids::SyncId;
use crate::settings::ai::{AISettings, AISettingsChangedEvent};
use crate::ui_components::blended_colors;
use crate::util::time_format::human_readable_precise_duration;
use crate::view_components::action_button::{ActionButton, PrimaryTheme};
use crate::workspace::WorkspaceAction;
use std::path::Path;
use warp_core::paths::home_relative_path;
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::{AnsiColorIdentifier, Fill};
use warpui::elements::{
    Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty,
    Expanded, Flex, MainAxisSize, Padding, ParentElement, Radius, Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{
    AppContext, Element, Entity, EntityId, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::cloud_conversation_continuation::TombstoneCta;
use crate::server::server_api::ServerApiProvider;

/// Metadata collected for display in the tombstone.
#[derive(Default)]
struct TombstoneDisplayData {
    title: Option<String>,
    is_error: bool,
    error_message: Option<String>,
    conversation_is_transcript: bool,
    /// Source of the task (Linear, Slack, etc.) - only for ambient agent tasks
    source: Option<String>,
    /// Skill/config name - only for ambient agent tasks
    skill_name: Option<String>,
    /// Run time as formatted string
    run_time: Option<String>,
    /// Credits spent
    credits: Option<String>,
    /// Working directory at start of conversation
    working_directory: Option<String>,
    /// Artifacts from the conversation
    artifacts: Vec<Artifact>,
}

#[derive(Debug, Clone)]
pub enum ConversationEndedTombstoneEvent {
    ContinueInCloud { task_id: AmbientAgentTaskId },
}

impl TombstoneDisplayData {
    fn from_conversation(
        conversation_id: AIConversationId,
        terminal_view_id: EntityId,
        has_task_id: bool,
        ctx: &ViewContext<ConversationEndedTombstoneView>,
    ) -> Self {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        let conversation_is_transcript = !has_task_id
            && history_model
                .as_ref(ctx)
                .is_terminal_view_conversation_transcript_viewer(terminal_view_id);
        let conversation = history_model
            .as_ref(ctx)
            .all_live_conversations_for_terminal_view(terminal_view_id)
            .find(|c| c.id() == conversation_id);

        let Some(conversation) = conversation else {
            return Self::default();
        };

        let conversation_status = conversation_output_status_from_conversation(conversation);
        let is_error = matches!(
            conversation_status,
            Some(AmbientConversationStatus::Error { .. })
        );
        let error_message = conversation_status
            .as_ref()
            .and_then(|status| match status {
                AmbientConversationStatus::Error { error } => Some(error.to_string()),
                _ => None,
            });

        // Calculate run time from exchanges
        let run_time = (|| {
            let first_exchange = conversation.first_exchange()?;
            let last_exchange = conversation.latest_exchange()?;
            let finish_time = last_exchange.finish_time?;
            let duration = finish_time.signed_duration_since(first_exchange.start_time);
            Some(human_readable_precise_duration(duration))
        })();

        Self {
            title: conversation.title(),
            is_error,
            error_message,
            conversation_is_transcript,
            source: None,
            skill_name: None,
            run_time,
            credits: Some(format_credits(conversation.credits_spent())),
            working_directory: conversation.initial_working_directory(),
            artifacts: conversation.artifacts().to_vec(),
        }
    }

    fn enrich_from_task(&mut self, task: AmbientAgentTask) {
        // Use task title if we don't have a conversation title.
        if self.title.is_none() {
            self.title = Some(task.title.clone());
        }

        if let Some(source) = &task.source {
            self.source = Some(source.display_name().to_string());
        }
        if let Some(config) = &task.agent_config_snapshot {
            // FIXME: this can be the orchestrator agent name, not a skill.
            self.skill_name = config.name.clone();
        }

        if task.state.is_failure_like() {
            self.is_error = true;
            if let Some(status_message) = &task.status_message {
                self.error_message = Some(status_message.message.clone());
            }
        }

        // We update to use the task values when we have them, which includes
        // the full credit cost (inference + compute). This matches what we show in
        // the details panel.
        if let Some(run_time) = task.run_time() {
            self.run_time = Some(human_readable_precise_duration(run_time));
        }
        if let Some(credits) = task.credits_used() {
            self.credits = Some(format_credits(credits));
        }

        // Surface task artifacts (plans, PRs, files, screenshots) for third-party
        // harness runs.
        if !task.artifacts.is_empty() {
            self.artifacts = task.artifacts;
        }
    }
}

/// Tombstone view shown when an agent conversation ends.
/// Displays metadata, artifacts, and actions like "Continue locally".
pub struct ConversationEndedTombstoneView {
    display_data: TombstoneDisplayData,
    artifact_buttons_view: ViewHandle<ArtifactButtonsRow>,
    continue_in_cloud_button: Option<ViewHandle<ActionButton>>,
    #[cfg(not(target_family = "wasm"))]
    continue_locally_button: Option<ViewHandle<ActionButton>>,
    #[cfg(target_family = "wasm")]
    open_in_warp_button: Option<ViewHandle<ActionButton>>,
}

impl ConversationEndedTombstoneView {
    #[cfg_attr(target_family = "wasm", allow(unused_variables))]
    pub fn new(
        ctx: &mut ViewContext<Self>,
        terminal_view_id: EntityId,
        task_id: Option<AmbientAgentTaskId>,
        tombstone_cta: Option<TombstoneCta>,
    ) -> Self {
        let conversation_id = BlocklistAIHistoryModel::handle(ctx)
            .as_ref(ctx)
            .all_live_conversations_for_terminal_view(terminal_view_id)
            .next()
            .map(|c| c.id());

        let mut display_data = conversation_id
            .map(|id| {
                TombstoneDisplayData::from_conversation(
                    id,
                    terminal_view_id,
                    task_id.is_some(),
                    ctx,
                )
            })
            .unwrap_or_default();
        if display_data.is_error && task_id.is_none() && !display_data.conversation_is_transcript {
            display_data.title = Some("Cloud agent failed to start".to_string());
            display_data.credits = None;
        }

        let artifact_buttons_view =
            ctx.add_typed_action_view(|ctx| ArtifactButtonsRow::new(&display_data.artifacts, ctx));
        let continue_in_cloud_button = match tombstone_cta {
            Some(TombstoneCta::ContinueInCloud { task_id }) => {
                Some(ctx.add_typed_action_view(move |_| {
                    ActionButton::new("Continue", PrimaryTheme)
                        .with_tooltip("Continue this cloud conversation")
                        .on_click(move |ctx| {
                            ctx.dispatch_typed_action(
                                ConversationEndedTombstoneAction::ContinueInCloud { task_id },
                            );
                        })
                }))
            }
            Some(TombstoneCta::ContinueLocally { .. }) | None => None,
        };

        #[cfg(not(target_family = "wasm"))]
        let continue_locally_button = match tombstone_cta {
            Some(TombstoneCta::ContinueLocally { conversation_id }) => {
                Some(ctx.add_typed_action_view(move |_| {
                    ActionButton::new("Continue locally", PrimaryTheme)
                        .with_tooltip("Fork this conversation locally")
                        .on_click(move |ctx| {
                            ctx.dispatch_typed_action(
                                ConversationEndedTombstoneAction::ContinueLocally(conversation_id),
                            );
                        })
                }))
            }
            Some(TombstoneCta::ContinueInCloud { .. }) | None => None,
        };

        // In wasm, continuing locally is impossible so we instead
        // offer to open the conversation in warp (where you can continue locally).
        #[cfg(target_family = "wasm")]
        let open_in_warp_button =
            if matches!(tombstone_cta, Some(TombstoneCta::ContinueInCloud { .. })) {
                None
            } else {
                conversation_id.map(|conv_id| {
                    ctx.add_typed_action_view(move |_| {
                        ActionButton::new("Open in Warp", PrimaryTheme)
                            .with_tooltip("Open this conversation in the Warp desktop app")
                            .on_click(move |ctx| {
                                ctx.dispatch_typed_action(
                                    ConversationEndedTombstoneAction::OpenInWarp(conv_id),
                                );
                            })
                    })
                })
            };

        let view = Self {
            display_data,
            artifact_buttons_view,
            continue_in_cloud_button,
            #[cfg(not(target_family = "wasm"))]
            continue_locally_button,
            #[cfg(target_family = "wasm")]
            open_in_warp_button,
        };

        ctx.subscribe_to_view(
            &view.artifact_buttons_view,
            |_, _, event, ctx| match event {
                ArtifactButtonsRowEvent::OpenPlan { notebook_uid } => {
                    send_telemetry_from_ctx!(
                        AgentManagementTelemetryEvent::TombstoneArtifactClicked {
                            artifact_type: ArtifactType::Plan
                        },
                        ctx
                    );
                    ctx.dispatch_typed_action(&WorkspaceAction::OpenNotebook {
                        id: SyncId::ServerId((*notebook_uid).into()),
                    });
                }
                ArtifactButtonsRowEvent::CopyBranch { branch } => {
                    send_telemetry_from_ctx!(
                        AgentManagementTelemetryEvent::TombstoneArtifactClicked {
                            artifact_type: ArtifactType::Branch
                        },
                        ctx
                    );
                    ctx.clipboard()
                        .write(warpui::clipboard::ClipboardContent::plain_text(
                            branch.clone(),
                        ));
                }
                ArtifactButtonsRowEvent::OpenPullRequest { url } => {
                    send_telemetry_from_ctx!(
                        AgentManagementTelemetryEvent::TombstoneArtifactClicked {
                            artifact_type: ArtifactType::PullRequest
                        },
                        ctx
                    );
                    ctx.open_url(url);
                }
                ArtifactButtonsRowEvent::ViewScreenshots { artifact_uids } => {
                    crate::ai::artifacts::open_screenshot_lightbox(artifact_uids, ctx);
                }
                ArtifactButtonsRowEvent::DownloadFile { artifact_uid } => {
                    send_telemetry_from_ctx!(
                        AgentManagementTelemetryEvent::TombstoneArtifactClicked {
                            artifact_type: ArtifactType::File
                        },
                        ctx
                    );
                    crate::ai::artifacts::download_file_artifact(artifact_uid, ctx);
                }
            },
        );

        // Re-render when AI settings change so the continue button hides/shows accordingly.
        ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, event, ctx| {
            if matches!(event, AISettingsChangedEvent::IsAnyAIEnabled { .. }) {
                ctx.notify();
            }
        });

        // Fetch AmbientAgentTask for additional metadata (source, skill, artifacts, etc.)
        if let Some(task_id) = task_id {
            let ai_client = ServerApiProvider::handle(ctx).as_ref(ctx).get_ai_client();
            ctx.spawn(
                async move { ai_client.get_ambient_agent_task(&task_id).await },
                |me, result, ctx| match result {
                    Ok(task) => {
                        me.display_data.enrich_from_task(task);
                        me.artifact_buttons_view.update(ctx, |row, ctx| {
                            row.update_artifacts(&me.display_data.artifacts, ctx);
                        });
                        ctx.notify();
                    }
                    Err(err) => {
                        log::warn!(
                            "Failed to fetch AmbientAgentTask for tombstone metadata: {err}"
                        );
                    }
                },
            );
        }

        view
    }

    fn render_header(&self, is_transcript: bool, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        if is_transcript {
            return Text::new(
                "You're viewing a snapshot",
                appearance.overline_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(blended_colors::text_main(theme, theme.background()))
            .with_style(Properties::default().weight(Weight::Bold))
            .finish();
        }

        let icon = if self.display_data.is_error {
            Icon::X
        } else {
            Icon::Check
        };
        let icon_color = if self.display_data.is_error {
            Fill::Solid(theme.ansi_fg_red())
        } else {
            Fill::Solid(theme.ansi_fg_green())
        };
        let icon_element = Container::new(
            ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
                .with_height(14.)
                .with_width(14.)
                .finish(),
        )
        .with_margin_right(8.)
        .finish();

        let title = self
            .display_data
            .title
            .clone()
            .unwrap_or_else(|| "Agent task".to_string());
        Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(icon_element)
            .with_child(
                Shrinkable::new(
                    1.,
                    Text::new(
                        title,
                        appearance.overline_font_family(),
                        appearance.monospace_font_size(),
                    )
                    .with_color(blended_colors::text_main(theme, theme.background()))
                    .with_style(Properties::default().weight(Weight::Bold))
                    .soft_wrap(true)
                    .finish(),
                )
                .finish(),
            )
            .finish()
    }

    fn render_snapshot_subtitle(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(
            Text::new(
                "This shared conversation shows the state when you opened it. \
                 If the agent is still running, refresh to see the latest progress.",
                appearance.overline_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(blended_colors::text_main(theme, theme.background()))
            .soft_wrap(true)
            .finish(),
        )
        .with_margin_top(4.)
        .finish()
    }

    fn render_metadata_row(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut parts: Vec<String> = Vec::new();

        if let Some(dir) = &self.display_data.working_directory {
            let display_dir = home_relative_path(Path::new(dir));
            parts.push(format!("Directory: {display_dir}"));
        }

        if let Some(source) = &self.display_data.source {
            parts.push(format!("Source: {source}"));
        }

        if let Some(skill) = &self.display_data.skill_name {
            parts.push(format!("Skill: {skill}"));
        }

        if let Some(run_time) = &self.display_data.run_time {
            parts.push(format!("Run time: {run_time}"));
        }

        if let Some(credits) = &self.display_data.credits {
            parts.push(format!("Credits used: {credits}"));
        }

        if parts.is_empty() {
            return Empty::new().finish();
        }

        Text::new(
            parts.join(" • "),
            appearance.overline_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(blended_colors::text_sub(theme, theme.background()))
        .soft_wrap(true)
        .finish()
    }

    fn render_error_message(&self, appearance: &Appearance) -> Box<dyn Element> {
        let Some(error_message) = &self.display_data.error_message else {
            return Empty::new().finish();
        };

        let theme = appearance.theme();

        Container::new(
            Text::new(
                error_message.clone(),
                appearance.overline_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(theme.ansi_fg_red())
            .soft_wrap(true)
            .finish(),
        )
        .with_margin_top(4.)
        .finish()
    }

    #[cfg_attr(target_family = "wasm", allow(unused_variables))]
    fn render_action_buttons(
        &self,
        _appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.);

        let mut has_button = false;

        let is_any_ai_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);
        if is_any_ai_enabled {
            if let Some(continue_in_cloud_button) = &self.continue_in_cloud_button {
                row.add_child(ChildView::new(continue_in_cloud_button).finish());
                has_button = true;
            }
            #[cfg(not(target_family = "wasm"))]
            {
                if let Some(continue_locally_button) = &self.continue_locally_button {
                    row.add_child(ChildView::new(continue_locally_button).finish());
                    has_button = true;
                }
            }
        }

        #[cfg(target_family = "wasm")]
        {
            // Don't show on mobile devices - they can't use the desktop app
            if !warpui::platform::wasm::is_mobile_device() {
                if let Some(ref open_in_warp_button) = self.open_in_warp_button {
                    row.add_child(ChildView::new(open_in_warp_button).finish());
                    has_button = true;
                }
            }
        }

        if !has_button {
            return Empty::new().finish();
        }
        row.finish()
    }
}

#[cfg(test)]
impl ConversationEndedTombstoneView {
    pub(in crate::terminal::view) fn title_for_test(&self) -> Option<&str> {
        self.display_data.title.as_deref()
    }

    pub(in crate::terminal::view) fn error_message_for_test(&self) -> Option<&str> {
        self.display_data.error_message.as_deref()
    }

    pub(in crate::terminal::view) fn credits_for_test(&self) -> Option<&str> {
        self.display_data.credits.as_deref()
    }

    #[cfg(not(target_family = "wasm"))]
    pub(in crate::terminal::view) fn has_continue_locally_button_for_test(&self) -> bool {
        self.continue_locally_button.is_some()
    }

    pub(in crate::terminal::view) fn has_continue_in_cloud_button_for_test(&self) -> bool {
        self.continue_in_cloud_button.is_some()
    }
}
#[derive(Debug, Clone)]
pub enum ConversationEndedTombstoneAction {
    ContinueInCloud {
        task_id: AmbientAgentTaskId,
    },
    #[cfg(not(target_family = "wasm"))]
    ContinueLocally(AIConversationId),
    #[cfg(target_family = "wasm")]
    OpenInWarp(AIConversationId),
}

impl View for ConversationEndedTombstoneView {
    fn ui_name() -> &'static str {
        "ConversationEndedTombstoneView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::handle(app).as_ref(app);
        let theme = appearance.theme();
        let is_transcript = self.display_data.conversation_is_transcript;

        let mut left_column = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(self.render_header(is_transcript, appearance));

        if is_transcript {
            left_column.add_child(self.render_snapshot_subtitle(appearance));
        }

        let metadata_margin_top = if is_transcript { 12. } else { 4. };
        left_column.add_child(
            Container::new(self.render_metadata_row(appearance))
                .with_margin_top(metadata_margin_top)
                .finish(),
        );

        if !is_transcript {
            left_column.add_child(self.render_error_message(appearance));
        }

        if !self.display_data.artifacts.is_empty() {
            left_column.add_child(
                Container::new(ChildView::new(&self.artifact_buttons_view).finish())
                    .with_margin_top(12.)
                    .finish(),
            );
        }

        let content = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(12.)
            .with_child(Expanded::new(1., left_column.finish()).finish())
            .with_child(self.render_action_buttons(appearance, app))
            .finish();

        // Card styling
        let (background, border_fill) = if self.display_data.is_error {
            let red = AnsiColorIdentifier::Red.to_ansi_color(&theme.terminal_colors().normal);
            (theme.ansi_overlay_1(red).into(), theme.ansi_fg_red().into())
        } else {
            (theme.surface_2(), theme.outline())
        };

        let card = Container::new(content)
            .with_background(background)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_border(Border::all(1.0).with_border_fill(border_fill))
            .with_uniform_margin(8.)
            .with_padding(Padding::uniform(16.))
            .finish();

        Container::new(card)
            .with_border(Border::top(1.0).with_border_fill(theme.outline()))
            .finish()
    }
}

impl Entity for ConversationEndedTombstoneView {
    type Event = ConversationEndedTombstoneEvent;
}

impl TypedActionView for ConversationEndedTombstoneView {
    type Action = ConversationEndedTombstoneAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ConversationEndedTombstoneAction::ContinueInCloud { task_id } => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::TombstoneContinueInCloud {
                        task_id: task_id.to_string()
                    },
                    ctx
                );
                ctx.emit(ConversationEndedTombstoneEvent::ContinueInCloud { task_id: *task_id });
            }
            #[cfg(not(target_family = "wasm"))]
            ConversationEndedTombstoneAction::ContinueLocally(conversation_id) => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::TombstoneContinueLocally,
                    ctx
                );
                ctx.dispatch_typed_action(&WorkspaceAction::ContinueConversationLocally {
                    conversation_id: *conversation_id,
                });
            }
            #[cfg(target_family = "wasm")]
            ConversationEndedTombstoneAction::OpenInWarp(conversation_id) => {
                send_telemetry_from_ctx!(AgentManagementTelemetryEvent::TombstoneOpenInWarp, ctx);
                let conversation = BlocklistAIHistoryModel::handle(ctx)
                    .as_ref(ctx)
                    .conversation(conversation_id);

                if let Some(conversation) = conversation {
                    if let Some(token) = conversation.server_conversation_token() {
                        let url_string = token.conversation_link();
                        if let Ok(url) = url::Url::parse(&url_string) {
                            ctx.dispatch_typed_action(&WorkspaceAction::OpenLinkOnDesktop(url));
                        } else {
                            log::error!("Failed to parse conversation URL: {}", url_string);
                        }
                    } else {
                        log::warn!("No server conversation token available for conversation");
                    }
                } else {
                    log::error!("Conversation not found in history model");
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "conversation_ended_tombstone_view_tests.rs"]
mod tests;
