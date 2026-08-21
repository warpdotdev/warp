use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use base64::Engine;
use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{FromSample, I24, Sample, SampleFormat, SizedSample, StreamConfig, U24};
use futures::channel::oneshot;
use parking_lot::Mutex;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use thiserror::Error;
use warp_errors::report_error;
use warpui_core::r#async::SpawnedFutureHandle;
use warpui_core::event::KeyState;
use warpui_core::platform::MicrophoneAccessState;
use warpui_core::{Entity, ModelContext, SingletonEntity};

const DEFAULT_CHUNK_SIZE: u32 = 512;
// We only support mono for now.
const NUM_CHANNELS: u16 = 1;
// Voice input is typically sampled at 16000Hz (and required by Wispr)
const TARGET_SAMPLE_RATE: f32 = 16000.0;
const STREAM_TIMEOUT: Duration = Duration::from_secs(60 * 6);

/// Surface-independent voice-input lifecycle state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VoiceInputLifecycleState {
    #[default]
    Idle,
    Listening,
    Transcribing,
}

/// Lifecycle shared by voice-input surfaces.
///
/// Surfaces retain ownership of presentation, telemetry, async handles, and
/// transcription destinations. This type centralizes valid state transitions;
/// surfaces abort their owned handles before cancelling or replacing a session.
#[derive(Debug, Clone, Copy, Default)]
pub struct VoiceInputLifecycle {
    state: VoiceInputLifecycleState,
}

impl VoiceInputLifecycle {
    pub fn state(&self) -> VoiceInputLifecycleState {
        self.state
    }

    pub fn is_active(&self) -> bool {
        self.state != VoiceInputLifecycleState::Idle
    }

    /// Starts listening when idle.
    pub fn start(&mut self) -> bool {
        if self.is_active() {
            return false;
        }
        self.state = VoiceInputLifecycleState::Listening;
        true
    }

    /// Advances listening to transcription.
    pub fn begin_transcribing(&mut self) -> bool {
        if self.state != VoiceInputLifecycleState::Listening {
            return false;
        }
        self.state = VoiceInputLifecycleState::Transcribing;
        true
    }

    /// Completes the active transcription.
    pub fn complete(&mut self) -> bool {
        if self.state != VoiceInputLifecycleState::Transcribing {
            return false;
        }
        self.state = VoiceInputLifecycleState::Idle;
        true
    }

    /// Fails the active session.
    pub fn fail(&mut self) -> bool {
        if !self.is_active() {
            return false;
        }
        self.state = VoiceInputLifecycleState::Idle;
        true
    }

    /// Cancels the current session.
    pub fn cancel(&mut self) -> bool {
        if !self.is_active() {
            return false;
        }
        self.state = VoiceInputLifecycleState::Idle;
        true
    }
}

pub struct VoiceInput {
    state: VoiceInputState,
    pub should_suppress_new_feature_popup: bool,
    voice_session_start: Option<instant::Instant>,
    wav_conversion_handle: Option<SpawnedFutureHandle>,
}

#[derive(Default)]
pub enum VoiceInputState {
    #[default]
    Idle,

    Listening {
        stream: cpal::Stream,
        chunk_size: usize,
        enabled_from: VoiceInputToggledFrom,
        resampler: Arc<Mutex<SincFixedIn<f32>>>,
        resampled: Arc<Mutex<Vec<f32>>>,
        /// Channel to send the result when recording stops.
        result_tx: Option<oneshot::Sender<VoiceSessionResult>>,
    },

    Transcribing,
}

#[derive(Debug, Clone)]
pub enum VoiceInputToggledFrom {
    Button,
    Key { state: KeyState },
}

/// Result of a voice recording session.
#[derive(Debug)]
pub enum VoiceSessionResult {
    /// Recording completed successfully with audio data.
    Audio {
        wav_base64: String,
        session_duration_ms: u64,
    },
    /// Recording was aborted without producing audio.
    Aborted { session_duration_ms: Option<u64> },
}

