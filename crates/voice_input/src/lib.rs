use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{Sample, StreamConfig};
use futures::channel::oneshot;
use parking_lot::Mutex;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use thiserror::Error;
use warpui::event::KeyState;
use warpui::platform::MicrophoneAccessState;
use warpui::{Entity, ModelContext, SingletonEntity};

const DEFAULT_CHUNK_SIZE: u32 = 512;
// We only support mono for now.
const NUM_CHANNELS: u16 = 1;
// Voice input is typically sampled at 16000Hz (and required by Wispr)
const TARGET_SAMPLE_RATE: f32 = 16000.0;
const STREAM_TIMEOUT: Duration = Duration::from_secs(60 * 6);

pub struct VoiceInput {
    state: VoiceInputState,
    pub should_suppress_new_feature_popup: bool,
    voice_session_start: Option<instant::Instant>,
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

impl VoiceInput {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            state: VoiceInputState::Idle,
            should_suppress_new_feature_popup: false,
            voice_session_start: None,
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
        if self.is_listening() {
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

        let config = input_device.default_input_config().map_err(|e| {
            log::error!("Failed to get default input config: {e}");
            StartListeningError::Other(anyhow::anyhow!("Failed to get default input config: {}", e))
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
        let sample_rate = config.sample_rate() as f64;
        let num_channels = config.channels();
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

        let stream = input_device
            .build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let is_empty = data.iter().all(|&x| x == 0.0);
                    log::debug!("Sending audio frame to resampling thread. is_empty: {is_empty}");

                    // Average the channels into mono at this point.
                    let mono_samples: Vec<f32> = data
                        .chunks_exact(num_channels as usize)
                        .map(|frame| frame.iter().sum::<f32>() / num_channels as f32)
                        .collect();

                    // This is blocking, but we aren't on the main thread.
                    let _ = warpui::r#async::block_on(audio_frame_tx.send(mono_samples));
                },
                |err| {
                    log::error!("Error in voice input stream: {err}");
                },
                Some(STREAM_TIMEOUT),
            )
            .map_err(|e| {
                StartListeningError::Other(anyhow::anyhow!("Failed to build input stream: {e}"))
            })?;
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
            let _ = ctx.spawn(
                Self::convert_to_wav(resampled.clone()),
                move |me, wav_result, _ctx| {
                    if let Some(tx) = result_tx {
                        let result = match wav_result {
                            Ok(wav_base64) => VoiceSessionResult::Audio {
                                wav_base64,
                                session_duration_ms,
                            },
                            Err(e) => {
                                log::error!("Failed to convert to WAV: {e}");
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
            );

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
                    log::error!("Failed to resample audio frame: {e}");
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
