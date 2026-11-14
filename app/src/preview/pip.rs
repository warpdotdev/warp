use std::time::Duration;

use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{Vector2F, vec2f};
use warp_preview::Rate;
use warpui::r#async::Timer;
use warpui::elements::{
    ChildAnchor, ConstrainedBox, Container, CornerRadius, Draggable, DraggableState,
    OffsetPositioning, ParentAnchor, ParentOffsetBounds, Radius,
};
use warpui::prelude::DropShadow;
use warpui::{
    AppContext, BlurContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View,
    ViewContext,
};

use super::playing::{self, PlayingInput};
use super::stage::{StageAction, StageMouseStates, StageOptions, render_stage};
use super::streams::{PreviewId, PreviewStreams};
use super::view::{AppApproval, save_screenshot};
use crate::browser::AgentApproval;
use crate::workspace::WorkspaceAction;

const PIP_WIDTH: f32 = 420.;
const PIP_HEIGHT: f32 = 270.;
/// Space between the picture-in-picture window and the edges of the Warp window.
const PIP_MARGIN: f32 = 16.;
const NOTICE_DURATION: Duration = Duration::from_secs(2);
const AGENT_STEP_LINGER: Duration = Duration::from_secs(4);

/// The corner of the Warp window the picture-in-picture window sits in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PipCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
}

impl PipCorner {
    /// The corner nearest to where a window of size `window_size` was dropped at `rect`.
    pub fn nearest(rect: RectF, window_size: Vector2F) -> Self {
        let center = rect.center();
        let left = center.x() < window_size.x() / 2.;
        let top = center.y() < window_size.y() / 2.;
        match (top, left) {
            (true, true) => Self::TopLeft,
            (true, false) => Self::TopRight,
            (false, true) => Self::BottomLeft,
            (false, false) => Self::BottomRight,
        }
    }