/// Represents an active voice recording session.
///
/// The caller owns this session and can await the result directly.
/// Dropping the session will prevent the caller from receiving the result,
/// but does not itself stop or abort the underlying recording.
pub struct VoiceSession {
    result_rx: oneshot::Receiver<VoiceSessionResult>,
}

impl VoiceSession {
    /// Awaits the result of the voice recording session.
    ///
    /// Returns `VoiceSessionResult::Audio` if recording completed successfully,
    /// or `VoiceSessionResult::Aborted` if the recording was cancelled.
    pub async fn await_result(self) -> VoiceSessionResult {
        match self.result_rx.await {
            Ok(result) => result,
            // Channel closed without sending - treat as aborted
            Err(_) => VoiceSessionResult::Aborted {
                session_duration_ms: None,
            },
        }
    }
}

/// Error returned when starting voice input fails.
#[derive(Debug, Error)]
pub enum StartListeningError {
    /// Voice input is already running.
    #[error("Voice input is already running")]
    AlreadyRunning,
    /// Microphone access was denied or restricted.
    #[error("Microphone access denied")]
    AccessDenied,
    /// Other error (e.g., no input device, failed to create stream).
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputSampleFormat {
    I8,
    I16,
    I24,
    I32,
    I64,
    U8,
    U16,
    U24,
    U32,
    U64,
    F32,
    F64,
}

impl TryFrom<SampleFormat> for InputSampleFormat {
    type Error = StartListeningError;

    fn try_from(sample_format: SampleFormat) -> Result<Self, Self::Error> {
        match sample_format {
            SampleFormat::I8 => Ok(Self::I8),
            SampleFormat::I16 => Ok(Self::I16),
            SampleFormat::I24 => Ok(Self::I24),
            SampleFormat::I32 => Ok(Self::I32),
            SampleFormat::I64 => Ok(Self::I64),
            SampleFormat::U8 => Ok(Self::U8),
            SampleFormat::U16 => Ok(Self::U16),
            SampleFormat::U24 => Ok(Self::U24),
            SampleFormat::U32 => Ok(Self::U32),
            SampleFormat::U64 => Ok(Self::U64),
            SampleFormat::F32 => Ok(Self::F32),
            SampleFormat::F64 => Ok(Self::F64),
            sample_format => Err(StartListeningError::Other(anyhow::anyhow!(
                "Unsupported input sample format: {sample_format}"
            ))),
        }
    }
}

fn normalize_audio_frame<T>(data: &[T], num_channels: u16) -> Result<Vec<f32>, anyhow::Error>
where
    T: Sample,
    f32: FromSample<T>,
{
    if num_channels == 0 {
        return Err(anyhow::anyhow!("Input stream reported zero channels"));
    }

    Ok(data
        .chunks_exact(num_channels as usize)
        .map(|frame| {
            frame
                .iter()
                .map(|sample| sample.to_sample::<f32>())
                .sum::<f32>()
                / num_channels as f32
        })
        .collect())
}

fn send_audio_frame<T>(
    data: &[T],
    num_channels: u16,
    audio_frame_tx: &async_channel::Sender<Vec<f32>>,
) where
    T: Sample,
    f32: FromSample<T>,
{
    let mono_samples = match normalize_audio_frame(data, num_channels) {
        Ok(samples) => samples,
        Err(error) => {
            report_error!(error.context("Failed to normalize voice input frame"));
            return;
        }
    };

    let is_empty = mono_samples.iter().all(|&sample| sample == 0.0);
    log::debug!("Sending audio frame to resampling thread. is_empty: {is_empty}");

    // This is blocking, but we aren't on the main thread.
    let _ = warpui_core::r#async::block_on(audio_frame_tx.send(mono_samples));
}

fn build_input_stream<T>(
    input_device: &cpal::Device,
    stream_config: &StreamConfig,
    num_channels: u16,
    audio_frame_tx: async_channel::Sender<Vec<f32>>,
) -> Result<cpal::Stream, StartListeningError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    // Some audio backends (notably ALSA on Linux) fire this error callback repeatedly in a tight
    // loop when the input device wedges. Reporting only the first error prevents flooding Sentry
    // with millions of identical events.
    let mut has_logged_stream_error = false;
    input_device
        .build_input_stream(
            stream_config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                send_audio_frame(data, num_channels, &audio_frame_tx);
            },
            move |err| {
                if has_logged_stream_error {
                    log::debug!("Error in voice input stream (suppressed repeat): {err}");
                } else {
                    has_logged_stream_error = true;
                    report_error!(anyhow::Error::new(err).context("Error in voice input stream"));
                }
            },
            Some(STREAM_TIMEOUT),
        )
        .map_err(|e| {
            StartListeningError::Other(anyhow::anyhow!("Failed to build input stream: {e}"))
        })
}

