use std::path::PathBuf;
use std::time::Duration;

use warp_browser::local_servers::LocalServer;
use warp_core::ui::appearance::Appearance;
use warp_preview::simulator::SimulatorEntry;
use warp_preview::window::WindowEntry;
use warp_preview::{Rate, SimulatorSource, Source, WindowSource};
use warpui::r#async::Timer;
use warpui::clipboard::{ClipboardContent, ImageData};
use warpui::elements::{ChildView, Container, Flex, MouseStateHandle, ParentElement};
use warpui::{
    AppContext, BlurContext, Element, Entity, FocusContext, ModelHandle, SingletonEntity,
    TypedActionView, View, ViewContext, ViewHandle,
};

use super::PreviewRegistry;
use super::playing::{self, PlayingInput};
use super::stage::{StageAction, StageMouseStates, StageOptions, render_stage};
use super::streams::{ChromiumState, PreviewId, PreviewStreams};
use crate::browser::AgentApproval;
use crate::editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions, TextOptions};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view::{self, HeaderContent};
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};
use crate::ui_components::icons::Icon;
use crate::ui_components::start_page;
use crate::workspace::WorkspaceAction;

const DEFAULT_TITLE: &str = "Preview";
const URL_FIELD_PLACEHOLDER: &str = "Enter a URL to preview, such as localhost:3000";
const MAX_LOCAL_SERVERS: usize = 6;
const MAX_WINDOWS: usize = 12;
const MAX_SIMULATORS: usize = 6;
const NOTICE_DURATION: Duration = Duration::from_secs(2);
/// How long an agent's step shows on the badge after its last call.
const AGENT_STEP_LINGER: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewViewEvent {
    Pane(PaneEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewViewAction {
    Stage(StageAction),
    OpenUrl(String),
    OpenWindow(WindowSource),
    OpenSimulator(SimulatorSource),
    /// Leaves the start page for the stage, when there are previews to show.
    BackToStage,
    RefreshWindows,
    OpenScreenRecordingSettings,
    FocusUrlField,
    SetUpAgentTools,
}

/// Actions for the pane header's overflow menu, which has no items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewHeaderAction {}

/// What the start page knows about other apps' windows.
enum WindowList {
    Unsupported,
    Loading,
    NeedsPermission,
    Listed(Vec<WindowEntry>),
    Failed(String),
}

/// What the start page knows about iOS simulators.
enum SimulatorList {
    Unsupported,
    Loading,
    /// Baguette, which runs simulators without their window, is not installed.
    NeedsBaguette,
    Listed(Vec<SimulatorEntry>),
    Failed(String),
}

/// An app an agent asked to watch, waiting for the user's answer.
pub struct AppApproval {
    pub key: String,
    pub name: String,
}

/// A preview pane: a stack of live previews with one in front, or a start page for choosing what
/// to preview.
pub struct PreviewView {
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    position_id: String,
    cards: Vec<PreviewId>,
    front: Option<PreviewId>,
    show_start_page: bool,
    /// Whether the pane is in a pane group. A closed pane stays alive, detached, while its close
    /// can still be undone.
    attached: bool,
    url_editor: ViewHandle<EditorView>,
    stage_mouse_states: StageMouseStates,
    local_servers: Vec<LocalServer>,
    windows: WindowList,
    simulators: SimulatorList,
    row_buttons: Vec<MouseStateHandle>,
    download_button: MouseStateHandle,
    permission_button: MouseStateHandle,
    settings_button: MouseStateHandle,
    agent_setup_button: MouseStateHandle,
    back_button: MouseStateHandle,
    notice: Option<&'static str>,
    agent_step: Option<String>,
    agent_step_generation: u64,
    approval: Option<AppApproval>,
    focused: bool,
    playing_input: PlayingInput,
}

impl PreviewView {
    /// Creates a pane showing `previews`, or the start page when there are none.
    pub fn new(previews: Vec<PreviewId>, ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(DEFAULT_TITLE));

        let appearance = Appearance::as_ref(ctx);
        let url_text = TextOptions::ui_text(Some(appearance.ui_font_size()), appearance);
        let url_editor = ctx.add_typed_action_view(|ctx| {
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: url_text,
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text(URL_FIELD_PLACEHOLDER, ctx);
            editor
        });
        ctx.subscribe_to_view(&url_editor, |me, _, event, ctx| {
            me.handle_url_editor_event(event, ctx);
        });
        ctx.observe(&PreviewStreams::handle(ctx), |me, _, ctx| {
            me.forget_closed_previews(ctx);
            me.sync_title(ctx);
            ctx.notify();
        });

        let handle = ctx.handle();
        PreviewRegistry::handle(ctx).update(ctx, |registry, _| registry.register_view(handle));

        let mut view = Self {
            pane_configuration,
            focus_handle: None,
            position_id: format!("preview_view_stage_{}", ctx.view_id()),
            front: previews.last().copied(),
            show_start_page: previews.is_empty(),
            cards: previews,
            attached: true,
            url_editor,
            stage_mouse_states: StageMouseStates::default(),
            local_servers: Vec::new(),
            windows: WindowList::Loading,
            simulators: SimulatorList::Loading,
            row_buttons: (0..MAX_LOCAL_SERVERS + MAX_WINDOWS + MAX_SIMULATORS)
                .map(|_| MouseStateHandle::default())
                .collect(),
            download_button: MouseStateHandle::default(),
            permission_button: MouseStateHandle::default(),
            settings_button: MouseStateHandle::default(),
            agent_setup_button: MouseStateHandle::default(),
            back_button: MouseStateHandle::default(),
            notice: None,
            agent_step: None,
            agent_step_generation: 0,
            approval: None,
            focused: false,
            playing_input: PlayingInput::default(),
        };
        view.stage_mouse_states.ensure_cards(view.cards.len());
        view.refresh_start_page(ctx);
        view.sync_title(ctx);
        view
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        if self.show_start_page {
            ctx.focus(&self.url_editor);
        } else {
            ctx.focus_self();
        }
    }

    pub fn is_attached(&self) -> bool {
        self.attached
    }

    /// Records whether the pane is in a pane group. Its previews stop when it is closed for good,
    /// and agent calls waiting for approval here are rejected once nobody can see the prompt.
    pub fn set_attached(&mut self, attached: bool, ctx: &mut ViewContext<Self>) {
        self.attached = attached;
        if attached {
            return;
        }
        if self.approval.take().is_some() {
            let view_id = ctx.view_id();
            super::PreviewAgent::handle(ctx)
                .update(ctx, |agent, ctx| agent.cancel_for_view(view_id, ctx));
        }
    }

    /// Stops every preview in the pane.
    pub fn close_previews(&mut self, ctx: &mut ViewContext<Self>) {
        let cards = std::mem::take(&mut self.cards);
        self.front = None;
        PreviewStreams::handle(ctx).update(ctx, |streams, ctx| {
            for id in cards {
                streams.close(id, ctx);
            }
        });
    }

    pub fn previews(&self) -> &[PreviewId] {
        &self.cards
    }

    pub fn front(&self) -> Option<PreviewId> {
        self.front
    }

    /// Adds a preview to the stack and brings it to the front.
    pub fn add_preview(&mut self, id: PreviewId, ctx: &mut ViewContext<Self>) {
        if !self.cards.contains(&id) {
            self.cards.push(id);
            self.stage_mouse_states.ensure_cards(self.cards.len());
        }
        self.front = Some(id);
        self.show_start_page = false;
        self.sync_title(ctx);
        ctx.notify();
    }

    /// Shows that an agent is taking `step` with the front preview.
    pub fn record_agent_step(&mut self, step: String, ctx: &mut ViewContext<Self>) {
        self.agent_step = Some(step);
        self.agent_step_generation += 1;
        let generation = self.agent_step_generation;
        ctx.spawn(Timer::after(AGENT_STEP_LINGER), move |me, _, ctx| {
            if me.agent_step_generation == generation {
                me.agent_step = None;
                ctx.notify();
            }
        });
        ctx.notify();
    }

    pub fn request_approval(&mut self, approval: AppApproval, ctx: &mut ViewContext<Self>) {
        self.approval = Some(approval);
        self.show_start_page = self.cards.is_empty();
        ctx.notify();
    }

    pub fn clear_approval(&mut self, ctx: &mut ViewContext<Self>) {
        self.approval = None;
        ctx.notify();
    }

    /// Sets how often each preview updates from whether the stage was drawn in the last frame.
    pub(super) fn sync_rates(&self, ctx: &mut ViewContext<Self>) {
        let visible = !self.show_start_page
            && ctx
                .element_position_by_id_at_last_frame(ctx.window_id(), &self.position_id)
                .is_some();
        let front = self.front;
        let cards = self.cards.clone();
        PreviewStreams::handle(ctx).update(ctx, |streams, _| {
            for id in cards {
                let rate = match (visible, Some(id) == front) {
                    (false, _) => Rate::Paused,
                    (true, true) => Rate::Full,
                    (true, false) => Rate::Thumbnail,
                };
                streams.set_rate(id, rate);
            }
        });
    }

    fn open_source(&mut self, source: Source, ctx: &mut ViewContext<Self>) {
        let id = PreviewStreams::handle(ctx).update(ctx, |streams, ctx| streams.open(source, ctx));
        self.add_preview(id, ctx);
        ctx.focus_self();
    }

    fn forget_closed_previews(&mut self, ctx: &mut ViewContext<Self>) {
        let streams = PreviewStreams::as_ref(ctx);
        self.cards.retain(|id| streams.get(*id).is_some());
        if self.front.is_some_and(|id| !self.cards.contains(&id)) {
            self.front = self.cards.last().copied();
        }
        if self.cards.is_empty() {
            self.show_start_page = true;
        }
    }

    fn sync_title(&mut self, ctx: &mut ViewContext<Self>) {
        let title = match self
            .front
            .and_then(|id| PreviewStreams::as_ref(ctx).get(id))
        {
            Some(preview) if !self.show_start_page => preview.label(),
            _ => DEFAULT_TITLE.to_owned(),
        };
        self.pane_configuration.update(ctx, |configuration, ctx| {
            configuration.set_title(title, ctx)
        });
    }

    /// Looks again for local servers and windows to offer on the start page.
    fn refresh_start_page(&mut self, ctx: &mut ViewContext<Self>) {
        #[cfg(not(target_family = "wasm"))]
        ctx.spawn(
            crate::browser::detect_local_servers(),
            |me, servers, ctx| {
                me.local_servers = servers;
                ctx.notify();
            },
        );
        self.refresh_windows(ctx);
        self.refresh_simulators(ctx);
    }

    fn refresh_simulators(&mut self, ctx: &mut ViewContext<Self>) {
        if !warp_preview::simulator::is_supported() {
            self.simulators = SimulatorList::Unsupported;
            return;
        }
        if warp_preview::simulator::find_baguette().is_none() {
            self.simulators = SimulatorList::NeedsBaguette;
            return;
        }
        ctx.spawn(
            async { warp_preview::simulator::list_simulators() },
            |me, simulators, ctx| {
                me.simulators = match simulators {
                    Ok(simulators) => SimulatorList::Listed(simulators),
                    Err(err) => SimulatorList::Failed(err.to_string()),
                };
                ctx.notify();
            },
        );
    }

    fn refresh_windows(&mut self, ctx: &mut ViewContext<Self>) {
        if !warp_preview::window::is_supported() {
            self.windows = WindowList::Unsupported;
            return;
        }
        if !warp_preview::window::has_permission() {
            self.windows = WindowList::NeedsPermission;
            ctx.notify();
            return;
        }
        ctx.spawn(
            async { warp_preview::window::list_windows() },
            |me, windows, ctx| {
                me.windows = match windows {
                    Ok(windows) => WindowList::Listed(windows),
                    Err(warp_preview::Error::ScreenRecordingDenied) => WindowList::NeedsPermission,
                    Err(err) => WindowList::Failed(err.to_string()),
                };
                ctx.notify();
            },
        );
    }

    fn show_notice(&mut self, message: &'static str, ctx: &mut ViewContext<Self>) {
        self.notice = Some(message);
        ctx.notify();
        ctx.spawn(Timer::after(NOTICE_DURATION), move |me, _, ctx| {
            if me.notice == Some(message) {
                me.notice = None;
                ctx.notify();
            }
        });
    }

    /// Saves the front preview's picture to the Desktop and copies it.
    fn take_screenshot(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(jpeg) = self
            .front
            .and_then(|id| PreviewStreams::as_ref(ctx).get(id))
            .and_then(|preview| preview.latest_jpeg().cloned())
        else {
            self.show_notice("Nothing to capture yet", ctx);
            return;
        };
        match save_screenshot(&jpeg) {
            Ok(_) => {
                ctx.clipboard().write(jpeg_clipboard_content(jpeg.to_vec()));
                self.show_notice("Screenshot saved and copied", ctx);
            }
            Err(err) => {
                log::warn!("Failed to save a preview screenshot: {err:#}");
                self.show_notice("Couldn't save the screenshot", ctx);
            }
        }
    }

    /// Moves this pane's previews to the window's picture-in-picture, and closes the pane.
    fn pop_out(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(front) = self.front else {
            return;
        };
        let cards = std::mem::take(&mut self.cards);
        self.front = None;
        super::open_picture_in_picture(cards, front, ctx);
        ctx.emit(PreviewViewEvent::Pane(PaneEvent::Close));
    }

    fn handle_stage_action(&mut self, action: &StageAction, ctx: &mut ViewContext<Self>) {
        match action {
            StageAction::BringToFront(id) => {
                self.front = Some(*id);
                self.playing_input = PlayingInput::default();
                self.sync_title(ctx);
                ctx.notify();
            }
            StageAction::Close(id) => {
                PreviewStreams::handle(ctx).update(ctx, |streams, ctx| streams.close(*id, ctx));
            }
            StageAction::Retry(id) => {
                PreviewStreams::handle(ctx).update(ctx, |streams, ctx| streams.retry(*id, ctx));
            }
            StageAction::DownloadChromium => {
                PreviewStreams::handle(ctx)
                    .update(ctx, |streams, ctx| streams.download_chromium(ctx));
            }
            StageAction::GrantScreenRecording => {
                PreviewStreams::handle(ctx)
                    .update(ctx, |streams, ctx| streams.request_screen_recording(ctx));
                self.refresh_windows(ctx);
            }
            StageAction::Screenshot => self.take_screenshot(ctx),
            StageAction::Rotate => {
                if let Some(front) = self.front {
                    PreviewStreams::handle(ctx).update(ctx, |streams, _| {
                        streams.rotate(front);
                    });
                }
            }
            StageAction::TogglePictureInPicture => self.pop_out(ctx),
            StageAction::AddPreview => {
                self.show_start_page = true;
                self.refresh_start_page(ctx);
                self.sync_title(ctx);
                ctx.focus(&self.url_editor);
                ctx.notify();
            }
            StageAction::ResolveApproval(decision) => self.resolve_approval(*decision, ctx),
            StageAction::SetPlaying(playing) => {
                let Some(front) = self.front else {
                    return;
                };
                self.playing_input = PlayingInput::default();
                if let Err(notice) = playing::set_playing(front, *playing, ctx) {
                    self.show_notice(notice, ctx);
                }
            }
            StageAction::Input(input) => {
                if let Some(front) = self.front {
                    self.playing_input
                        .forward(front, input, &self.position_id, ctx);
                }
            }
        }
    }

    fn resolve_approval(&mut self, decision: AgentApproval, ctx: &mut ViewContext<Self>) {
        let Some(approval) = self.approval.take() else {
            return;
        };
        ctx.notify();
        super::PreviewAgent::handle(ctx).update(ctx, |agent, ctx| {
            agent.resolve_approval(&approval.key, decision, ctx)
        });
    }

    fn handle_url_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Enter => {
                let input = self.url_editor.as_ref(ctx).buffer_text(ctx);
                if input.trim().is_empty() {
                    return;
                }
                self.url_editor
                    .update(ctx, |editor, ctx| editor.set_buffer_text("", ctx));
                self.open_source(
                    Source::Browser {
                        url: warp_browser::resolve_input(&input),
                    },
                    ctx,
                );
            }
            EditorEvent::Escape if !self.cards.is_empty() => {
                self.show_start_page = false;
                self.sync_title(ctx);
                ctx.focus_self();
                ctx.notify();
            }
            _ => {}
        }
    }

    fn render_start_page(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut rows = self.row_buttons.iter();
        let mut children = Vec::new();

        if !self.cards.is_empty() {
            children.push(
                Container::new(start_page::link(
                    "← Back to previews",
                    &self.back_button,
                    PreviewViewAction::BackToStage,
                    appearance,
                ))
                .with_margin_bottom(14.)
                .finish(),
            );
        }
        children.push(start_page::heading(
            "Preview",
            "Watch a page, an app or a game live beside your terminal, then play it with your \
             pointer and keys, or let an agent drive it.",
            appearance,
        ));
        children.push(start_page::input_frame(
            Icon::Globe,
            ChildView::new(&self.url_editor).finish(),
            appearance,
        ));
        children.push(
            Container::new(self.render_chromium_status(appearance, app))
                .with_margin_top(6.)
                .finish(),
        );

        if !self.local_servers.is_empty() {
            let server_rows = self
                .local_servers
                .iter()
                .take(MAX_LOCAL_SERVERS)
                .zip(rows.by_ref())
                .map(|(server, mouse_state)| {
                    start_page::row(
                        start_page::RowContent {
                            icon: Icon::Globe,
                            title: &warp_browser::display_url(&server.url()),
                            detail: Some(&server.process),
                            hover_hint: "Preview",
                        },
                        mouse_state,
                        PreviewViewAction::OpenUrl(server.url()),
                        appearance,
                    )
                })
                .collect();
            children.push(start_page::section(
                Icon::Terminal,
                "Running on this machine",
                server_rows,
                None,
                appearance,
            ));
        }

        let (window_rows, window_note) = match &self.windows {
            WindowList::Unsupported => (
                Vec::new(),
                Some(start_page::note(
                    "Previewing other apps' windows is available on macOS.",
                    appearance,
                )),
            ),
            WindowList::Loading => (
                Vec::new(),
                Some(start_page::note("Looking for windows…", appearance)),
            ),
            WindowList::NeedsPermission => (
                Vec::new(),
                Some(
                    Flex::column()
                        .with_child(start_page::note(
                            "Warp needs the Screen Recording permission to show other apps' \
                             windows, such as a game engine or a simulator. Nothing is recorded \
                             while no preview is open.",
                            appearance,
                        ))
                        .with_child(
                            Flex::row()
                                .with_spacing(4.)
                                .with_child(start_page::link(
                                    "Allow Screen Recording",
                                    &self.permission_button,
                                    PreviewViewAction::Stage(StageAction::GrantScreenRecording),
                                    appearance,
                                ))
                                .with_child(start_page::link(
                                    "Open System Settings",
                                    &self.settings_button,
                                    PreviewViewAction::OpenScreenRecordingSettings,
                                    appearance,
                                ))
                                .finish(),
                        )
                        .finish(),
                ),
            ),
            WindowList::Failed(reason) => (
                Vec::new(),
                Some(start_page::note(&format!("{reason}."), appearance)),
            ),
            WindowList::Listed(windows) if windows.is_empty() => (
                Vec::new(),
                Some(start_page::note(
                    "No other app windows are open.",
                    appearance,
                )),
            ),
            WindowList::Listed(windows) => (
                windows
                    .iter()
                    .take(MAX_WINDOWS)
                    .zip(rows.by_ref())
                    .map(|(window, mouse_state)| {
                        let detail = match (window.on_screen, window.source.title.is_empty()) {
                            (false, _) => Some("minimized".to_owned()),
                            (true, true) => None,
                            (true, false) => Some(window.source.title.clone()),
                        };
                        start_page::row(
                            start_page::RowContent {
                                icon: Icon::Laptop,
                                title: &window.source.app_name,
                                detail: detail.as_deref(),
                                hover_hint: "Preview",
                            },
                            mouse_state,
                            PreviewViewAction::OpenWindow(window.source.clone()),
                            appearance,
                        )
                    })
                    .collect(),
                None,
            ),
        };
        children.push(start_page::section(
            Icon::Laptop,
            "App windows",
            window_rows,
            window_note,
            appearance,
        ));

        if let Some(section) = self.render_simulators(rows.by_ref(), appearance) {
            children.push(section);
        }

        children.push(start_page::section(
            Icon::AgentMode,
            "Agents",
            Vec::new(),
            Some(
                Flex::column()
                    .with_child(start_page::note(
                        "Agents in Warp terminals list, open, look at and drive previews with \
                         the preview_* tools, asking you before they see an app.",
                        appearance,
                    ))
                    .with_child(start_page::link(
                        "Set up Claude Code tools",
                        &self.agent_setup_button,
                        PreviewViewAction::SetUpAgentTools,
                        appearance,
                    ))
                    .finish(),
            ),
            appearance,
        ));

        start_page::page(children)
    }

    /// The iOS simulators section, or `None` where simulators can't be previewed.
    fn render_simulators<'a>(
        &self,
        rows: impl Iterator<Item = &'a MouseStateHandle>,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let (simulator_rows, note) = match &self.simulators {
            SimulatorList::Unsupported => return None,
            SimulatorList::Loading => (
                Vec::new(),
                Some(start_page::note("Looking for simulators…", appearance)),
            ),
            SimulatorList::NeedsBaguette => (
                Vec::new(),
                Some(start_page::note(
                    "Simulators run here without their own window through Baguette. Install it \
                     with `brew install baguette`, then open this page again.",
                    appearance,
                )),
            ),
            SimulatorList::Failed(reason) => (
                Vec::new(),
                Some(start_page::note(&format!("{reason}."), appearance)),
            ),
            SimulatorList::Listed(simulators) if simulators.is_empty() => (
                Vec::new(),
                Some(start_page::note(
                    "No iOS simulators are installed. Add one in Xcode.",
                    appearance,
                )),
            ),
            SimulatorList::Listed(simulators) => (
                simulators
                    .iter()
                    .take(MAX_SIMULATORS)
                    .zip(rows)
                    .map(|(simulator, mouse_state)| {
                        let detail = if simulator.booted {
                            format!("{} · running", simulator.runtime)
                        } else {
                            simulator.runtime.clone()
                        };
                        start_page::row(
                            start_page::RowContent {
                                icon: Icon::Phone,
                                title: &simulator.source.name,
                                detail: Some(&detail),
                                hover_hint: "Preview",
                            },
                            mouse_state,
                            PreviewViewAction::OpenSimulator(simulator.source.clone()),
                            appearance,
                        )
                    })
                    .collect(),
                None,
            ),
        };
        Some(start_page::section(
            Icon::Phone,
            "iOS simulators",
            simulator_rows,
            note,
            appearance,
        ))
    }

    /// Which browser page previews use, or how to get one.
    fn render_chromium_status(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        match PreviewStreams::as_ref(app).chromium() {
            ChromiumState::Looking => start_page::note("Looking for a browser…", appearance),
            ChromiumState::Found(path) => start_page::note(
                &format!("Pages run headless in {}.", browser_name(path)),
                appearance,
            ),
            ChromiumState::Downloading => start_page::note("Downloading Chromium…", appearance),
            ChromiumState::Missing | ChromiumState::Failed(_) => {
                let text = match PreviewStreams::as_ref(app).chromium() {
                    ChromiumState::Failed(reason) => format!("{reason}."),
                    _ => "Page previews need Chrome or Chromium. Warp can download a headless \
                          Chromium for you (about 100 MB), or use Chrome once it's installed."
                        .to_owned(),
                };
                Flex::column()
                    .with_child(start_page::note(&text, appearance))
                    .with_child(start_page::link(
                        "Download for me",
                        &self.download_button,
                        PreviewViewAction::Stage(StageAction::DownloadChromium),
                        appearance,
                    ))
                    .finish()
            }
        }
    }
}

