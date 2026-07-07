// TODO(roland): Delete all of this once agent mode fully replaces the AI assistant panel.
// app/src/ai/request_usage_model duplicates much of this logic.
use std::sync::Arc;

use anyhow::Result;
use chrono::{OutOfRangeError, Utc};
use futures::stream::AbortHandle;
use warp_core::user_preferences::GetUserPreferences as _;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use super::execution_context::WarpAiExecutionContext;
use super::utils::{markdown_segments_from_text, FormattedTranscriptMessage, TranscriptPart};
use crate::ai::{RequestLimitInfo, RequestUsageInfo};
use crate::ai_assistant::utils::{AssistantTranscriptPart, TranscriptPartSubType};
use crate::auth::AuthStateProvider;
use crate::send_telemetry_from_ctx;
use crate::server::server_api::ai::AIClient;
use crate::server::server_api::ServerApi;
use crate::server::telemetry::{TelemetryEvent, WarpAIRequestResult};
use crate::workspaces::user_workspaces::UserWorkspaces;

/// The key for the corresponding entry in UserDefaults.
/// Not wiring through Settings for now since this data is only needed by the panel view.
pub const REQUEST_LIMIT_INFO_CACHE_KEY: &str = "AIAssistantRequestLimitInfo";

/// Tracks the current request status for making Warp AI requests against server.
pub enum RequestStatus {
    /// There isn't a request in flight right now.
    NotInFlight,

    /// There's currently a request in flight.
    InFlight {
        /// The request itself (i.e. the prompt).
        request: FormattedTranscriptMessage,
        /// A handle to abort the request if desired.
        abort_handle: AbortHandle,
    },
}

fn cache_request_limit_info(request_limit_info: RequestLimitInfo, app_mut: &mut AppContext) {
    if let Ok(serialized) = serde_json::to_string(&request_limit_info) {
        let _ = app_mut
            .private_user_preferences()
            .write_value(REQUEST_LIMIT_INFO_CACHE_KEY, serialized);
    }
}

fn get_cached_request_limit_info(app_mut: &mut AppContext) -> Option<RequestLimitInfo> {
    app_mut
        .private_user_preferences()
        .read_value(REQUEST_LIMIT_INFO_CACHE_KEY)
        .unwrap_or_default()
        .and_then(|serialized| serde_json::from_str(serialized.as_str()).ok())
}

#[derive(Debug, Clone)]
pub enum GenerateDialogueResult {
    Success {
        answer: String,
        truncated: bool,
        request_limit_info: RequestLimitInfo,
        transcript_summarized: bool,
    },
    Failure {
        request_limit_info: RequestLimitInfo,
    },
}

pub struct Requests {
    server_api: Arc<ServerApi>,
    ai_client: Arc<dyn AIClient>,
    request_status: RequestStatus,
    request_limit_info: RequestLimitInfo,

    /// The currently displayed transcript.
    current_transcript: Vec<TranscriptPart>,

    /// Has the server summarized the current transcript because it's running long?
    current_transcript_summarized: bool,

    /// When a user Restarts their transcript, we still remember
    /// the previous transcript parts for things like suggestions.
    /// This list is mutually exclusive from current_transcript.  
    old_transcript_parts: Vec<TranscriptPart>,

    ai_execution_context: Option<WarpAiExecutionContext>,
}

impl Entity for Requests {
    type Event = Event;
}

pub enum Event {
    RequestFinished { succeeded: bool },
}

/// Private interface.
impl Requests {
    fn remaining_time_to_refresh_std(&self) -> Result<std::time::Duration, OutOfRangeError> {
        self.request_limit_info
            .next_refresh_time
            .utc()
            .signed_duration_since(Utc::now())
            .to_std()
    }
}

/// Public interface.
impl Requests {
    pub fn new(
        server_api: Arc<ServerApi>,
        ai_client: Arc<dyn AIClient>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        // Check if the user has cached request limit info from before.
        // If not, let's just make an assumption about the server's default request limit
        // and fetch the true request limit later.
        let cached_request_limit_info = get_cached_request_limit_info(ctx);
        let request_limit_info = cached_request_limit_info.unwrap_or_default();

        let requests = Self {
            server_api,
            ai_client,
            current_transcript: Vec::new(),
            current_transcript_summarized: false,
            old_transcript_parts: Vec::new(),
            request_status: RequestStatus::NotInFlight,
            request_limit_info,
            ai_execution_context: None,
        };

        if cached_request_limit_info.is_none()
            && AuthStateProvider::as_ref(ctx).get().is_logged_in()
        {
            let ai_client = requests.ai_client.clone();
            let _ = ctx.spawn(
                async move { ai_client.get_request_limit_info().await },
                Self::update_request_limit_info,
            );
        }
        requests
    }

    pub fn update_ai_execution_context(
        &mut self,
        ai_execution_context: Option<WarpAiExecutionContext>,
    ) {
        self.ai_execution_context = ai_execution_context;
    }

