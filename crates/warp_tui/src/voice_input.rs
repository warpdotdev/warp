//! TUI voice-input lifecycle and async task ownership.

use std::time::Duration;

use warp::settings::{AISettings, TuiVoiceSettings};
pub(crate) use warp::tui_export::VoiceInputLifecycleState as TuiVoiceInputState;
use warp::tui_export::{
    AIRequestUsageModel, BlocklistAIInputModel, RequestTeamScope, StartListeningError,
    TeamContextResolver, TelemetryEvent, TranscribeError, UserWorkspaces, VoiceInput,
    VoiceInputToggledFrom, VoiceSessionResult, VoiceTranscriber,
};
use warp_core::settings::Setting as _;
use warp_errors::report_error;
use warpui::event::KeyState;
use warpui_core::r#async::SpawnedFutureHandle;
use warpui_core::elements::animation::AnimationClock;
use warpui_core::platform::keyboard::KeyCode;
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TuiVoiceInputEvent {
    StateChanged(TuiVoiceInputState),
    Completed(String),
    Failed(String),
    Cancelled,
}

/// The physical modifier hold-to-talk is configured to use, if any.
pub(crate) fn configured_hold_key(ctx: &AppContext) -> Option<KeyCode> {
    (*TuiVoiceSettings::as_ref(ctx).voice_input_hold_key.value()).into()
}

/// Whether hold-to-talk needs the terminal to report modifier press and
/// release events.
pub(crate) fn requires_modifier_key_reporting(ctx: &AppContext) -> bool {
    configured_hold_key(ctx).is_some()
}

#[derive(Clone, Copy)]
pub(crate) enum VoiceInputStartSource {
    SlashCommand,
    Keybinding,
    /// A hold-to-talk press of the given physical modifier, which keeps the
    /// recording open until that same modifier is released.
    HoldKey(KeyCode),
    Button,
}

impl VoiceInputStartSource {
    pub(crate) fn clears_input(self) -> bool {
        matches!(self, Self::SlashCommand)
    }

    fn hold_key(self) -> Option<KeyCode> {
        match self {
            Self::HoldKey(key) => Some(key),
            Self::SlashCommand | Self::Keybinding | Self::Button => None,
        }
    }

    fn toggled_from(self) -> VoiceInputToggledFrom {
        match self {
            Self::SlashCommand | Self::Button => VoiceInputToggledFrom::Button,
            Self::Keybinding | Self::HoldKey(_) => VoiceInputToggledFrom::Key {
                state: KeyState::Pressed,
            },
        }
    }
}

pub(crate) struct TuiVoiceInputModel {
    state: TuiVoiceInputState,
    input_mode: ModelHandle<BlocklistAIInputModel>,
    /// The physical modifier holding the current recording open. Only ever set
    /// while a hold-to-talk press owns a `Listening` session, so a release can
    /// never stop a recording another entry point started.
    hold_key: Option<KeyCode>,
    animation_clock: AnimationClock,
    recording_handle: Option<SpawnedFutureHandle>,
    transcription_handle: Option<SpawnedFutureHandle>,
    /// Resolves this model's team context on demand, so transcription requests are scoped to
    /// the owning input view's window rather than an ambient, unscoped workspace read.
    team_context_resolver: TeamContextResolver,
}

impl Entity for TuiVoiceInputModel {
    type Event = TuiVoiceInputEvent;
}

impl TuiVoiceInputModel {
    pub(crate) fn new(
        input_mode: ModelHandle<BlocklistAIInputModel>,
        team_context_resolver: TeamContextResolver,
        _ctx: &mut ModelContext<Self>,
    ) -> Self {
        Self {
            state: TuiVoiceInputState::Idle,
            input_mode,
            hold_key: None,
            animation_clock: AnimationClock::starting_at(Duration::ZERO),
            recording_handle: None,
            transcription_handle: None,
            team_context_resolver,
        }
    }

    pub(crate) fn state(&self) -> TuiVoiceInputState {
        self.state
    }

    pub(crate) fn hold_key(&self) -> Option<KeyCode> {
        self.hold_key
    }

    pub(crate) fn is_active(&self) -> bool {
        self.state != TuiVoiceInputState::Idle
    }

    pub(crate) fn animation_clock(&self) -> AnimationClock {
        self.animation_clock
    }