/// A short name for the browser at `path`, such as "Google Chrome".
fn browser_name(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    if text.contains("preview-chromium") {
        "the Chromium Warp downloaded".to_owned()
    } else if text.contains("Google Chrome") || text.contains("google-chrome") {
        "Chrome".to_owned()
    } else if text.contains("Edge") || text.contains("msedge") {
        "Edge".to_owned()
    } else if text.contains("Brave") {
        "Brave".to_owned()
    } else {
        "Chromium".to_owned()
    }
}

pub(super) fn jpeg_clipboard_content(data: Vec<u8>) -> ClipboardContent {
    ClipboardContent {
        images: Some(vec![ImageData {
            data,
            mime_type: "image/jpeg".to_owned(),
            filename: Some("preview.jpg".to_owned()),
        }]),
        ..Default::default()
    }
}

/// Saves `jpeg` to the Desktop, as macOS saves its own screenshots, and returns its path.
pub(super) fn save_screenshot(jpeg: &[u8]) -> std::io::Result<PathBuf> {
    let dir = dirs::desktop_dir()
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let path = dir.join(format!(
        "warp-preview-{}.jpg",
        chrono::Local::now().format("%Y-%m-%d-%H%M%S")
    ));
    std::fs::write(&path, jpeg)?;
    Ok(path)
}

