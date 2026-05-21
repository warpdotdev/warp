use super::{EditorAction, EditorView, VoiceTranscriptionOptions};
use crate::ai::blocklist::InputType;
use crate::appearance::Appearance;
use crate::editor::EditorElement;
use crate::server::server_api::TranscribeError;
use crate::server::telemetry::TelemetryEvent;
use crate::settings::{AISettings, VoiceInputToggleKey};
use crate::themes::theme::Fill;
use crate::ui_components::buttons::{icon_button, icon_button_with_color};
use crate::ui_components::icons;
use crate::view_components::{FeaturePopup, NewFeaturePopupLabel};
use crate::workspace::ToastStack;
use crate::workspaces::user_workspaces::UserWorkspaces;
use settings::Setting as _;
use voice_input::{StartListeningError, VoiceInput, VoiceSessionResult};
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::AnsiColorIdentifier;
use warpui::elements;
use warpui::elements::{Container, CornerRadius, Icon, Radius};
use warpui::platform::Cursor;
use warpui::r#async::SpawnedFutureHandle;
use warpui::ui_components::button::ButtonTooltipPosition;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ViewHandle;
use warpui::{AppContext, Element, SingletonEntity, ViewContext};

use super::VoiceTranscriber;

const MICROPHONE_ACCESS_ERROR_ID: &str = "MICROPHONE_ACCESS_ERROR";
const NUM_TIMES_TO_SHOW_VOICE_NEW_FEATURE_POPUP: usize = 4;

#[derive(Debug, Default, Clone)]
pub(super) enum VoiceInputState {
    #[default]
    Stopped,

    /// We are listening for voice input. This is happening in the singleton voice transcriber.
    Listening,

    /// We are done listening and are transcribing voice input.
    Transcribing {
        /// The handle to the future that is spawned for voice input while transcribing is taking place.
        handle: SpawnedFutureHandle,
    },
}

impl VoiceInputState {
    pub(super) fn is_active(&self) -> bool {
        matches!(
            self,
            VoiceInputState::Listening | VoiceInputState::Transcribing { .. }
        )
    }

    pub(super) fn icon(&self) -> Option<icons::Icon> {
        match self {
            VoiceInputState::Listening => Some(icons::Icon::Microphone),
            VoiceInputState::Transcribing { .. } => Some(icons::Icon::DotsHorizontal),
            VoiceInputState::Stopped => None,
        }
    }
}

impl EditorView {
    pub(super) fn is_voice_input_active(&self) -> bool {
        self.voice_input_state.is_active()
    }

