use std::collections::VecDeque;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use warp_preview::input::InputEvent;
use warp_preview::{Environment, Rate, Source, Stream, StreamEvent};
use warpui::assets::asset_cache::{AssetCache, AssetSource};
use warpui::image_cache::ImageType;
use warpui::{Entity, ModelContext, SingletonEntity};

/// The size, in points, pages in previews are laid out at.
const PAGE_VIEWPORT: (u32, u32) = (1280, 800);

/// How many log lines a preview keeps.
const MAX_LOG_LINES: usize = 500;

/// Where Warp keeps the Chromium it downloads for previews.
fn chromium_install_dir() -> PathBuf {
    warp_core::paths::data_dir().join("preview-chromium")
}

/// Identifies a preview for its whole life, wherever it is shown. Agents address previews by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PreviewId(pub u64);

impl fmt::Display for PreviewId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewStatus {
    Starting,
    Live,
    /// The window is minimized or on another Space, so it sends no frames.
    Minimized,
    /// A page preview is waiting for Chromium to be found or downloaded.
    NeedsChromium,
    /// Window previews are waiting for the Screen Recording permission.
    NeedsPermission,
    Ended(String),
}

/// Whether previews of pages can run, and if not, why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChromiumState {
    Looking,
    Found(PathBuf),
    Missing,
    Downloading,
    Failed(String),
}

/// A preview and what it last showed.
pub struct Preview {
    pub id: PreviewId,
    pub source: Source,
    pub title: Option<String>,
    pub status: PreviewStatus,
    /// Size of the latest frame, in pixels.
    pub frame_size: Option<(u32, u32)>,
    /// Whether the user's pointer and keys over the preview go to its source.
    pub playing: bool,
    latest_jpeg: Option<Arc<[u8]>>,
    log: VecDeque<String>,
    rate: Rate,
    stream: Option<Stream>,
    /// Kept to read frames that queued up behind the one being handled, so only the newest is
    /// decoded.
    events: Option<async_channel::Receiver<StreamEvent>>,
}

impl Preview {
    /// A short name for the card: the page title, or the source's own label.
    pub fn label(&self) -> String {
        match (&self.title, &self.source) {
            (Some(title), Source::Browser { .. }) if !title.is_empty() => title.clone(),
            (_, source) => source.label(),
        }
    }

    pub fn latest_jpeg(&self) -> Option<&Arc<[u8]>> {
        self.latest_jpeg.as_ref()
    }

    /// The newest `count` log lines, oldest first.
    pub fn recent_log(&self, count: usize) -> impl Iterator<Item = &String> {
        self.log.iter().skip(self.log.len().saturating_sub(count))
    }

    pub fn frame_asset(&self) -> AssetSource {
        frame_asset(self.id)
    }
}

/// The image the latest frame of a preview is stored under.
pub fn frame_asset(id: PreviewId) -> AssetSource {
    AssetSource::Raw {
        id: frame_asset_id(id),
    }
}

fn frame_asset_id(id: PreviewId) -> String {
    format!("preview-frame-{id}")
}

/// Every preview running in this Warp instance, wherever it is shown. Views show previews by id
/// and tell this model how often each should update.
pub struct PreviewStreams {
    previews: Vec<Preview>,
    next_id: u64,
    chromium: ChromiumState,
    /// The preview an agent opened or looked at last, which tools use when given none.
    last_used: Option<PreviewId>,
}