    pub(crate) fn start(
        &mut self,
        local_skills_available: bool,
        source: VoiceInputStartSource,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if self.is_active() {
            return false;
        }

        let available = local_skills_available
            && AISettings::as_ref(ctx).is_voice_input_enabled(ctx)
            && UserWorkspaces::as_ref(ctx).is_voice_enabled()
            && AIRequestUsageModel::as_ref(ctx).can_request_voice();
        if !available {
            ctx.emit(TuiVoiceInputEvent::Failed(
                "Voice input is unavailable".to_owned(),
            ));
            return false;
        }

        let session_result = VoiceInput::handle(ctx).update(ctx, |voice_input, ctx| {
            voice_input.start_listening(ctx, source.toggled_from())
        });
        let session = match session_result {
            Ok(session) => session,
            Err(error) => {
                let hint = match error {
                    StartListeningError::AccessDenied => "Microphone access denied",
                    StartListeningError::AlreadyRunning | StartListeningError::Other(_) => {
                        "Unable to start voice input"
                    }
                };
                ctx.emit(TuiVoiceInputEvent::Failed(hint.to_owned()));
                return false;
            }
        };

        self.hold_key = source.hold_key();
        self.animation_clock = AnimationClock::starting_at(Duration::ZERO);
        self.set_state(TuiVoiceInputState::Listening, ctx);
        warp::send_telemetry_from_ctx!(
            TelemetryEvent::VoiceInputUsed {
                action: "start".to_owned(),
                session_duration_ms: None,
                is_udi_enabled: false,
                current_input_mode: self.input_mode.as_ref(ctx).input_type(),
            },
            ctx
        );
        self.recording_handle = Some(ctx.spawn(
            async move { session.await_result().await },
            Self::handle_session_result,
        ));
        true
    }

    pub(crate) fn stop(&mut self, ctx: &mut ModelContext<Self>) {
        if self.state != TuiVoiceInputState::Listening {
            return;
        }

        let result =
            VoiceInput::handle(ctx).update(ctx, |voice_input, ctx| voice_input.stop_listening(ctx));
        if let Err(error) = result {
            VoiceInput::handle(ctx).update(ctx, |voice_input, _| {
                voice_input.abort_listening();
            });
            self.fail("Failed to stop voice input", ctx);
            report_error!(error.context("Failed to stop TUI voice input"));
            return;
        }

        self.set_state(TuiVoiceInputState::Transcribing, ctx);
    }

    /// Applies a hold-to-talk press or release of the physical modifier `key`.
    /// A press is ignored while any voice session is already active, and a
    /// release only stops the recording that the same key started.
    pub(crate) fn handle_hold_key(
        &mut self,
        key: KeyCode,
        state: KeyState,
        local_skills_available: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        match state {
            KeyState::Pressed => {
                if self.hold_key.is_none() {
                    self.start(
                        local_skills_available,
                        VoiceInputStartSource::HoldKey(key),
                        ctx,
                    );
                }
            }
            KeyState::Released => {
                if self.hold_key == Some(key) {
                    self.stop(ctx);
                }
            }
        }
    }

    /// Stops a recording that a hold press is keeping open, leaving recordings
    /// started by any other entry point running.
    pub(crate) fn stop_hold(&mut self, ctx: &mut ModelContext<Self>) {
        if self.hold_key.is_some() {
            self.stop(ctx);
        }
    }

    pub(crate) fn cancel(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_active() {
            return;
        }
        self.abort_active(true, ctx);
    }

    fn abort_active(&mut self, emit_cancelled: bool, ctx: &mut ModelContext<Self>) {
        match self.state {
            TuiVoiceInputState::Listening => {
                VoiceInput::handle(ctx).update(ctx, |voice_input, _| {
                    voice_input.abort_listening();
                });
            }
            TuiVoiceInputState::Transcribing => {
                VoiceInput::handle(ctx).update(ctx, |voice_input, _| {
                    voice_input.set_transcribing_active(false);
                });
            }
            TuiVoiceInputState::Idle => return,
        }
        if let Some(handle) = self.recording_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.transcription_handle.take() {
            handle.abort();
        }
        self.set_state(TuiVoiceInputState::Idle, ctx);
        if emit_cancelled {
            ctx.emit(TuiVoiceInputEvent::Cancelled);
        }
    }

