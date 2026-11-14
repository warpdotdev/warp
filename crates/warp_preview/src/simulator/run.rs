//! Runs Baguette for one simulator: boots it, streams its screen as MJPEG, and feeds gestures to a
//! long-lived `baguette input`.

// Streams run on their own native threads, which block on channels and pipes by design.
#![allow(clippy::disallowed_methods)]

use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use command::blocking::Command;
use serde_json::Value;

use super::SimulatorEntry;
use super::wire::{MultipartReader, input_lines, parse_simctl_devices};
use crate::stream::Control;
use crate::{Error, Rate, SimulatorSource, Stream, StreamEvent, jpeg};

/// Frames a second and the integer downscale for the preview in front, and for cards behind it.
const FULL: (u32, u32) = (20, 2);
const THUMBNAIL: (u32, u32) = (2, 4);

pub(super) fn list_simulators() -> Result<Vec<SimulatorEntry>, Error> {
    let output = Command::new("xcrun")
        .args(["simctl", "list", "devices", "available", "-j"])
        .stdin(Stdio::null())
        .output()
        .map_err(|err| Error::Other(format!("Couldn't run simctl: {err}")))?;
    if !output.status.success() {
        return Err(Error::Other(
            "Couldn't list simulators. Is Xcode installed?".to_owned(),
        ));
    }
    Ok(parse_simctl_devices(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

pub(super) fn start(
    source: SimulatorSource,
    baguette: PathBuf,
    rate: Rate,
    events: async_channel::Sender<StreamEvent>,
) -> Result<Stream, Error> {
    let (control_tx, control_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("preview-simulator".to_owned())
        .spawn(move || {
            if let Err(reason) = run(&source, &baguette, rate, &control_rx, &events) {
                let _ = events.send_blocking(StreamEvent::Ended(reason));
            }
        })
        .map_err(|err| Error::Other(err.to_string()))?;
    Ok(Stream::new(control_tx))
}

fn run(
    source: &SimulatorSource,
    baguette: &Path,
    mut rate: Rate,
    control: &mpsc::Receiver<Control>,
    events: &async_channel::Sender<StreamEvent>,
) -> Result<(), String> {
    boot(baguette, &source.udid)?;
    let mut screen = screen_points(baguette, &source.udid)?;
    let mut orientation = Orientation::Portrait;
    let mut frames = FrameStream::start(baguette, &source.udid, rate, events)?;
    let mut input: Option<InputProcess> = None;
    loop {
        match control.recv() {
            Ok(Control::SetRate(new_rate)) => {
                if new_rate != rate {
                    rate = new_rate;
                    frames.stop();
                    frames = FrameStream::start(baguette, &source.udid, rate, events)?;
                }
            }
            Ok(Control::Input(event)) => {
                if input.is_none() {
                    input = Some(InputProcess::start(baguette, &source.udid)?);
                }
                if let Some(process) = &mut input
                    && let Err(err) = process.send(&input_lines(&event, screen))
                {
                    log::warn!("Simulator input failed: {err}");
                    input = None;
                }
            }
            Ok(Control::Navigate(_) | Control::Resize(..)) => {}
            Ok(Control::Rotate) => {
                orientation = orientation.next();
                set_orientation(baguette, &source.udid, orientation)?;
                screen = screen_points(baguette, &source.udid)?;
            }
            Ok(Control::Stop) | Err(_) => {
                frames.stop();
                return Ok(());
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Orientation {
    Portrait,
    LandscapeRight,
    PortraitUpsideDown,
    LandscapeLeft,
}

impl Orientation {
    fn next(self) -> Self {
        match self {
            Self::Portrait => Self::LandscapeRight,
            Self::LandscapeRight => Self::PortraitUpsideDown,
            Self::PortraitUpsideDown => Self::LandscapeLeft,
            Self::LandscapeLeft => Self::Portrait,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Portrait => "portrait",
            Self::LandscapeRight => "landscape-right",
            Self::PortraitUpsideDown => "pud",
            Self::LandscapeLeft => "landscape-left",
        }
    }
}

fn set_orientation(baguette: &Path, udid: &str, orientation: Orientation) -> Result<(), String> {
    let output = Command::new(baguette)
        .args(["orientation", "--udid", udid, orientation.as_str()])
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("Couldn't rotate simulator: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "Couldn't rotate simulator: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// Boots the device without the Simulator app's window. Booting a booted device is harmless.
fn boot(baguette: &Path, udid: &str) -> Result<(), String> {
    let output = Command::new(baguette)
        .args(["boot", "--udid", udid])
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("Couldn't run Baguette: {err}"))?;
    if !output.status.success() {
        log::info!(
            "baguette boot exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// The device's screen size in points, which Baguette's gestures are given in.
fn screen_points(baguette: &Path, udid: &str) -> Result<(f64, f64), String> {
    let output = Command::new(baguette)
        .args(["chrome", "layout", "--udid", udid])
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("Couldn't run Baguette: {err}"))?;
    let layout: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "The simulator didn't start. Is it installed in Xcode?".to_owned())?;
    match (
        layout["screen"]["width"].as_f64(),
        layout["screen"]["height"].as_f64(),
    ) {
        (Some(width), Some(height)) if width > 0. && height > 0. => Ok((width, height)),
        _ => Err("Baguette didn't report the simulator's screen size".to_owned()),
    }
}

/// A running `baguette stream`, whose frames a reader thread forwards as they arrive.
struct FrameStream {
    child: Option<Child>,
    stopping: Arc<AtomicBool>,
}

impl FrameStream {
    fn start(
        baguette: &Path,
        udid: &str,
        rate: Rate,
        events: &async_channel::Sender<StreamEvent>,
    ) -> Result<Self, String> {
        let (fps, scale) = match rate {
            Rate::Full => FULL,
            Rate::Thumbnail => THUMBNAIL,
            Rate::Paused => {
                return Ok(Self {
                    child: None,
                    stopping: Arc::new(AtomicBool::new(true)),
                });
            }
        };
        let mut child = Command::new(baguette)
            .args(["stream", "--udid", udid, "--format", "mjpeg"])
            .args(["--fps", &fps.to_string(), "--scale", &scale.to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("Couldn't start Baguette: {err}"))?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let stopping = Arc::new(AtomicBool::new(false));
        let reader_stopping = stopping.clone();
        let events = events.clone();
        std::thread::Builder::new()
            .name("preview-simulator-frames".to_owned())
            .spawn(move || {
                let mut parts = MultipartReader::new(BufReader::new(stdout));
                while let Ok(Some(part)) = parts.next_part() {
                    let Some(frame) = jpeg::frame_from_jpeg(part) else {
                        continue;
                    };
                    if events.send_blocking(StreamEvent::Frame(frame)).is_err() {
                        return;
                    }
                }
                if !reader_stopping.load(Ordering::SeqCst) {
                    let _ = events.send_blocking(StreamEvent::Ended(
                        "The simulator stopped streaming".to_owned(),
                    ));
                }
            })
            .map_err(|err| err.to_string())?;
        Ok(Self {
            child: Some(child),
            stopping,
        })
    }

    fn stop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for FrameStream {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A long-lived `baguette input`, which reads one JSON gesture per line.
struct InputProcess {
    child: Child,
    stdin: ChildStdin,
}

impl InputProcess {
    fn start(baguette: &Path, udid: &str) -> Result<Self, String> {
        let mut child = Command::new(baguette)
            .args(["input", "--udid", udid])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("Couldn't start Baguette input: {err}"))?;
        let stdin = child.stdin.take().expect("stdin is piped");
        Ok(Self { child, stdin })
    }

    fn send(&mut self, lines: &[String]) -> std::io::Result<()> {
        for line in lines {
            writeln!(self.stdin, "{line}")?;
        }
        self.stdin.flush()
    }
}

impl Drop for InputProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