impl VoiceInput {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            state: VoiceInputState::Idle,
            should_suppress_new_feature_popup: false,
            voice_session_start: None,
            wav_conversion_handle: None,
        }
    }

    pub fn is_listening(&self) -> bool {
        matches!(self.state, VoiceInputState::Listening { .. })
    }

    pub fn is_transcribing(&self) -> bool {
        matches!(self.state, VoiceInputState::Transcribing)
    }

    /// Returns true if voice is currently recording or transcribing.
    pub fn is_active(&self) -> bool {
        self.is_listening() || self.is_transcribing()
    }

    pub fn state(&self) -> &VoiceInputState {
        &self.state
    }

    /// Starts listening for voice input and returns a session that will receive the result.
    ///
    /// The returned `VoiceSession` can be awaited to receive the audio data when recording
    /// stops. Dropping the session will abort the recording.
    pub fn start_listening(
        &mut self,
        ctx: &mut ModelContext<Self>,
        source: VoiceInputToggledFrom,
    ) -> Result<VoiceSession, StartListeningError> {
        if self.is_active() {
            log::debug!("Already listening, not starting again");
            return Err(StartListeningError::AlreadyRunning);
        }

        log::debug!("Enabling voice input");
        let (audio_frame_tx, audio_frame_rx) = async_channel::unbounded();
        let _ = ctx.spawn_stream_local(audio_frame_rx.clone(), Self::on_audio_frame, |_, _| {
            log::debug!("Stream done");
        });

        let host = cpal::default_host();
        let Some(input_device) = host.default_input_device() else {
            return Err(anyhow::anyhow!("No default input device found").into());
        };

        let config = input_device
            .default_input_config()
            .context("Failed to get default input config")
            .map_err(|e| {
                report_error!(&e);
                StartListeningError::Other(e)
            })?;

        // Kind of annoying that we need to check this here, but cpal will actually still create an audio
        // stream of empty frames even if the user denies access on MacOS.
        if matches!(
            ctx.microphone_access_state(),
            MicrophoneAccessState::Denied | MicrophoneAccessState::Restricted
        ) {
            return Err(StartListeningError::AccessDenied);
        }

        // Try to use our default chunk size, but clamped to the supported range.
        let buffer_size = match config.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => DEFAULT_CHUNK_SIZE.clamp(*min, *max),
            cpal::SupportedBufferSize::Unknown => DEFAULT_CHUNK_SIZE,
        };
        let sample_format = InputSampleFormat::try_from(config.sample_format())?;
        let sample_rate = config.sample_rate() as f64;
        let num_channels = config.channels();
        if num_channels == 0 {
            return Err(StartListeningError::Other(anyhow::anyhow!(
                "Input stream reported zero channels"
            )));
        }
        let stream_config: StreamConfig = config.into();

        // Set the buffer size to a fixed size so it's easier to resample.
        let stream_config = StreamConfig {
            buffer_size: cpal::BufferSize::Fixed(buffer_size),
            ..stream_config
        };

        log::debug!("Stream config: {stream_config:?}");

        // Set up the resampler to resample the audio to 16000Hz, which is typical for voice input.
        let resampler = SincFixedIn::new(
            TARGET_SAMPLE_RATE as f64 / sample_rate,
            2.0,
            SincInterpolationParameters {
                interpolation: SincInterpolationType::Linear,
                window: WindowFunction::Hann,
                sinc_len: buffer_size as usize,
                f_cutoff: 0.95,
                oversampling_factor: 1,
            },
            buffer_size as usize,
            NUM_CHANNELS as usize,
        )
        .map_err(|e| {
            StartListeningError::Other(anyhow::anyhow!("Resampler construction failed: {e}"))
        })?;

        let stream = match sample_format {
            InputSampleFormat::I8 => build_input_stream::<i8>(
                &input_device,
                &stream_config,
                num_channels,
                audio_frame_tx,
            ),
            InputSampleFormat::I16 => build_input_stream::<i16>(
                &input_device,
                &stream_config,
                num_channels,
                audio_frame_tx,
            ),
            InputSampleFormat::I24 => build_input_stream::<I24>(
                &input_device,
                &stream_config,
                num_channels,
                audio_frame_tx,
            ),
            InputSampleFormat::I32 => build_input_stream::<i32>(
                &input_device,
                &stream_config,
                num_channels,
                audio_frame_tx,
            ),
            InputSampleFormat::I64 => build_input_stream::<i64>(
                &input_device,
                &stream_config,
                num_channels,
                audio_frame_tx,
            ),
            InputSampleFormat::U8 => build_input_stream::<u8>(
                &input_device,
                &stream_config,
                num_channels,
                audio_frame_tx,
            ),
            InputSampleFormat::U16 => build_input_stream::<u16>(
                &input_device,
                &stream_config,
                num_channels,
                audio_frame_tx,
            ),
            InputSampleFormat::U24 => build_input_stream::<U24>(
                &input_device,
                &stream_config,
                num_channels,
                audio_frame_tx,
            ),
            InputSampleFormat::U32 => build_input_stream::<u32>(
                &input_device,
                &stream_config,
                num_channels,
                audio_frame_tx,
            ),
            InputSampleFormat::U64 => build_input_stream::<u64>(
                &input_device,
                &stream_config,
                num_channels,
                audio_frame_tx,
            ),
            InputSampleFormat::F32 => build_input_stream::<f32>(
                &input_device,
                &stream_config,
                num_channels,
                audio_frame_tx,
            ),
            InputSampleFormat::F64 => build_input_stream::<f64>(
                &input_device,
                &stream_config,
                num_channels,
                audio_frame_tx,
            ),
        }?;
        cpal::traits::StreamTrait::play(&stream).map_err(|e| {
            StartListeningError::Other(anyhow::anyhow!("Failed to play stream: {e}"))
        })?;

        log::debug!("Starting voice input stream with chunk size {buffer_size}");

        // Track voice session start time
        self.voice_session_start = Some(instant::Instant::now());

        // Create channel for returning result to caller
        let (result_tx, result_rx) = oneshot::channel();

        self.state = VoiceInputState::Listening {
            resampler: Arc::new(Mutex::new(resampler)),
            resampled: Arc::new(Mutex::new(vec![])),
            chunk_size: buffer_size as usize,
            enabled_from: source,
            result_tx: Some(result_tx),
            // We need to keep the stream around to keep the audio flowing.
            stream,
        };

        Ok(VoiceSession { result_rx })
    }

    pub fn start_time(&self) -> Option<instant::Instant> {
        self.voice_session_start
    }

    pub fn set_transcribing_active(&mut self, active: bool) {
        if active {
            self.state = VoiceInputState::Transcribing;
        } else {
            if let Some(handle) = self.wav_conversion_handle.take() {
                handle.abort();
            }
            self.state = VoiceInputState::Idle;
        }
    }

    /// Stops listening and triggers WAV conversion. The result will be sent through
    /// the VoiceSession returned from start_listening.
    pub fn stop_listening(&mut self, ctx: &mut ModelContext<Self>) -> Result<(), anyhow::Error> {
        if let VoiceInputState::Listening {
            stream,
            resampled,
            result_tx,
            ..
        } = &mut self.state
        {
            cpal::traits::StreamTrait::pause(stream)?;

            // Calculate session duration before conversion
            let session_duration_ms = self
                .voice_session_start
                .take()
                .map(|start| start.elapsed().as_millis() as u64)
                .unwrap_or(0);

            log::debug!("Disabling voice input and converting to WAV");

            // Take the result_tx out to use in the spawn closure
            let result_tx = result_tx.take();

            // Spawn WAV conversion and send result through channel
            self.wav_conversion_handle = Some(ctx.spawn(
                Self::convert_to_wav(resampled.clone()),
                move |me, wav_result, _ctx| {
                    me.wav_conversion_handle = None;
                    if let Some(tx) = result_tx {
                        let result = match wav_result {
                            Ok(wav_base64) => VoiceSessionResult::Audio {
                                wav_base64,
                                session_duration_ms,
                            },
                            Err(e) => {
                                report_error!(e.context("Failed to convert to WAV"));
                                VoiceSessionResult::Aborted {
                                    session_duration_ms: Some(session_duration_ms),
                                }
                            }
                        };
                        let _ = tx.send(result);
                    }
                    // Move to Idle after sending result
                    me.state = VoiceInputState::Idle;
                },
            ));

            // Move to Transcribing state while conversion is happening
            self.state = VoiceInputState::Transcribing;
        } else {
            log::debug!("Not currently listening for voice input");
        }
        Ok(())
    }

    /// Stops listening without forwarding audio for processing.
    /// The VoiceSession will receive VoiceSessionResult::Aborted.
    pub fn abort_listening(&mut self) {
        log::debug!("Aborting voice input");

        // Calculate session duration before aborting
        let session_duration_ms = self
            .voice_session_start
            .take()
            .map(|start| start.elapsed().as_millis() as u64);

        // Take ownership and send abort result through channel
        let old_state = std::mem::take(&mut self.state);
        if let VoiceInputState::Listening {
            result_tx: Some(tx),
            ..
        } = old_state
        {
            let _ = tx.send(VoiceSessionResult::Aborted {
                session_duration_ms,
            });
        }

        // Reset to Idle state
        self.state = VoiceInputState::Idle;
    }

    // Enqueues a single audio frame to be processed on a background thread.
    fn on_audio_frame(&mut self, mut input_buffer: Vec<f32>, ctx: &mut ModelContext<Self>) {
        let VoiceInputState::Listening {
            resampler,
            resampled,
            chunk_size,
            ..
        } = &mut self.state
        else {
            return;
        };

        if input_buffer.len() < *chunk_size {
            input_buffer.resize(*chunk_size, 0.0); // Zero-pad if too short.
        }

        let resampler = resampler.clone();
        let resampled = resampled.clone();
        ctx.spawn(
            async move {
                if let Err(e) = Self::resample_audio_frame(resampler, resampled, input_buffer).await
                {
                    report_error!(e.context("Failed to resample audio frame"));
                }
            },
            |_, _, _| {},
        );
    }

    // Processes a single audio frame, resampling it to 16000Hz and adding it to the resampled buffer.
    async fn resample_audio_frame(
        resampler: Arc<Mutex<SincFixedIn<f32>>>,
        resampled: Arc<Mutex<Vec<f32>>>,
        input_buffer: Vec<f32>,
    ) -> Result<(), anyhow::Error> {
        let mut resampler = resampler.lock();
        let mut resampled = resampled.lock();
        resampled.extend(resampler.process(&[input_buffer], None)?[0].to_vec());
        Ok(())
    }

    // Converts the resampled audio to a WAV file and returns the base64 encoded WAV data.
    // Should be called on a background thread.
    async fn convert_to_wav(resampled: Arc<Mutex<Vec<f32>>>) -> Result<String, anyhow::Error> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let resampled = resampled.lock();
        let mut wav_cursor = Cursor::new(Vec::with_capacity(resampled.len() * 2));
        let mut wav_writer = hound::WavWriter::new(&mut wav_cursor, spec)?;

        for sample in resampled.as_slice() {
            let amplitude = sample.to_sample::<i16>();
            wav_writer.write_sample(amplitude)?;
        }

        wav_writer.finalize()?;

        let wav_bytes = wav_cursor.into_inner();
        let wav_base64 = base64::engine::general_purpose::STANDARD.encode(wav_bytes);
        Ok(wav_base64)
    }
}

impl Entity for VoiceInput {
    type Event = ();
}

impl SingletonEntity for VoiceInput {}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