    pub fn update_request_limit_info(
        &mut self,
        result: Result<RequestUsageInfo>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Ok(usage_info) => {
                self.request_limit_info = usage_info.request_limit_info;
                ctx.notify();
                cache_request_limit_info(usage_info.request_limit_info, ctx);
            }
            Err(e) => {
                log::warn!("Failed to retrieve initial request limit info: {e:#}");
            }
        }
    }

    /// Starts a Warp AI request against the server with the given request prompt.
    pub fn issue_request(&mut self, request: String, ctx: &mut ModelContext<Self>) {
        let server_api = self.server_api.clone();
        let raw_request = request.trim();
        let request_for_api = raw_request.to_string();
        let transcript = self.current_transcript.clone();
        let transcript_part_index = transcript.len();
        let ai_execution_context = self.ai_execution_context.clone();

        let request_in_markdown = markdown_segments_from_text(
            transcript_part_index,
            TranscriptPartSubType::Question,
            raw_request,
        );

        let future_handle = ctx.spawn(
            async move {
                let start_time = Utc::now();
                (start_time, server_api
                    .generate_dialogue_answer(transcript, request_for_api, ai_execution_context)
                    .await)
            },
            move |model, (start_time, response), ctx| {
                let succeeded = response.is_ok();
                let end_time = Utc::now();
                let mut current_request_status = RequestStatus::NotInFlight;
                std::mem::swap(&mut model.request_status, &mut current_request_status);
                if let RequestStatus::InFlight { request, .. } = current_request_status {
                    match response {
                        Ok(GenerateDialogueResult::Success {
                            mut answer,
                            truncated,
                            request_limit_info,
                            transcript_summarized,
                        }) => {
                            if truncated {
                                answer.push_str("...");
                            }

                            let trimmed_response = answer.trim();
                            let response_in_markdown = markdown_segments_from_text(
                                transcript_part_index,
                                TranscriptPartSubType::Answer,
                                trimmed_response,
                            );
                            model.current_transcript.push(TranscriptPart {
                                user: request,
                                assistant: AssistantTranscriptPart {
                                    is_error: false,
                                    copy_all_tooltip_and_button_mouse_handles: Some((Default::default(), Default::default())),
                                    formatted_message: FormattedTranscriptMessage {
                                        markdown: response_in_markdown,
                                        raw: trimmed_response.to_string(),
                                    },
                                },
                            });

                            cache_request_limit_info(request_limit_info, ctx);
                            model.request_limit_info = request_limit_info;

                            // If the transcript was already marked as summarized before,
                            // it will remain so until it's reset.
                            model.current_transcript_summarized |= transcript_summarized;


                            let req_latency = end_time.signed_duration_since(start_time).num_milliseconds();
                            send_telemetry_from_ctx!(
                                TelemetryEvent::WarpAIRequestIssued { result: WarpAIRequestResult::Succeeded { latency_ms: req_latency, truncated }},
                                ctx
                            );
                        }
                        Ok(GenerateDialogueResult::Failure { request_limit_info }) if request_limit_info.limit <= request_limit_info.num_requests_used_since_refresh => {
                            cache_request_limit_info(request_limit_info, ctx);
                            model.request_limit_info = request_limit_info;
                            let next_time = if let Some(next_refresh_time) = model.serialized_time_until_refresh() {
                                format!("after {next_refresh_time}")
                            } else {
                                String::from("later")
                            };

                            let auth_state = AuthStateProvider::as_ref(ctx).get();
                            let response = if let Some(team) = UserWorkspaces::as_ref(ctx).current_team() {
                                let current_user_email = auth_state.user_email().unwrap_or_default();
                                let has_admin_permissions = team.has_admin_permissions(&current_user_email);
                                if team.billing_metadata.can_upgrade_to_higher_tier_plan() {
                                    if has_admin_permissions {
                                        let upgrade_url = UserWorkspaces::upgrade_link_for_team(team.uid);
                                        format!("It seems you're out of credits. Please try again {next_time}.\n\n[Upgrade]({upgrade_url}) for more credits.")
                                    } else {
                                        format!("It seems you're out of credits. Please try again {next_time}.\n\nContact a team admin to upgrade for more credits.")
                                    }
                                } else {
                                    format!("It seems you're out of credits. Please try again {next_time}.")
                                }
                            } else {
                                let user_id = auth_state.user_id().unwrap_or_default();
                                let upgrade_url = UserWorkspaces::upgrade_link(user_id);
                                format!("It seems you're out of credits. Please try again {next_time}.\n\n[Upgrade]({upgrade_url}) for more credits.")
                            };
                            let response_in_markdown = markdown_segments_from_text(
                                transcript_part_index,
                                TranscriptPartSubType::Answer,
                                &response,
                            );
                            model.current_transcript.push(TranscriptPart {
                                user: request,
                                assistant: AssistantTranscriptPart {
                                    is_error: true,
                                    copy_all_tooltip_and_button_mouse_handles: None,
                                    formatted_message: FormattedTranscriptMessage {
                                        markdown: response_in_markdown,
                                        raw: response,
                                    },
                                },
                            });

                            send_telemetry_from_ctx!(
                                TelemetryEvent::WarpAIRequestIssued { result: WarpAIRequestResult::OutOfRequests},
                                ctx
                            );
                        }
                        _ => {
                            let response = "We're experiencing technical difficulties right now. Please try again later.".to_owned();
                            let response_in_markdown = markdown_segments_from_text(
                                transcript_part_index,
                                TranscriptPartSubType::Answer,
                                &response,
                            );
                            model.current_transcript.push(TranscriptPart {
                                user: request,
                                assistant: AssistantTranscriptPart {
                                    is_error: true,
                                    copy_all_tooltip_and_button_mouse_handles: None,
                                    formatted_message: FormattedTranscriptMessage {
                                        markdown: response_in_markdown,
                                        raw: response,
                                    },
                                },
                            });

                            send_telemetry_from_ctx!(
                                TelemetryEvent::WarpAIRequestIssued { result: WarpAIRequestResult::Failed},
                                ctx
                            );
                        }
                    }
                }

                ctx.emit(Event::RequestFinished { succeeded });
                ctx.notify();
            },
        );

        self.request_status = RequestStatus::InFlight {
            request: FormattedTranscriptMessage {
                markdown: request_in_markdown,
                raw: raw_request.to_string(),
            },
            abort_handle: future_handle.abort_handle(),
        };

        ctx.notify();
    }

    pub fn reset(&mut self, ctx: &mut ModelContext<Self>) {
        if let RequestStatus::InFlight { abort_handle, .. } = &self.request_status {
            abort_handle.abort();
        }
        let mut old_transcript = Vec::new();
        std::mem::swap(&mut old_transcript, &mut self.current_transcript);
        self.old_transcript_parts.extend(old_transcript);
        self.request_status = RequestStatus::NotInFlight;
        self.current_transcript_summarized = false;
        ctx.notify();
    }

    pub fn transcript(&self) -> &[TranscriptPart] {
        self.current_transcript.as_slice()
    }

    /// Includes the old transcript parts appended with the current
    /// transcript parts. You likely want to just be using the current transcript parts
    /// (exposed by the `Requests::transcript` API) in most use cases.
    fn total_transcript_history(&self) -> impl Iterator<Item = &TranscriptPart> {
        self.old_transcript_parts
            .iter()
            .chain(self.current_transcript.iter())
    }

    pub fn all_past_transcript_prompts(&self) -> Vec<String> {
        self.total_transcript_history()
            .map(|p| p.raw_user_prompt().to_string())
            .collect()
    }

    pub fn request_status(&self) -> &RequestStatus {
        &self.request_status
    }

    pub fn current_transcript_summarized(&self) -> bool {
        self.current_transcript_summarized
    }

    /// Returns the number of remaining requests the user has based on their latest rate limit info.
    /// If the current time is past the next refresh time, then the number of remaining reqs is the limit.
    pub fn num_remaining_reqs(&self) -> usize {
        match self.remaining_time_to_refresh_std() {
            Err(_) => self.request_limit_info.limit,
            Ok(t) if t.is_zero() => self.request_limit_info.limit,
            Ok(_t) => {
                self.request_limit_info.limit
                    - self.request_limit_info.num_requests_used_since_refresh
            }
        }
    }

    pub fn num_requests_used(&self) -> usize {
        self.request_limit_info.limit - self.num_remaining_reqs()
    }

    pub fn request_limit(&self) -> usize {
        self.request_limit_info.limit
    }

    /// Returns the next refresh time based on the latest rate limit info as a formatted string.
    /// If the current time is past the next refresh time, then returns None.
    pub fn serialized_time_until_refresh(&self) -> Option<String> {
        match self.remaining_time_to_refresh_std() {
            Err(_) => None,
            Ok(t) if t.is_zero() => None,
            Ok(t) => {
                let num_minutes = t.as_secs() / 60;
                let num_hours = num_minutes / 60;
                let num_days = num_hours / 24;
                let remaining_text = if num_days > 0 {
                    format!("{num_days} days")
                } else if num_hours > 0 {
                    format!("{num_hours} hours")
                } else {
                    format!("{num_minutes} minutes")
                };
                Some(remaining_text)
            }
        }
    }
}

#[cfg(test)]
impl Requests {
    pub fn new_with_transcript(transcript: Vec<TranscriptPart>) -> Self {
        use crate::server::server_api::ServerApiProvider;

        Self {
            server_api: ServerApiProvider::new_for_test().get(),
            ai_client: ServerApiProvider::new_for_test().get_ai_client(),
            current_transcript: transcript,
            current_transcript_summarized: false,
            old_transcript_parts: Vec::new(),
            request_status: RequestStatus::NotInFlight,
            request_limit_info: RequestLimitInfo::default(),
            ai_execution_context: None,
        }
    }
}
