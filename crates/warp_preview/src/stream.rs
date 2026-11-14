use std::path::PathBuf;
use std::sync::mpsc;

use crate::input::InputEvent;
use crate::{Error, Rate, Source, StreamEvent};

/// What starting a stream needs beyond its source.
#[derive(Clone, Debug, Default)]
pub struct Environment {
    /// The Chromium-based browser to run pages in, from [`browser::find_chromium`].
    pub chromium: Option<PathBuf>,
    /// The size, in points, a page is laid out at.
    pub viewport: (u32, u32),
}

/// A message to a running stream's thread.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Control {
    SetRate(Rate),
    /// Loads a URL. Browser sources only.
    Navigate(String),
    /// Lays the page out at a new size, in points. Browser sources only.
    Resize(u32, u32),
    /// Rotates a simulator to its next orientation. Simulator sources only.
    Rotate,
    Input(InputEvent),
    Stop,
}

/// A running preview. Its source keeps producing [`StreamEvent`]s until the stream is dropped.
pub struct Stream {
    control: mpsc::Sender<Control>,
}

impl Stream {
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn new(control: mpsc::Sender<Control>) -> Self {
        Self { control }
    }

    pub fn set_rate(&self, rate: Rate) {
        let _ = self.control.send(Control::SetRate(rate));
    }

    /// Loads `url` in a browser preview. Does nothing for windows.
    pub fn navigate(&self, url: String) {
        let _ = self.control.send(Control::Navigate(url));
    }

    /// Lays a browser preview's page out at `width` by `height` points. Does nothing for windows.
    pub fn resize(&self, width: u32, height: u32) {
        let _ = self.control.send(Control::Resize(width, height));
    }

    /// Rotates a simulator to its next orientation. Does nothing for other sources.
    pub fn rotate(&self) {
        let _ = self.control.send(Control::Rotate);
    }

    /// Delivers pointer or keyboard input to the source.
    pub fn input(&self, event: InputEvent) {
        let _ = self.control.send(Control::Input(event));
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.control.send(Control::Stop);
    }
}

/// Starts streaming `source` at `rate`, reporting to `events`. Fails before any thread starts
/// when the source cannot be shown here.
pub fn start(
    source: &Source,
    environment: &Environment,
    rate: Rate,
    events: async_channel::Sender<StreamEvent>,
) -> Result<Stream, Error> {
    #[cfg(not(target_family = "wasm"))]
    match source {
        Source::Browser { url } => {
            let chromium = environment.chromium.clone().ok_or(Error::NoChromium)?;
            crate::browser::start_screencast(
                chromium,
                url.clone(),
                environment.viewport,
                rate,
                events,
            )
        }
        Source::Window(window_source) => {
            crate::window::start_capture(window_source.clone(), rate, events)
        }
        Source::Simulator(simulator) => crate::simulator::start(simulator.clone(), rate, events),
    }
    #[cfg(target_family = "wasm")]
    {
        let _ = (source, environment, rate, events);
        Err(Error::Unsupported)
    }
}