    pub(super) fn create_voice_new_feature_popup(
        ctx: &mut ViewContext<EditorView>,
    ) -> ViewHandle<FeaturePopup> {
        let voice_new_feature_popup = ctx.add_typed_action_view(|_| {
            FeaturePopup::new_feature(NewFeaturePopupLabel::FromString(
                "Try Voice Input".to_string(),
            ))
        });

        ctx.subscribe_to_view(&voice_new_feature_popup, |_me, _, event, ctx| {
            if matches!(
                event,
                crate::view_components::NewFeaturePopupEvent::Dismissed
            ) {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    warp_core::report_if_error!(settings
                        .dismissed_voice_input_new_feature_popup
                        .set_value(true, ctx));
                });
                ctx.notify();
            }
        });

        voice_new_feature_popup
    }

    pub(super) fn should_show_voice_new_feature_popup(&self, app: &AppContext) -> bool {
        let ai_settings = AISettings::handle(app).as_ref(app);
        let voice_input = voice_input::VoiceInput::handle(app).as_ref(app);

        let num_times_entered_agent_mode = *ai_settings.entered_agent_mode_num_times;
        let manually_dismissed_voice_input_new_feature_popup =
            *ai_settings.dismissed_voice_input_new_feature_popup;
        let explicitly_interacted_with_voice = *ai_settings.explicitly_interacted_with_voice;

        num_times_entered_agent_mode <= NUM_TIMES_TO_SHOW_VOICE_NEW_FEATURE_POPUP
            && !manually_dismissed_voice_input_new_feature_popup
            && !explicitly_interacted_with_voice
            && !voice_input.should_suppress_new_feature_popup
    }

    /// Configures an [`EditorElement`] for the current voice input state.
    pub(super) fn configure_editor_element_voice(
        &self,
        editor_element: EditorElement,
        appearance: &Appearance,
    ) -> EditorElement {
        if let Some(icon) = self.voice_input_state.icon() {
            editor_element.with_voice_input_cursor_icon(
                Container::new(
                    Icon::new(icon.into(), internal_colors::neutral_1(appearance.theme())).finish(),
                )
                .with_background(Fill::Solid(appearance.theme().accent().into()))
                .with_uniform_padding(4.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .finish(),
            )
        } else {
            editor_element
        }
    }

    pub fn update_voice_transcription_options(
        &mut self,
        options: VoiceTranscriptionOptions,
        ctx: &mut ViewContext<Self>,
    ) {
        if !UserWorkspaces::handle(ctx).as_ref(ctx).is_voice_enabled() {
            return;
        }

        log::debug!("update_voice_transcription_options: {options:?}");
        self.voice_transcription_options = options;
        if !self.voice_transcription_options.is_enabled() {
            self.stop_voice_input(true, ctx);
        }
        ctx.notify();
    }

    pub(super) fn voice_options(ctx: &mut ViewContext<Self>) -> VoiceTranscriptionOptions {
        let ai_settings_handle = AISettings::handle(ctx);
        if ai_settings_handle.as_ref(ctx).is_voice_input_enabled(ctx) {
            VoiceTranscriptionOptions::Enabled { show_button: false }
        } else {
            VoiceTranscriptionOptions::Disabled
        }
    }

    pub(super) fn stop_voice_input(
        &mut self,
        cancel_transcription: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if !UserWorkspaces::handle(ctx).as_ref(ctx).is_voice_enabled() {
            return;
        }

        let voice_input = voice_input::VoiceInput::handle(ctx);
        if voice_input.as_ref(ctx).is_listening() {
            log::debug!("Stopping voice input, cancelling transcription: {cancel_transcription}");
            voice_input.update(ctx, |voice_input, ctx| {
                if cancel_transcription {
                    voice_input.abort_listening();
                } else if let Err(e) = voice_input.stop_listening(ctx) {
                    log::error!("Failed to stop voice input: {e:?}");
                }
            });
        }
        if cancel_transcription {
            self.stop_transcribing_voice_input(ctx);
        }
        ctx.notify();
    }

    pub(super) fn stop_transcribing_voice_input(&mut self, ctx: &mut ViewContext<Self>) {
        VoiceInput::handle(ctx).update(ctx, |voice, _| voice.set_transcribing_active(false));
        if let VoiceInputState::Transcribing { handle, .. } = &self.voice_input_state {
            log::debug!("Aborting voice input transcription");
            handle.abort();
        }
        self.set_voice_input_state(VoiceInputState::Stopped, ctx);
        ctx.notify();
    }

    fn voice_error_toast(&mut self, message: &str, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast = crate::view_components::DismissibleToast::error(message.to_string());
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    pub fn toggle_voice_input(
        &mut self,
        source: &voice_input::VoiceInputToggledFrom,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if !UserWorkspaces::handle(ctx).as_ref(ctx).is_voice_enabled() {
            return false;
        }

        if !matches!(
            Self::voice_options(ctx),
            VoiceTranscriptionOptions::Enabled { .. }
        ) {
            return false;
        }

        log::debug!(
            "Toggling voice input from {:?} for current state: {:?}",
            source,
            self.voice_input_state
        );

        match *source {
            voice_input::VoiceInputToggledFrom::Button => {
                // Allow button clicks to focus and start/stop voice input.
                ctx.focus_self();
            }
            voice_input::VoiceInputToggledFrom::Key { state } => {
                // For keypresses, only start voice input if the editor is focused.
                // Note, stopping voice input via keypress is handled in a global handler.
                if !self.focused {
                    return false;
                }

                // If the keypress is not valid in the current state, we ignore it.
                match &self.voice_input_state {
                    // For example, the user could press Fn in a different app, then switch focus
                    // to Warp and let it go - we should NOT activate voice input in this case.
                    VoiceInputState::Stopped => {
                        if matches!(state, warpui::event::KeyState::Released) {
                            return false;
                        }
                    }
                    // Note that in reality, this case is unreachable because we stop voice input
                    // if the user is not focused on Warp (since we lose the ability to listen to modifier
                    // key events). Thus, the user cannot enter a state where we're listening for voice input
                    // but the key is not held already.
                    VoiceInputState::Listening => {
                        if matches!(state, warpui::event::KeyState::Pressed) {
                            return false;
                        }
                    }
                    _ => {}
                }
            }
        }

        match &self.voice_input_state {
            VoiceInputState::Stopped => {
                if !self.voice_transcription_options.is_enabled() {
                    return false;
                }

                if !crate::ai::AIRequestUsageModel::handle(ctx)
                    .as_ref(ctx)
                    .can_request_voice()
                {
                    self.voice_error_toast(super::VOICE_LIMIT_HIT_TOAST_TEXT, ctx);
                    return false;
                }

                // We allow toggling voice input from a button click even if the editor is not focused.
                if self.focused || matches!(*source, voice_input::VoiceInputToggledFrom::Button) {
                    // Try to start voice input and get the session
                    let session_result = voice_input::VoiceInput::handle(ctx)
                        .update(ctx, |voice_input, ctx| {
                            voice_input.start_listening(ctx, source.clone())
                        });

                    let session = match session_result {
                        Ok(session) => session,
                        Err(e) => {
                            match e {
                                StartListeningError::AccessDenied => {
                                    Self::show_microphone_access_toast(ctx);
                                }
                                _ => {
                                    log::error!("Failed to start voice input: {e:?}");
                                }
                            }
                            ctx.notify();
                            return false;
                        }
                    };

                    // Immediately transition to Listening state
                    self.set_voice_input_state(VoiceInputState::Listening, ctx);

                    // Send telemetry for start
                    let is_udi_enabled = crate::settings::InputSettings::handle(ctx)
                        .as_ref(ctx)
                        .is_universal_developer_input_enabled(ctx);
                    let current_input_mode = if self.is_ai_input {
                        InputType::AI
                    } else {
                        InputType::Shell
                    };
                    send_telemetry_from_ctx!(
                        TelemetryEvent::VoiceInputUsed {
                            action: "start".to_string(),
                            session_duration_ms: None,
                            is_udi_enabled,
                            current_input_mode,
                        },
                        ctx
                    );

                    // Spawn future to await the session result
                    ctx.spawn(
                        async move { session.await_result().await },
                        Self::handle_voice_session_result,
                    );

                    if matches!(*source, voice_input::VoiceInputToggledFrom::Button) {
                        // If the user hasn't explicitly interacted with voice yet, show first-time toast.
                        let window_id = ctx.window_id();
                        AISettings::handle(ctx).update(ctx, |settings, ctx| {
                            if let Some(toggle_key) = settings.maybe_setup_first_time_voice(ctx) {
                                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                                    let toast = crate::view_components::DismissibleToast::success(
                                        format!(
                                            "Voice input is enabled. You can also press and hold the `{}` key to activate voice input (configure in Settings > AI > Voice)",
                                            toggle_key.display_name()
                                        )
                                            .to_string(),
                                    );
                                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                                });
                            }
                        });
                    }
                    ctx.notify();
                    return true;
                }
            }
            VoiceInputState::Listening => {
                self.stop_voice_input(false, ctx);
            }
            VoiceInputState::Transcribing { .. } => {
                // Do nothing, we're already transcribing. We don't allow switching states while this is happening.
                // TODO(zach): We may want to show some sort of progress indicator when in this state.
            }
        }
        ctx.notify();
        false
    }

    fn show_microphone_access_toast(ctx: &mut ViewContext<Self>) {
        let active_window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, move |toast_stack, ctx| {
            let mut toast = crate::view_components::DismissibleToast::error(String::from(
                crate::i18n::tr_static(
                    ctx,
                    "Failed to start voice input (you may need to enable Microphone access)",
                ),
            ));
            // Set an id so the toast is shown at most once.
            toast = toast.with_object_id(MICROPHONE_ACCESS_ERROR_ID.to_string());
            toast_stack.add_ephemeral_toast(toast, active_window_id, ctx);
        });
    }

    fn set_voice_input_state(
        &mut self,
        voice_input_state: VoiceInputState,
        ctx: &mut ViewContext<Self>,
    ) {
        let was_active = self.is_voice_input_active();
        let is_listening = matches!(voice_input_state, VoiceInputState::Listening);
        let is_transcribing = matches!(voice_input_state, VoiceInputState::Transcribing { .. });

        let will_be_active = matches!(
            voice_input_state,
            VoiceInputState::Listening | VoiceInputState::Transcribing { .. }
        );

        if !was_active && will_be_active {
            // Lock before marking active, so set_interaction_state applies normally.
            self.interaction_state_before_voice = Some(self.interaction_state(ctx));
            self.set_interaction_state(super::InteractionState::Selectable, ctx);
            self.voice_input_state = voice_input_state;
        } else if was_active && !will_be_active {
            // Mark inactive before restoring, so set_interaction_state applies normally.
            self.voice_input_state = voice_input_state;
            if let Some(state) = self.interaction_state_before_voice.take() {
                self.set_interaction_state(state, ctx);
            }
        } else {
            // Transition between active states (e.g. Listening → Transcribing).
            self.voice_input_state = voice_input_state;
        }

        ctx.emit(super::Event::VoiceStateUpdated {
            is_listening,
            is_transcribing,
        });
    }

    /// Handles the result of a voice recording session.
    /// This is called when the VoiceSession future resolves.
    pub(super) fn handle_voice_session_result(
        &mut self,
        result: VoiceSessionResult,
        ctx: &mut ViewContext<Self>,
    ) {
        if !UserWorkspaces::handle(ctx).as_ref(ctx).is_voice_enabled() {
            return;
        }

        let is_udi_enabled = crate::settings::InputSettings::handle(ctx)
            .as_ref(ctx)
            .is_universal_developer_input_enabled(ctx);
        let current_input_mode = if self.is_ai_input {
            InputType::AI
        } else {
            InputType::Shell
        };

        match result {
            VoiceSessionResult::Audio {
                wav_base64,
                session_duration_ms,
            } => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::VoiceInputUsed {
                        action: "stop".to_string(),
                        session_duration_ms: Some(session_duration_ms),
                        is_udi_enabled,
                        current_input_mode,
                    },
                    ctx
                );

                // Start transcription
                let voice_transcriber = VoiceTranscriber::handle(ctx).as_ref(ctx);
                if let Some(transcriber) = voice_transcriber.transcriber() {
                    let transcriber = transcriber.clone();

                    VoiceInput::handle(ctx).update(ctx, |voice, _| {
                        voice.set_transcribing_active(true);
                    });

                    self.set_voice_input_state(
                        VoiceInputState::Transcribing {
                            handle: ctx.spawn(
                                async move { transcriber.transcribe(wav_base64).await },
                                Self::apply_transcribed_voice_input,
                            ),
                        },
                        ctx,
                    );
                } else {
                    self.set_voice_input_state(VoiceInputState::Stopped, ctx);
                }
            }
            VoiceSessionResult::Aborted {
                session_duration_ms,
            } => {
                log::info!("Aborted listening for voice input");

                send_telemetry_from_ctx!(
                    TelemetryEvent::VoiceInputUsed {
                        action: "cancel".to_string(),
                        session_duration_ms,
                        is_udi_enabled,
                        current_input_mode,
                    },
                    ctx
                );

                self.set_voice_input_state(VoiceInputState::Stopped, ctx);
            }
        }
        ctx.notify();
    }

    fn apply_transcribed_voice_input(
        &mut self,
        result: Result<String, TranscribeError>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.voice_transcription_options.is_enabled() {
            self.stop_transcribing_voice_input(ctx);
            return;
        }

        self.stop_transcribing_voice_input(ctx);
        match result {
            Ok(transcribe_response) => {
                log::debug!("Transcribed voice input: {transcribe_response:?}");
                self.user_insert(&transcribe_response, ctx);
            }
            Err(e) => match e {
                TranscribeError::QuotaLimit => {
                    self.voice_error_toast(super::VOICE_LIMIT_HIT_TOAST_TEXT, ctx)
                }
                _ => {
                    log::error!("Failed to transcribe voice input: {e:?}");
                    self.voice_error_toast(super::VOICE_ERROR_TOAST_TEXT, ctx)
                }
            },
        }
        ctx.notify();
    }

    fn render_voice_transcription_button_tooltip(
        &self,
        appearance: &crate::appearance::Appearance,
        app: &AppContext,
    ) -> Box<dyn FnOnce() -> Box<dyn Element>> {
        let tooltip_background = appearance.theme().surface_1().into_solid();
        let tooltip_text_color = appearance
            .theme()
            .main_text_color(tooltip_background.into())
            .into_solid();
        let ui_builder = appearance.ui_builder().clone();

        let microphone_access_state = app.microphone_access_state();
        let mic_access_denied = matches!(
            microphone_access_state,
            warpui::platform::MicrophoneAccessState::Restricted
                | warpui::platform::MicrophoneAccessState::Denied
        );

        let modifier_key = AISettings::handle(app).as_ref(app).voice_input_toggle_key;
        let tooltip_text = if mic_access_denied {
            "Voice transcription is disabled because Microphone access was not granted.".to_string()
        } else if modifier_key == VoiceInputToggleKey::None {
            "Voice transcription".to_string()
        } else {
            format!(
                "Voice transcription (hold `{}` key)",
                modifier_key.display_name().to_lowercase()
            )
        };

        Box::new(move || {
            let tool_tip_style = UiComponentStyles {
                background: Some(elements::Fill::Solid(tooltip_background)),
                font_color: Some(tooltip_text_color),
                ..Default::default()
            };

            ui_builder
                .tool_tip(tooltip_text)
                .with_style(tool_tip_style)
                .build()
                .finish()
        })
    }

    pub(super) fn render_voice_transcription_button(
        &self,
        icon_size: f32,
        appearance: &crate::appearance::Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut button = if voice_input::VoiceInput::handle(app)
            .as_ref(app)
            .is_listening()
        {
            icon_button_with_color(
                appearance,
                icons::Icon::Stop,
                true,
                self.voice_transcription_button_mouse_handle.clone(),
                Fill::Solid(
                    AnsiColorIdentifier::Red
                        .to_ansi_color(&appearance.theme().terminal_colors().normal)
                        .into(),
                ),
            )
        } else {
            let is_transcribing =
                matches!(self.voice_input_state, VoiceInputState::Transcribing { .. });
            icon_button(
                appearance,
                icons::Icon::Microphone,
                is_transcribing,
                self.voice_transcription_button_mouse_handle.clone(),
            )
        };

        button = button.with_style(UiComponentStyles {
            width: Some(icon_size),
            height: Some(icon_size),
            padding: Some(Coords::uniform(icon_size / 10.)),
            ..Default::default()
        });

        if !self.should_show_voice_new_feature_popup(app) {
            button = button
                .with_tooltip_position(ButtonTooltipPosition::Above)
                .with_tooltip(self.render_voice_transcription_button_tooltip(appearance, app));
        }

        if matches!(self.voice_input_state, VoiceInputState::Transcribing { .. }) {
            button = button.disabled();
        }

        warpui::elements::SavePosition::new(
            button
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(EditorAction::ToggleVoiceInput(
                        voice_input::VoiceInputToggledFrom::Button,
                    ));
                })
                .with_cursor(Cursor::PointingHand)
                .finish(),
            "voice_transcription_button",
        )
        .finish()
    }
}