impl Entity for PreviewView {
    type Event = PreviewViewEvent;
}

impl View for PreviewView {
    fn ui_name() -> &'static str {
        "PreviewView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focused = true;
            ctx.notify();
        }
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.focused = false;
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let front = match self.front {
            Some(front) if !self.show_start_page => front,
            _ => return self.render_start_page(app),
        };
        render_stage(
            &self.cards,
            front,
            &self.stage_mouse_states,
            StageOptions {
                position_id: &self.position_id,
                in_picture_in_picture: false,
                focused: self.focused,
                approval: self
                    .approval
                    .as_ref()
                    .map(|approval| approval.name.as_str()),
                agent_step: self.agent_step.as_deref(),
                notice: self.notice,
            },
            PreviewViewAction::Stage,
            app,
        )
    }
}

impl TypedActionView for PreviewView {
    type Action = PreviewViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            PreviewViewAction::Stage(action) => self.handle_stage_action(action, ctx),
            PreviewViewAction::OpenUrl(url) => self.open_source(
                Source::Browser {
                    url: warp_browser::resolve_input(url),
                },
                ctx,
            ),
            PreviewViewAction::OpenWindow(window) => {
                self.open_source(Source::Window(window.clone()), ctx)
            }
            PreviewViewAction::OpenSimulator(simulator) => {
                self.open_source(Source::Simulator(simulator.clone()), ctx)
            }
            PreviewViewAction::BackToStage => {
                if !self.cards.is_empty() {
                    self.show_start_page = false;
                    self.sync_title(ctx);
                    ctx.focus_self();
                    ctx.notify();
                }
            }
            PreviewViewAction::RefreshWindows => self.refresh_windows(ctx),
            PreviewViewAction::OpenScreenRecordingSettings => {
                ctx.open_url(warp_preview::window::PERMISSION_SETTINGS_URL);
            }
            PreviewViewAction::FocusUrlField => {
                self.show_start_page = true;
                ctx.focus(&self.url_editor);
                ctx.notify();
            }
            PreviewViewAction::SetUpAgentTools => {
                ctx.dispatch_typed_action(&WorkspaceAction::SetUpClaudeCodeBrowserTools);
            }
        }
    }
}

impl BackingView for PreviewView {
    type PaneHeaderOverflowMenuAction = PreviewHeaderAction;
    type CustomAction = PreviewViewAction;
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        _ctx: &mut ViewContext<Self>,
    ) {
        match *action {}
    }

    fn handle_custom_action(&mut self, action: &PreviewViewAction, ctx: &mut ViewContext<Self>) {
        self.handle_action(action, ctx);
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(PreviewViewEvent::Pane(PaneEvent::Close));
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus(ctx);
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> HeaderContent {
        HeaderContent::simple(self.pane_configuration.as_ref(app).title())
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}