    /// Where the picture-in-picture window goes in a stack covering the Warp window.
    pub fn positioning(self) -> OffsetPositioning {
        let (offset, anchor, child_anchor) = match self {
            Self::TopLeft => (
                vec2f(PIP_MARGIN, PIP_MARGIN),
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
            Self::TopRight => (
                vec2f(-PIP_MARGIN, PIP_MARGIN),
                ParentAnchor::TopRight,
                ChildAnchor::TopRight,
            ),
            Self::BottomLeft => (
                vec2f(PIP_MARGIN, -PIP_MARGIN),
                ParentAnchor::BottomLeft,
                ChildAnchor::BottomLeft,
            ),
            Self::BottomRight => (
                vec2f(-PIP_MARGIN, -PIP_MARGIN),
                ParentAnchor::BottomRight,
                ChildAnchor::BottomRight,
            ),
        };
        OffsetPositioning::offset_from_parent(
            offset,
            ParentOffsetBounds::ParentByPosition,
            anchor,
            child_anchor,
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PreviewPipViewAction {
    Stage(StageAction),
    Dropped(RectF),
}

/// A small floating stage over the workspace that keeps previews in view while the user works in
/// other panes or tabs. It can be dragged to any corner.
pub struct PreviewPipView {
    cards: Vec<PreviewId>,
    front: PreviewId,
    corner: PipCorner,
    position_id: String,
    draggable_state: DraggableState,
    stage_mouse_states: StageMouseStates,
    notice: Option<&'static str>,
    agent_step: Option<String>,
    agent_step_generation: u64,
    approval: Option<AppApproval>,
    focused: bool,
    playing_input: PlayingInput,
}

impl PreviewPipView {
    pub fn new(cards: Vec<PreviewId>, front: PreviewId, ctx: &mut ViewContext<Self>) -> Self {
        ctx.observe(&PreviewStreams::handle(ctx), |me, _, ctx| {
            me.forget_closed_previews(ctx);
            ctx.notify();
        });
        let mut stage_mouse_states = StageMouseStates::default();
        stage_mouse_states.ensure_cards(cards.len());
        Self {
            cards,
            front,
            corner: PipCorner::default(),
            position_id: format!("preview_pip_stage_{}", ctx.view_id()),
            draggable_state: DraggableState::default(),
            stage_mouse_states,
            notice: None,
            agent_step: None,
            agent_step_generation: 0,
            approval: None,
            focused: false,
            playing_input: PlayingInput::default(),
        }
    }

    pub fn corner(&self) -> PipCorner {
        self.corner
    }

    pub fn previews(&self) -> &[PreviewId] {
        &self.cards
    }

    pub fn add_previews(
        &mut self,
        previews: Vec<PreviewId>,
        front: PreviewId,
        ctx: &mut ViewContext<Self>,
    ) {
        for id in previews {
            if !self.cards.contains(&id) {
                self.cards.push(id);
            }
        }
        self.stage_mouse_states.ensure_cards(self.cards.len());
        self.front = front;
        ctx.notify();
    }

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
        ctx.notify();
    }

    pub fn clear_approval(&mut self, ctx: &mut ViewContext<Self>) {
        self.approval = None;
        ctx.notify();
    }

    /// Updates the front preview at full rate while the picture-in-picture window is drawn. The
    /// others are not shown here, so they pause.
    pub(super) fn sync_rates(&self, ctx: &mut ViewContext<Self>) {
        let visible = ctx
            .element_position_by_id_at_last_frame(ctx.window_id(), &self.position_id)
            .is_some();
        let front = self.front;
        let cards = self.cards.clone();
        PreviewStreams::handle(ctx).update(ctx, |streams, _| {
            for id in cards {
                let rate = if visible && id == front {
                    Rate::Full
                } else {
                    Rate::Paused
                };
                streams.set_rate(id, rate);
            }
        });
    }

    fn forget_closed_previews(&mut self, ctx: &mut ViewContext<Self>) {
        let streams = PreviewStreams::as_ref(ctx);
        self.cards.retain(|id| streams.get(*id).is_some());
        match self.cards.last() {
            None => self.dismiss(ctx),
            Some(last) if !self.cards.contains(&self.front) => self.front = *last,
            Some(_) => {}
        }
    }

    /// Removes the picture-in-picture window. Its previews keep running only if something else
    /// took them.
    fn dismiss(&mut self, ctx: &mut ViewContext<Self>) {
        if self.approval.take().is_some() {
            let view_id = ctx.view_id();
            super::PreviewAgent::handle(ctx)
                .update(ctx, |agent, ctx| agent.cancel_for_view(view_id, ctx));
        }
        super::close_picture_in_picture(ctx);
    }

    /// Moves the previews back into a preview pane.
    fn return_to_pane(&mut self, ctx: &mut ViewContext<Self>) {
        let mut previews = std::mem::take(&mut self.cards);
        // The pane brings its last preview to the front.
        previews.retain(|id| *id != self.front);
        previews.push(self.front);
        ctx.dispatch_typed_action(&WorkspaceAction::OpenPreviewPane { previews });
        self.dismiss(ctx);
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

    fn handle_stage_action(&mut self, action: &StageAction, ctx: &mut ViewContext<Self>) {
        match action {
            StageAction::BringToFront(id) => {
                self.front = *id;
                self.playing_input = PlayingInput::default();
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
            }
            StageAction::Screenshot => {
                let jpeg = PreviewStreams::as_ref(ctx)
                    .get(self.front)
                    .and_then(|preview| preview.latest_jpeg().cloned());
                let Some(jpeg) = jpeg else {
                    self.show_notice("Nothing to capture yet", ctx);
                    return;
                };
                match save_screenshot(&jpeg) {
                    Ok(_) => {
                        ctx.clipboard()
                            .write(super::view::jpeg_clipboard_content(jpeg.to_vec()));
                        self.show_notice("Saved and copied", ctx);
                    }
                    Err(err) => {
                        log::warn!("Failed to save a preview screenshot: {err:#}");
                        self.show_notice("Couldn't save it", ctx);
                    }
                }
            }
            StageAction::Rotate => {
                PreviewStreams::handle(ctx).update(ctx, |streams, _| {
                    streams.rotate(self.front);
                });
            }
            StageAction::TogglePictureInPicture | StageAction::AddPreview => {
                self.return_to_pane(ctx)
            }
            StageAction::ResolveApproval(decision) => self.resolve_approval(*decision, ctx),
            StageAction::SetPlaying(playing) => {
                self.playing_input = PlayingInput::default();
                if let Err(notice) = playing::set_playing(self.front, *playing, ctx) {
                    self.show_notice(notice, ctx);
                }
            }
            StageAction::Input(input) => {
                self.playing_input
                    .forward(self.front, input, &self.position_id, ctx);
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
}

impl Entity for PreviewPipView {
    type Event = ();
}

impl View for PreviewPipView {
    fn ui_name() -> &'static str {
        "PreviewPipView"
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
        let stage = render_stage(
            &self.cards,
            self.front,
            &self.stage_mouse_states,
            StageOptions {
                position_id: &self.position_id,
                in_picture_in_picture: true,
                focused: self.focused,
                approval: self
                    .approval
                    .as_ref()
                    .map(|approval| approval.name.as_str()),
                agent_step: self.agent_step.as_deref(),
                notice: self.notice,
            },
            PreviewPipViewAction::Stage,
            app,
        );
        let window = ConstrainedBox::new(
            Container::new(stage)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
                .with_drop_shadow(DropShadow::new_with_standard_offset_and_spread(
                    ColorU::new(0, 0, 0, 110),
                ))
                .finish(),
        )
        .with_width(PIP_WIDTH)
        .with_height(PIP_HEIGHT)
        .finish();
        Draggable::new(self.draggable_state.clone(), window)
            .with_defer_to_handled_child_mouse_down()
            .on_drop(|ctx, _, rect, _| {
                ctx.dispatch_typed_action(PreviewPipViewAction::Dropped(rect))
            })
            .finish()
    }
}

impl TypedActionView for PreviewPipView {
    type Action = PreviewPipViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            PreviewPipViewAction::Stage(action) => self.handle_stage_action(action, ctx),
            PreviewPipViewAction::Dropped(rect) => {
                let window_id = ctx.window_id();
                if let Some(bounds) = ctx.window_bounds(&window_id) {
                    self.corner = PipCorner::nearest(*rect, bounds.size());
                    ctx.notify();
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "pip_tests.rs"]
mod tests;