    /// Applies a lifecycle transition and announces it, dropping the
    /// hold-to-talk marker whenever the recording it holds open ends. Any other
    /// state a subscriber reads for the new lifecycle state — the animation
    /// clock, the hold marker — must be assigned before this call.
    fn set_state(&mut self, state: TuiVoiceInputState, ctx: &mut ModelContext<Self>) {
        self.state = state;
        if state != TuiVoiceInputState::Listening {
            self.hold_key = None;
        }
        ctx.emit(TuiVoiceInputEvent::StateChanged(state));
    }

    fn handle_session_result(&mut self, result: VoiceSessionResult, ctx: &mut ModelContext<Self>) {
        self.recording_handle = None;
        if self.state != TuiVoiceInputState::Transcribing {
            return;
        }

        let wav_base64 = match result {
            VoiceSessionResult::Audio {
                wav_base64,
                session_duration_ms,
            } => {
                warp::send_telemetry_from_ctx!(
                    TelemetryEvent::VoiceInputUsed {
                        action: "stop".to_owned(),
                        session_duration_ms: Some(session_duration_ms),
                        is_udi_enabled: false,
                        current_input_mode: self.input_mode.as_ref(ctx).input_type(),
                    },
                    ctx
                );
                wav_base64
            }
            VoiceSessionResult::Aborted {
                session_duration_ms,
            } => {
                warp::send_telemetry_from_ctx!(
                    TelemetryEvent::VoiceInputUsed {
                        action: "cancel".to_owned(),
                        session_duration_ms,
                        is_udi_enabled: false,
                        current_input_mode: self.input_mode.as_ref(ctx).input_type(),
                    },
                    ctx
                );
                self.fail("Voice input stopped", ctx);
                return;
            }
        };
        let Some(transcriber) = VoiceTranscriber::as_ref(ctx).transcriber().cloned() else {
            self.fail("Voice transcription is unavailable", ctx);
            return;
        };
        let language = AISettings::as_ref(ctx)
            .voice_input_language_code()
            .map(str::to_owned);
        let team_scope = RequestTeamScope::from_scope(&(self.team_context_resolver)(ctx));
        VoiceInput::handle(ctx).update(ctx, |voice_input, _| {
            voice_input.set_transcribing_active(true);
        });
        self.transcription_handle = Some(ctx.spawn(
            async move {
                transcriber
                    .transcribe(wav_base64, language, team_scope)
                    .await
            },
            Self::handle_transcription_result,
        ));
    }

    fn handle_transcription_result(
        &mut self,
        result: Result<String, TranscribeError>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.transcription_handle = None;
        if self.state != TuiVoiceInputState::Transcribing {
            return;
        }
        VoiceInput::handle(ctx).update(ctx, |voice_input, _| {
            voice_input.set_transcribing_active(false);
        });
        self.set_state(TuiVoiceInputState::Idle, ctx);
        match result {
            Ok(text) => ctx.emit(TuiVoiceInputEvent::Completed(text)),
            Err(error) => {
                let hint = match error {
                    TranscribeError::QuotaLimit => "Voice input limit reached",
                    TranscribeError::ServerOverloaded => "Voice transcription is unavailable",
                    _ => "Failed to transcribe voice input",
                };
                ctx.emit(TuiVoiceInputEvent::Failed(hint.to_owned()));
            }
        }
    }

    fn fail(&mut self, hint: &str, ctx: &mut ModelContext<Self>) {
        if self.state == TuiVoiceInputState::Idle {
            return;
        }
        self.set_state(TuiVoiceInputState::Idle, ctx);
        ctx.emit(TuiVoiceInputEvent::Failed(hint.to_owned()));
    }

    #[cfg(test)]
    pub(crate) fn set_state_for_test(
        &mut self,
        state: TuiVoiceInputState,
        ctx: &mut ModelContext<Self>,
    ) {
        if state == TuiVoiceInputState::Listening {
            self.animation_clock = AnimationClock::starting_at(Duration::ZERO);
        }
        self.set_state(state, ctx);
    }

    #[cfg(test)]
    pub(crate) fn set_hold_key_for_test(&mut self, hold_key: Option<KeyCode>) {
        self.hold_key = hold_key;
    }
}

#[cfg(all(test, feature = "voice_input"))]
#[path = "voice_input_tests.rs"]
mod tests;
