//! TUI voice-input lifecycle and async task ownership.

use std::time::Duration;

use warp::settings::AISettings;
pub(crate) use warp::tui_export::VoiceInputLifecycleState as TuiVoiceInputState;
use warp::tui_export::{
    AIRequestUsageModel, StartListeningError, TranscribeError, UserWorkspaces, VoiceInput,
    VoiceInputToggledFrom, VoiceSessionResult, VoiceTranscriber,
};
use warp_errors::report_error;
use warpui::event::KeyState;
use warpui_core::r#async::SpawnedFutureHandle;
use warpui_core::elements::animation::AnimationClock;
use warpui_core::{Entity, ModelContext, SingletonEntity};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TuiVoiceInputEvent {
    StateChanged(TuiVoiceInputState),
    Completed(String),
    Failed(String),
    Cancelled,
}

#[derive(Clone, Copy)]
pub(crate) enum VoiceInputStartSource {
    SlashCommand,
    Keybinding,
}

impl VoiceInputStartSource {
    pub(crate) fn clears_input(self) -> bool {
        matches!(self, Self::SlashCommand)
    }

    fn toggled_from(self) -> VoiceInputToggledFrom {
        match self {
            Self::SlashCommand => VoiceInputToggledFrom::Button,
            Self::Keybinding => VoiceInputToggledFrom::Key {
                state: KeyState::Pressed,
            },
        }
    }
}

pub(crate) struct TuiVoiceInputModel {
    state: TuiVoiceInputState,
    animation_clock: AnimationClock,
    recording_handle: Option<SpawnedFutureHandle>,
    transcription_handle: Option<SpawnedFutureHandle>,
}

impl Entity for TuiVoiceInputModel {
    type Event = TuiVoiceInputEvent;
}

impl TuiVoiceInputModel {
    pub(crate) fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            state: TuiVoiceInputState::Idle,
            animation_clock: AnimationClock::starting_at(Duration::ZERO),
            recording_handle: None,
            transcription_handle: None,
        }
    }

    pub(crate) fn state(&self) -> TuiVoiceInputState {
        self.state
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

        self.state = TuiVoiceInputState::Listening;
        self.animation_clock = AnimationClock::starting_at(Duration::ZERO);
        ctx.emit(TuiVoiceInputEvent::StateChanged(self.state));
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

        self.state = TuiVoiceInputState::Transcribing;
        ctx.emit(TuiVoiceInputEvent::StateChanged(self.state));
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
        self.state = TuiVoiceInputState::Idle;
        if emit_cancelled {
            ctx.emit(TuiVoiceInputEvent::Cancelled);
        }
        ctx.emit(TuiVoiceInputEvent::StateChanged(self.state));
    }

    fn handle_session_result(&mut self, result: VoiceSessionResult, ctx: &mut ModelContext<Self>) {
        self.recording_handle = None;
        if self.state != TuiVoiceInputState::Transcribing {
            return;
        }

        let VoiceSessionResult::Audio { wav_base64, .. } = result else {
            self.fail("Voice input stopped", ctx);
            return;
        };
        let Some(transcriber) = VoiceTranscriber::as_ref(ctx).transcriber().cloned() else {
            self.fail("Voice transcription is unavailable", ctx);
            return;
        };
        let language = AISettings::as_ref(ctx)
            .voice_input_language_code()
            .map(str::to_owned);
        VoiceInput::handle(ctx).update(ctx, |voice_input, _| {
            voice_input.set_transcribing_active(true);
        });
        self.transcription_handle = Some(ctx.spawn(
            async move { transcriber.transcribe(wav_base64, language).await },
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
        self.state = TuiVoiceInputState::Idle;
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
        ctx.emit(TuiVoiceInputEvent::StateChanged(self.state));
    }

    fn fail(&mut self, hint: &str, ctx: &mut ModelContext<Self>) {
        if self.state == TuiVoiceInputState::Idle {
            return;
        }
        self.state = TuiVoiceInputState::Idle;
        ctx.emit(TuiVoiceInputEvent::Failed(hint.to_owned()));
        ctx.emit(TuiVoiceInputEvent::StateChanged(self.state));
    }

    #[cfg(test)]
    pub(crate) fn set_state_for_test(
        &mut self,
        state: TuiVoiceInputState,
        ctx: &mut ModelContext<Self>,
    ) {
        self.state = state;
        if state == TuiVoiceInputState::Listening {
            self.animation_clock = AnimationClock::starting_at(Duration::ZERO);
        }
        ctx.emit(TuiVoiceInputEvent::StateChanged(self.state));
    }
}

#[cfg(test)]
#[path = "voice_input_tests.rs"]
mod tests;