impl PreviewStreams {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let streams = Self {
            previews: Vec::new(),
            next_id: 1,
            chromium: ChromiumState::Looking,
            last_used: None,
        };
        if !super::is_enabled() {
            return streams;
        }
        ctx.spawn(
            async {
                warp_preview::browser::clean_up_after_crashed_runs();
                warp_preview::browser::find_chromium(Some(&chromium_install_dir()))
            },
            |me, found, ctx| {
                me.chromium = match found {
                    Some(path) => ChromiumState::Found(path),
                    None => ChromiumState::Missing,
                };
                me.restart_waiting(PreviewStatus::NeedsChromium, ctx);
                ctx.notify();
            },
        );
        streams
    }

    pub fn chromium(&self) -> &ChromiumState {
        &self.chromium
    }

    /// Downloads Chromium for page previews, then starts the previews waiting for it.
    pub fn download_chromium(&mut self, ctx: &mut ModelContext<Self>) {
        if matches!(
            self.chromium,
            ChromiumState::Downloading | ChromiumState::Found(_)
        ) {
            return;
        }
        self.chromium = ChromiumState::Downloading;
        ctx.notify();
        ctx.spawn(
            async { warp_preview::browser::install_chromium(&chromium_install_dir()) },
            |me, result, ctx| {
                me.chromium = match result {
                    Ok(path) => ChromiumState::Found(path),
                    Err(err) => {
                        log::warn!("Failed to download Chromium for previews: {err:#}");
                        ChromiumState::Failed(err.to_string())
                    }
                };
                me.restart_waiting(PreviewStatus::NeedsChromium, ctx);
                ctx.notify();
            },
        );
    }

    /// Asks for the Screen Recording permission, then retries window previews waiting for it.
    pub fn request_screen_recording(&mut self, ctx: &mut ModelContext<Self>) {
        warp_preview::window::request_permission();
        self.restart_waiting(PreviewStatus::NeedsPermission, ctx);
    }

    pub fn get(&self, id: PreviewId) -> Option<&Preview> {
        self.previews.iter().find(|preview| preview.id == id)
    }

    fn get_mut(&mut self, id: PreviewId) -> Option<&mut Preview> {
        self.previews.iter_mut().find(|preview| preview.id == id)
    }

    pub fn previews(&self) -> impl Iterator<Item = &Preview> {
        self.previews.iter()
    }

    pub fn last_used(&self) -> Option<PreviewId> {
        self.last_used
            .filter(|id| self.get(*id).is_some())
            .or_else(|| self.previews.last().map(|preview| preview.id))
    }

    pub fn mark_used(&mut self, id: PreviewId) {
        self.last_used = Some(id);
    }

    /// Starts a preview of `source`, which shows as starting until its first frame.
    pub fn open(&mut self, source: Source, ctx: &mut ModelContext<Self>) -> PreviewId {
        let id = PreviewId(self.next_id);
        self.next_id += 1;
        self.previews.push(Preview {
            id,
            source,
            title: None,
            status: PreviewStatus::Starting,
            frame_size: None,
            playing: false,
            latest_jpeg: None,
            log: VecDeque::new(),
            rate: Rate::Paused,
            stream: None,
            events: None,
        });
        self.start(id, ctx);
        ctx.notify();
        id
    }

    /// Starts a preview that ended or was waiting, again.
    pub fn retry(&mut self, id: PreviewId, ctx: &mut ModelContext<Self>) {
        self.start(id, ctx);
        ctx.notify();
    }

    /// Stops a preview for good.
    pub fn close(&mut self, id: PreviewId, ctx: &mut ModelContext<Self>) {
        self.previews.retain(|preview| preview.id != id);
        ctx.notify();
    }

    pub fn set_playing(&mut self, id: PreviewId, playing: bool, ctx: &mut ModelContext<Self>) {
        if let Some(preview) = self.get_mut(id) {
            preview.playing = playing;
            ctx.notify();
        }
    }

    /// Delivers input to a preview's source. False when the preview is not running.
    pub fn send_input(&self, id: PreviewId, event: InputEvent) -> bool {
        match self.get(id).and_then(|preview| preview.stream.as_ref()) {
            Some(stream) => {
                stream.input(event);
                true
            }
            None => false,
        }
    }

    /// Rotates a simulator preview to its next orientation.
    pub fn rotate(&self, id: PreviewId) -> bool {
        let Some(preview) = self.get(id) else {
            return false;
        };
        if !matches!(preview.source, Source::Simulator(_)) {
            return false;
        }
        let Some(stream) = &preview.stream else {
            return false;
        };
        stream.rotate();
        true
    }

    /// Sets how often a preview updates, from how prominently it is shown.
    pub fn set_rate(&mut self, id: PreviewId, rate: Rate) {
        let Some(preview) = self.get_mut(id) else {
            return;
        };
        if preview.rate == rate {
            return;
        }
        preview.rate = rate;
        if let Some(stream) = &preview.stream {
            stream.set_rate(rate);
        }
    }

    fn restart_waiting(&mut self, waiting: PreviewStatus, ctx: &mut ModelContext<Self>) {
        let ids: Vec<PreviewId> = self
            .previews
            .iter()
            .filter(|preview| preview.status == waiting)
            .map(|preview| preview.id)
            .collect();
        for id in ids {
            self.start(id, ctx);
        }
    }

    fn start(&mut self, id: PreviewId, ctx: &mut ModelContext<Self>) {
        let chromium = match &self.chromium {
            ChromiumState::Found(path) => Some(path.clone()),
            ChromiumState::Looking
            | ChromiumState::Missing
            | ChromiumState::Downloading
            | ChromiumState::Failed(_) => None,
        };
        let Some(preview) = self.get_mut(id) else {
            return;
        };
        preview.stream = None;
        preview.events = None;
        let environment = Environment {
            chromium,
            viewport: PAGE_VIEWPORT,
        };
        let (events_tx, events_rx) = async_channel::unbounded();
        // A preview nobody shows yet still gets frames, so agents can look at it right away.
        let rate = match preview.rate {
            Rate::Paused => Rate::Thumbnail,
            rate => rate,
        };
        match warp_preview::start(&preview.source, &environment, rate, events_tx) {
            Ok(stream) => {
                preview.status = PreviewStatus::Starting;
                preview.stream = Some(stream);
                preview.events = Some(events_rx.clone());
                ctx.spawn_stream_local(
                    events_rx,
                    move |me, event, ctx| me.handle_event(id, event, ctx),
                    |_, _| {},
                );
            }
            Err(warp_preview::Error::NoChromium) => {
                preview.status = PreviewStatus::NeedsChromium;
            }
            Err(warp_preview::Error::ScreenRecordingDenied) => {
                preview.status = PreviewStatus::NeedsPermission;
            }
            Err(err) => preview.status = PreviewStatus::Ended(err.to_string()),
        }
    }

    fn handle_event(&mut self, id: PreviewId, event: StreamEvent, ctx: &mut ModelContext<Self>) {
        let Some(preview) = self.get_mut(id) else {
            return;
        };
        let mut frame = None;
        let mut pending = vec![event];
        // Frames that queued up while the main thread was busy are skipped, so only the newest
        // is decoded.
        if let Some(events) = &preview.events {
            while let Ok(event) = events.try_recv() {
                pending.push(event);
            }
        }
        for event in pending {
            match event {
                StreamEvent::Frame(new_frame) => frame = Some(new_frame),
                StreamEvent::Title(title) => preview.title = Some(title),
                StreamEvent::Url(url) => {
                    if let Source::Browser { url: current } = &mut preview.source {
                        *current = url;
                    }
                }
                StreamEvent::Log(line) => {
                    if preview.log.len() == MAX_LOG_LINES {
                        preview.log.pop_front();
                    }
                    preview.log.push_back(line);
                }
                StreamEvent::Minimized(minimized) => {
                    preview.status = if minimized {
                        PreviewStatus::Minimized
                    } else {
                        PreviewStatus::Live
                    };
                }
                StreamEvent::Ended(reason) => {
                    preview.status = PreviewStatus::Ended(reason);
                    preview.stream = None;
                    preview.events = None;
                }
            }
        }
        if let Some(frame) = frame {
            if preview.status == PreviewStatus::Starting {
                preview.status = PreviewStatus::Live;
            }
            preview.frame_size = Some((frame.width, frame.height));
            preview.latest_jpeg = Some(frame.jpeg.clone());
            AssetCache::handle(ctx).update(ctx, |cache, ctx| {
                cache.insert_raw_asset_bytes::<ImageType>(frame_asset_id(id), &frame.jpeg, ctx);
            });
        }
        ctx.notify();
    }
}

impl Entity for PreviewStreams {
    type Event = ();
}

impl SingletonEntity for PreviewStreams {}
