//! What a preview pane and the picture-in-picture window both show: the front preview, the other
//! previews as a pile of live cards over its corner, and a thin floating bar of controls over the
//! picture.

use std::time::Duration;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::{Vector2F, vec2f};
use warp_core::ui::appearance::Appearance;
use warp_preview::Source;
use warp_preview::input::{Modifiers, PointerButton};
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{
    Align, Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    DispatchEventResult, Empty, EventDispatchMode, EventHandler, Expanded, Flex, Hoverable, Image,
    MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Radius, SavePosition, Stack, Text,
};
use warpui::event::ModifiersState;
use warpui::fonts::FamilyId;
use warpui::image_cache::CacheOption;
use warpui::keymap::Keystroke;
use warpui::scene::DropShadow;
use warpui::ui_components::components::UiComponent;
use warpui::{Action, AppContext, Element, EventContext, SingletonEntity};

use super::streams::{ChromiumState, Preview, PreviewId, PreviewStatus, PreviewStreams};
use crate::browser::AgentApproval;
use crate::ui_components::icons::Icon;

/// Height of the floating control bar.
const BAR_HEIGHT: f32 = 30.;
const BAR_ICON_SIZE: f32 = 15.;
const BAR_BUTTON_WIDTH: f32 = 28.;
const PICTURE_CORNER_RADIUS: f32 = 10.;
/// Height of the row under the picture that holds the badge, the controls and the pile.
const FOOTER_HEIGHT: f32 = 46.;
const PIP_FOOTER_HEIGHT: f32 = 38.;
/// How many cards the collapsed pile draws; the rest are counted on its badge.
const PILE_DEPTH: usize = 3;
/// How far each card in the collapsed pile sits up and to the left of the one in front of it.
const PILE_STEP: f32 = 4.;
/// Keeps the pile fanned out while the pointer crosses the gaps between cards.
const PILE_HOVER_OUT_DELAY: Duration = Duration::from_millis(250);

/// The stage's colors, from the theme.
#[derive(Clone, Copy)]
struct Palette {
    background: ColorU,
    /// The translucent surface the controls float on over the picture.
    glass: ColorU,
    line: ColorU,
    hover: ColorU,
    text: ColorU,
    muted: ColorU,
    /// A switched-on control, and the text on it.
    lit: ColorU,
    on_lit: ColorU,
    /// Marks an agent acting on the preview.
    agent: ColorU,
}

impl Palette {
    fn new(appearance: &Appearance) -> Self {
        let theme = appearance.theme();
        let lit = theme.accent().into_solid();
        Self {
            background: theme.background().into_solid(),
            glass: ColorU {
                a: 235,
                ..theme.surface_2().into_solid()
            },
            line: theme.outline().into_solid(),
            hover: theme.surface_3().into_solid(),
            text: theme.active_ui_text_color().into_solid(),
            muted: theme.nonactive_ui_text_color().into_solid(),
            lit,
            on_lit: theme.font_color(lit).into_solid(),
            agent: theme.ui_warning_color(),
        }
    }
}

/// Something the user did on the stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StageAction {
    BringToFront(PreviewId),
    Close(PreviewId),
    Retry(PreviewId),
    DownloadChromium,
    GrantScreenRecording,
    Screenshot,
    /// Rotates the front simulator to its next orientation.
    Rotate,
    /// Moves the previews between the pane and the picture-in-picture window.
    TogglePictureInPicture,
    AddPreview,
    ResolveApproval(AgentApproval),
    /// Switches the front preview between Watching and Playing.
    SetPlaying(bool),
    /// Pointer or keyboard input on the front preview while it is playing.
    Input(StageInput),
}

/// The user's input on a playing preview. Positions are window points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StageInput {
    Down {
        at: (i32, i32),
        button: PointerButton,
        modifiers: Modifiers,
    },
    Up {
        at: (i32, i32),
    },
    Drag {
        at: (i32, i32),
    },
    /// The pointer moved over the picture with no button held.
    Hover {
        at: (i32, i32),
    },
    /// A wheel or trackpad scroll, in points as AppKit reports them.
    Scroll {
        delta: (i32, i32),
    },
    Key(Keystroke),
}

impl StageInput {
    pub fn point(position: Vector2F) -> (i32, i32) {
        (position.x().round() as i32, position.y().round() as i32)
    }
}
/// Mouse state for everything clickable on a stage. Owned by the view showing the stage.
#[derive(Default)]
pub struct StageMouseStates {
    watch: MouseStateHandle,
    control: MouseStateHandle,
    screenshot: MouseStateHandle,
    rotate: MouseStateHandle,
    picture_in_picture: MouseStateHandle,
    add: MouseStateHandle,
    status_button: MouseStateHandle,
    secondary_status_button: MouseStateHandle,
    approve_once: MouseStateHandle,
    approve_always: MouseStateHandle,
    deny: MouseStateHandle,
    pile: MouseStateHandle,
    cards: Vec<(MouseStateHandle, MouseStateHandle)>,
}

impl StageMouseStates {
    /// Makes sure there is mouse state for `count` cards.
    pub fn ensure_cards(&mut self, count: usize) {
        while self.cards.len() < count {
            self.cards.push(Default::default());
        }
    }
}

/// What a stage shows besides its previews.
pub struct StageOptions<'a> {
    pub position_id: &'a str,
    /// Whether the stage is the picture-in-picture window, which changes the pop-out button and
    /// shrinks the pile.
    pub in_picture_in_picture: bool,
    /// Whether the view showing the stage has keyboard focus, so a playing preview takes keys.
    pub focused: bool,
    /// An app an agent is waiting for the user to approve.
    pub approval: Option<&'a str>,
    /// What an agent is doing with the front preview right now.
    pub agent_step: Option<&'a str>,
    pub notice: Option<&'a str>,
}

/// Renders `cards` with `front` large. `wrap` turns stage actions into the owning view's actions.
pub fn render_stage<A: Action + Clone>(
    cards: &[PreviewId],
    front: PreviewId,
    mouse_states: &StageMouseStates,
    options: StageOptions<'_>,
    wrap: fn(StageAction) -> A,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let palette = Palette::new(appearance);
    let streams = PreviewStreams::as_ref(app);
    let front_preview = streams.get(front);
    let playing = front_preview.is_some_and(|preview| preview.playing);

    // Saved for a single frame so the stage counts as hidden whenever it is not drawn, which
    // pauses its previews.
    let mut picture_element = SavePosition::new(
        render_picture(front_preview, streams, mouse_states, wrap, appearance),
        options.position_id,
    )
    .for_single_frame()
    .finish();
    if playing {
        picture_element = forward_input(picture_element, options.focused, wrap);
    }
    // The approval banner over the picture takes its own clicks; in the default broadcast mode a
    // click on it would also reach a playing preview underneath.
    let mut picture = Stack::new()
        .with_event_dispatch_mode(EventDispatchMode::Waterfall)
        .with_child(picture_element);
    if let Some(app_name) = options.approval {
        picture.add_positioned_child(
            render_approval(app_name, mouse_states, wrap, appearance),
            OffsetPositioning::offset_from_parent(
                vec2f(0., 12.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopMiddle,
                ChildAnchor::TopMiddle,
            ),
        );
    }
    let mut picture = Container::new(picture.finish())
        .with_padding_top(10.)
        .with_horizontal_padding(10.);
    if options.agent_step.is_some() {
        picture = picture.with_border(Border::all(2.).with_border_fill(palette.agent));
    }

    let (footer_height, pile_size) = if options.in_picture_in_picture {
        (PIP_FOOTER_HEIGHT, PileSize::SMALL)
    } else {
        (FOOTER_HEIGHT, PileSize::REGULAR)
    };
    // The footer keeps the controls off the picture: the badge on the left, the capsule in the
    // middle and the pile on the right, on one line.
    let mut footer = Stack::new()
        .with_event_dispatch_mode(EventDispatchMode::Waterfall)
        .with_child(
            ConstrainedBox::new(
                Align::new(render_bar(
                    &options,
                    front_preview,
                    mouse_states,
                    wrap,
                    appearance,
                ))
                .finish(),
            )
            .with_height(footer_height)
            .finish(),
        );
    footer.add_positioned_child(
        render_badge(options.agent_step, playing, appearance),
        OffsetPositioning::offset_from_parent(
            vec2f(12., 0.),
            ParentOffsetBounds::ParentByPosition,
            ParentAnchor::MiddleLeft,
            ChildAnchor::MiddleLeft,
        ),
    );
    if cards.len() > 1 {
        // Anchored at the bottom so the pile fans out upward, over the picture, while hovered.
        footer.add_positioned_child(
            render_pile(
                cards,
                front,
                pile_size,
                streams,
                mouse_states,
                wrap,
                appearance,
            ),
            OffsetPositioning::offset_from_parent(
                vec2f(-12., -((footer_height - pile_size.collapsed_height()) / 2.)),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::BottomRight,
                ChildAnchor::BottomRight,
            ),
        );
    }

    Container::new(
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(Expanded::new(1., picture.finish()).finish())
            .with_child(footer.finish())
            .finish(),
    )
    .with_background(palette.background)
    .finish()
}

fn render_picture<A: Action + Clone>(
    preview: Option<&Preview>,
    streams: &PreviewStreams,
    mouse_states: &StageMouseStates,
    wrap: fn(StageAction) -> A,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let Some(preview) = preview else {
        return Empty::new().finish();
    };
    let message = status_message(preview, streams, mouse_states, wrap, appearance);
    if preview.frame_size.is_none() || message.is_some() {
        let mut stack = Stack::new().with_child(Empty::new().finish());
        if preview.frame_size.is_some() {
            stack.add_child(frame_image(preview, 0.35));
        }
        if let Some(message) = message {
            stack.add_child(Align::new(message).finish());
        }
        return stack.finish();
    }
    frame_image(preview, 1.)
}

fn frame_image(preview: &Preview, opacity: f32) -> Box<dyn Element> {
    frame_image_from(preview.frame_asset(), opacity)
}

fn frame_image_from(asset: AssetSource, opacity: f32) -> Box<dyn Element> {
    Image::new(asset, CacheOption::BySize)
        .contain()
        .with_opacity(opacity)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
            PICTURE_CORNER_RADIUS,
        )))
        .finish()
}

/// Sends the user's pointer and, while `focused`, keys over `picture` to the front preview.
fn forward_input<A: Action + Clone>(
    picture: Box<dyn Element>,
    focused: bool,
    wrap: fn(StageAction) -> A,
) -> Box<dyn Element> {
    let send = move |ctx: &mut EventContext, input: StageInput| {
        ctx.dispatch_typed_action(wrap(StageAction::Input(input)));
        DispatchEventResult::StopPropagation
    };
    let modifiers = |state: &ModifiersState| Modifiers {
        cmd: state.cmd,
        shift: state.shift,
        alt: state.alt,
        ctrl: state.ctrl,
    };
    let mut handler = EventHandler::new(picture)
        .on_left_mouse_down(move |ctx, _, position| {
            send(
                ctx,
                StageInput::Down {
                    at: StageInput::point(position),
                    button: PointerButton::Left,
                    modifiers: Modifiers::default(),
                },
            )
        })
        .on_left_mouse_up(move |ctx, _, position| {
            send(
                ctx,
                StageInput::Up {
                    at: StageInput::point(position),
                },
            )
        })
        .on_mouse_dragged(move |ctx, _, position| {
            send(
                ctx,
                StageInput::Drag {
                    at: StageInput::point(position),
                },
            )
        })
        .on_right_mouse_down(move |ctx, _, position, state| {
            send(
                ctx,
                StageInput::Down {
                    at: StageInput::point(position),
                    button: PointerButton::Right,
                    modifiers: modifiers(state),
                },
            )
        })
        .on_middle_mouse_down(move |ctx, _, position| {
            send(
                ctx,
                StageInput::Down {
                    at: StageInput::point(position),
                    button: PointerButton::Middle,
                    modifiers: Modifiers::default(),
                },
            )
        })
        .on_mouse_in(
            move |ctx, _, position| {
                ctx.dispatch_typed_action(wrap(StageAction::Input(StageInput::Hover {
                    at: StageInput::point(position),
                })));
                DispatchEventResult::PropagateToParent
            },
            None,
        )
        .on_scroll_wheel(move |ctx, _, delta, _| {
            send(
                ctx,
                StageInput::Scroll {
                    delta: StageInput::point(delta),
                },
            )
        });
    if focused {
        handler = handler
            .on_keydown(move |ctx, _, keystroke| send(ctx, StageInput::Key(keystroke.clone())));
    }
    handler.finish()
}

/// What to show over or instead of the picture, when the preview is not simply live.
fn status_message<A: Action + Clone>(
    preview: &Preview,
    streams: &PreviewStreams,
    mouse_states: &StageMouseStates,
    wrap: fn(StageAction) -> A,
    appearance: &Appearance,
) -> Option<Box<dyn Element>> {
    let palette = Palette::new(appearance);
    let id = preview.id;
    let (text, buttons): (String, Vec<(&str, StageAction)>) = match &preview.status {
        PreviewStatus::Live => return None,
        PreviewStatus::Starting => (format!("Starting {}…", preview.label()), Vec::new()),
        PreviewStatus::Minimized => (
            "This window is minimized or on another desktop. Restore it to watch it here."
                .to_owned(),
            Vec::new(),
        ),
        PreviewStatus::NeedsPermission => (
            "Warp needs the Screen Recording permission to show other apps' windows. Nothing is \
             recorded while no preview is open."
                .to_owned(),
            vec![("Allow Screen Recording", StageAction::GrantScreenRecording)],
        ),
        PreviewStatus::NeedsChromium => match streams.chromium() {
            ChromiumState::Looking => ("Looking for a browser…".to_owned(), Vec::new()),
            ChromiumState::Downloading => ("Downloading Chromium…".to_owned(), Vec::new()),
            ChromiumState::Failed(reason) => (
                format!("{reason}. Installing Chrome also works."),
                vec![("Try again", StageAction::DownloadChromium)],
            ),
            ChromiumState::Missing | ChromiumState::Found(_) => (
                "Page previews run in a headless Chromium. Warp can download one (about 100 MB), \
                 or use Chrome once it's installed."
                    .to_owned(),
                vec![("Download for me", StageAction::DownloadChromium)],
            ),
        },
        PreviewStatus::Ended(reason) => (
            format!("{reason}."),
            vec![
                ("Try again", StageAction::Retry(id)),
                ("Close", StageAction::Close(id)),
            ],
        ),
    };
    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(12.)
        .with_child(
            ConstrainedBox::new(
                Text::new(text, appearance.ui_font_family(), appearance.ui_font_size())
                    .with_color(palette.text)
                    .finish(),
            )
            .with_max_width(360.)
            .finish(),
        );
    if !buttons.is_empty() {
        let handles = [
            &mouse_states.status_button,
            &mouse_states.secondary_status_button,
        ];
        let mut row = Flex::row().with_spacing(8.);
        for (index, (label, action)) in buttons.into_iter().enumerate() {
            row.add_child(pill_button(
                label,
                index == 0,
                handles[index.min(1)],
                wrap(action),
                appearance,
            ));
        }
        column.add_child(row.finish());
    }
    Some(
        Container::new(column.finish())
            .with_uniform_padding(16.)
            .with_background(palette.glass)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
            .finish(),
    )
}

/// "Watching" or "Playing", or what an agent is doing while it acts.
fn render_badge(
    agent_step: Option<&str>,
    playing: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let palette = Palette::new(appearance);
    let (dot, label) = match (agent_step, playing) {
        (Some(step), _) => (palette.agent, step.to_owned()),
        (None, true) => (palette.lit, "Playing".to_owned()),
        (None, false) => (palette.muted, "Watching".to_owned()),
    };
    Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.)
            .with_child(
                ConstrainedBox::new(
                    Container::new(Empty::new().finish())
                        .with_background(dot)
                        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                        .finish(),
                )
                .with_width(6.)
                .with_height(6.)
                .finish(),
            )
            .with_child(
                Text::new_inline(label, appearance.ui_font_family(), 11.5)
                    .with_color(palette.text)
                    .finish(),
            )
            .finish(),
    )
    .with_horizontal_padding(10.)
    .with_vertical_padding(3.)
    .with_background(palette.glass)
    .with_border(Border::all(1.).with_border_fill(palette.line))
    .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
    .finish()
}

/// The thin floating bar of icon buttons over the bottom of the picture.
fn render_bar<A: Action + Clone>(
    options: &StageOptions<'_>,
    front: Option<&Preview>,
    mouse_states: &StageMouseStates,
    wrap: fn(StageAction) -> A,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let palette = Palette::new(appearance);
    let playing = front.is_some_and(|preview| preview.playing);
    let lit_when = |lit: bool| {
        if lit {
            BarButtonState::Lit
        } else {
            BarButtonState::Normal
        }
    };
    let play_tip = match front.map(|preview| &preview.source) {
        Some(Source::Window(_)) => {
            "Playing: your pointer and keys go to the window, which stays behind Warp"
        }
        Some(Source::Simulator(_)) => {
            "Playing: your pointer and keys go to the simulator, as touches and its keyboard"
        }
        Some(Source::Browser { .. }) | None => "Playing: your pointer and keys go to the page",
    };
    let switch = Container::new(
        Flex::row()
            .with_spacing(1.)
            .with_child(bar_button(
                Icon::Eye,
                "Watching: your input stays in Warp",
                lit_when(!playing),
                &mouse_states.watch,
                Some(wrap(StageAction::SetPlaying(false))),
                appearance,
            ))
            .with_child(bar_button(
                Icon::Hand,
                play_tip,
                lit_when(playing),
                &mouse_states.control,
                Some(wrap(StageAction::SetPlaying(true))),
                appearance,
            ))
            .finish(),
    )
    .with_uniform_padding(1.)
    .with_background(appearance.theme().dark_overlay())
    .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
    .finish();

    let (picture_in_picture_icon, picture_in_picture_tip) = if options.in_picture_in_picture {
        (Icon::Maximize, "Back to a pane")
    } else {
        (Icon::Minimize, "Picture in picture")
    };
    let mut bar = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(1.)
        .with_child(switch)
        .with_child(separator(palette))
        .with_child(bar_button(
            Icon::Image,
            "Screenshot: save and copy the picture",
            BarButtonState::Normal,
            &mouse_states.screenshot,
            Some(wrap(StageAction::Screenshot)),
            appearance,
        ))
        .with_child(
            if matches!(
                front.map(|preview| &preview.source),
                Some(Source::Simulator(_))
            ) {
                bar_button(
                    Icon::RefreshCw04,
                    "Rotate simulator",
                    BarButtonState::Normal,
                    &mouse_states.rotate,
                    Some(wrap(StageAction::Rotate)),
                    appearance,
                )
            } else {
                Empty::new().finish()
            },
        )
        .with_child(bar_button(
            picture_in_picture_icon,
            picture_in_picture_tip,
            BarButtonState::Normal,
            &mouse_states.picture_in_picture,
            Some(wrap(StageAction::TogglePictureInPicture)),
            appearance,
        ))
        .with_child(bar_button(
            Icon::Plus,
            "Add a preview",
            BarButtonState::Normal,
            &mouse_states.add,
            Some(wrap(StageAction::AddPreview)),
            appearance,
        ));
    if let Some(notice) = options.notice {
        bar.add_child(
            Container::new(
                Text::new_inline(notice.to_owned(), appearance.ui_font_family(), 11.5)
                    .with_color(palette.text)
                    .finish(),
            )
            .with_horizontal_padding(8.)
            .finish(),
        );
    }
    ConstrainedBox::new(
        Container::new(bar.finish())
            .with_uniform_padding(3.)
            .with_background(palette.glass)
            .with_border(Border::all(1.).with_border_fill(palette.line))
            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
            .finish(),
    )
    .with_height(BAR_HEIGHT)
    .finish()
}

fn separator(palette: Palette) -> Box<dyn Element> {
    Container::new(
        ConstrainedBox::new(
            Container::new(Empty::new().finish())
                .with_background(palette.line)
                .finish(),
        )
        .with_width(1.)
        .with_height(14.)
        .finish(),
    )
    .with_horizontal_margin(3.)
    .finish()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BarButtonState {
    Normal,
    Lit,
}

/// An icon button in the bar. Its tooltip appears above it right away, since the bar has no
/// labels.
fn bar_button<A: Action + Clone>(
    icon: Icon,
    tooltip: &'static str,
    state: BarButtonState,
    mouse_state: &MouseStateHandle,
    action: Option<A>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let palette = Palette::new(appearance);
    let ui_builder = appearance.ui_builder().clone();
    let hoverable = Hoverable::new(mouse_state.clone(), move |mouse| {
        let color = match state {
            BarButtonState::Lit => palette.on_lit,
            BarButtonState::Normal => palette.text,
        };
        let background = match state {
            BarButtonState::Lit => Some(palette.lit),
            BarButtonState::Normal if mouse.is_hovered() => Some(palette.hover),
            BarButtonState::Normal => None,
        };
        let mut button = Container::new(
            Align::new(
                ConstrainedBox::new(icon.to_warpui_icon(color.into()).finish())
                    .with_width(BAR_ICON_SIZE)
                    .with_height(BAR_ICON_SIZE)
                    .finish(),
            )
            .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)));
        if let Some(background) = background {
            button = button.with_background(background);
        }
        let button = ConstrainedBox::new(button.finish())
            .with_width(BAR_BUTTON_WIDTH)
            .with_height(BAR_HEIGHT - 8.)
            .finish();
        if !mouse.is_hovered() {
            return button;
        }
        let mut stack = Stack::new().with_child(button);
        stack.add_positioned_overlay_child(
            ui_builder.tool_tip(tooltip.to_owned()).build().finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(0., -9.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::TopMiddle,
                ChildAnchor::BottomMiddle,
            ),
        );
        stack.finish()
    });
    match action {
        Some(action) => hoverable
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
            .finish(),
        None => hoverable.finish(),
    }
}

fn pill_button<A: Action + Clone>(
    label: &str,
    primary: bool,
    mouse_state: &MouseStateHandle,
    action: A,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let palette = Palette::new(appearance);
    let label = label.to_owned();
    let font_family = appearance.ui_font_family();
    let font_size = appearance.ui_font_size();
    Hoverable::new(mouse_state.clone(), move |mouse| {
        let background = match (primary, mouse.is_hovered()) {
            (true, _) => palette.lit,
            (false, true) => palette.hover,
            (false, false) => ColorU::transparent_black(),
        };
        Container::new(
            Text::new_inline(label.clone(), font_family, font_size)
                .with_color(palette.text)
                .finish(),
        )
        .with_horizontal_padding(12.)
        .with_vertical_padding(4.)
        .with_background(background)
        .with_border(Border::all(1.).with_border_fill(if primary {
            palette.lit
        } else {
            palette.line
        }))
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
        .finish()
    })
    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
    .finish()
}

/// Asks the user whether an agent may watch an app, with the browser pane's three answers.
fn render_approval<A: Action + Clone>(
    app_name: &str,
    mouse_states: &StageMouseStates,
    wrap: fn(StageAction) -> A,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let palette = Palette::new(appearance);
    let text = format!(
        "An agent wants to watch {app_name}. It will see this window and its log, but can't \
         click or type in it."
    );
    Container::new(
        Flex::column()
            .with_spacing(10.)
            .with_child(
                ConstrainedBox::new(
                    Text::new(text, appearance.ui_font_family(), appearance.ui_font_size())
                        .with_color(palette.text)
                        .finish(),
                )
                .with_max_width(380.)
                .finish(),
            )
            .with_child(
                Flex::row()
                    .with_spacing(6.)
                    .with_child(pill_button(
                        "Allow once",
                        true,
                        &mouse_states.approve_once,
                        wrap(StageAction::ResolveApproval(AgentApproval::Once)),
                        appearance,
                    ))
                    .with_child(pill_button(
                        "Always allow",
                        false,
                        &mouse_states.approve_always,
                        wrap(StageAction::ResolveApproval(AgentApproval::Always)),
                        appearance,
                    ))
                    .with_child(pill_button(
                        "Deny",
                        false,
                        &mouse_states.deny,
                        wrap(StageAction::ResolveApproval(AgentApproval::Deny)),
                        appearance,
                    ))
                    .finish(),
            )
            .finish(),
    )
    .with_uniform_padding(14.)
    .with_background(palette.glass)
    .with_border(Border::all(1.).with_border_fill(palette.line))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
    .finish()
}

/// Sizes of the pile's cards: collapsed thumbnails and fanned-out cards.
#[derive(Clone, Copy)]
struct PileSize {
    thumbnail: (f32, f32),
    card: (f32, f32),
}

impl PileSize {
    /// The collapsed pile's height, with every card it can show.
    fn collapsed_height(self) -> f32 {
        self.thumbnail.1 + (PILE_DEPTH - 1) as f32 * PILE_STEP
    }

    const REGULAR: Self = Self {
        thumbnail: (52., 29.),
        card: (152., 86.),
    };
    const SMALL: Self = Self {
        thumbnail: (40., 22.),
        card: (104., 59.),
    };
}

/// The previews behind the front one, piled up over the picture's corner. Hovering the pile fans
/// it out into a row of cards; clicking a card brings it to the front.
fn render_pile<A: Action + Clone>(
    cards: &[PreviewId],
    front: PreviewId,
    size: PileSize,
    streams: &PreviewStreams,
    mouse_states: &StageMouseStates,
    wrap: fn(StageAction) -> A,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let behind: Vec<(&Preview, &(MouseStateHandle, MouseStateHandle))> = cards
        .iter()
        .zip(&mouse_states.cards)
        .filter(|(id, _)| **id != front)
        .filter_map(|(id, states)| Some((streams.get(*id)?, states)))
        .collect();
    if behind.is_empty() {
        return Empty::new().finish();
    }

    let font_family = appearance.ui_font_family();
    Hoverable::new(mouse_states.pile.clone(), |mouse| {
        if mouse.is_hovered() {
            return glass_tray(
                Flex::row()
                    .with_spacing(6.)
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_children(behind.iter().map(|(preview, (card_state, close_state))| {
                        render_card(preview, size, card_state, close_state, wrap, appearance)
                    }))
                    .finish(),
                Palette::new(appearance),
            );
        }
        let thumbnails: Vec<Option<AssetSource>> = behind
            .iter()
            .skip(behind.len().saturating_sub(PILE_DEPTH))
            .map(|(preview, _)| preview.frame_size.map(|_| preview.frame_asset()))
            .collect();
        let hidden_count = behind.len().saturating_sub(PILE_DEPTH);
        render_collapsed_pile(
            &thumbnails,
            hidden_count,
            size,
            font_family,
            Palette::new(appearance),
        )
    })
    .with_hover_out_delay(PILE_HOVER_OUT_DELAY)
    .finish()
}

/// Up to [`PILE_DEPTH`] thumbnails stacked with a small offset, the newest in front.
fn render_collapsed_pile(
    thumbnails: &[Option<AssetSource>],
    hidden_count: usize,
    size: PileSize,
    font_family: FamilyId,
    palette: Palette,
) -> Box<dyn Element> {
    let (width, height) = size.thumbnail;
    let depth = thumbnails.len().saturating_sub(1) as f32 * PILE_STEP;
    let mut pile = Stack::new().with_child(
        ConstrainedBox::new(Empty::new().finish())
            .with_width(width + depth)
            .with_height(height + depth)
            .finish(),
    );
    for (index, asset) in thumbnails.iter().enumerate() {
        let picture = match asset {
            Some(asset) => frame_image_from(asset.clone(), 1.),
            None => Empty::new().finish(),
        };
        let thumbnail = Container::new(
            ConstrainedBox::new(picture)
                .with_width(width)
                .with_height(height)
                .finish(),
        )
        .with_background(palette.background)
        .with_border(Border::all(1.).with_border_fill(palette.line))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_drop_shadow(DropShadow::default())
        .finish();
        let offset = index as f32 * PILE_STEP;
        pile.add_positioned_child(
            thumbnail,
            OffsetPositioning::offset_from_parent(
                vec2f(offset, offset),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
        );
    }
    if hidden_count > 0 {
        pile.add_positioned_child(
            Container::new(
                Text::new_inline(format!("+{hidden_count}"), font_family, 11.)
                    .with_color(palette.text)
                    .finish(),
            )
            .with_horizontal_padding(6.)
            .with_vertical_padding(1.)
            .with_background(palette.glass)
            .with_border(Border::all(1.).with_border_fill(palette.line))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(-4., -4.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::BottomRight,
                ChildAnchor::BottomRight,
            ),
        );
    }
    pile.finish()
}

fn glass_tray(child: Box<dyn Element>, palette: Palette) -> Box<dyn Element> {
    Container::new(child)
        .with_uniform_padding(6.)
        .with_background(palette.glass)
        .with_border(Border::all(1.).with_border_fill(palette.line))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
        .with_drop_shadow(DropShadow::default())
        .finish()
}

fn render_card<A: Action + Clone>(
    preview: &Preview,
    size: PileSize,
    card_state: &MouseStateHandle,
    close_state: &MouseStateHandle,
    wrap: fn(StageAction) -> A,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let palette = Palette::new(appearance);
    let id = preview.id;
    let label = preview.label();
    let asset = preview.frame_size.map(|_| preview.frame_asset());
    let font_family = appearance.ui_font_family();
    let (width, height) = size.card;
    let card = Hoverable::new(card_state.clone(), move |mouse| {
        let picture = match &asset {
            Some(asset) => frame_image_from(asset.clone(), 1.),
            None => Empty::new().finish(),
        };
        let (border, text) = if mouse.is_hovered() {
            (palette.lit, palette.text)
        } else {
            (palette.line, palette.muted)
        };
        ConstrainedBox::new(
            Container::new(
                Flex::column()
                    .with_spacing(4.)
                    .with_child(ConstrainedBox::new(picture).with_height(height).finish())
                    .with_child(
                        Text::new_inline(label.clone(), font_family, 11.)
                            .with_color(text)
                            .finish(),
                    )
                    .finish(),
            )
            .with_uniform_padding(4.)
            .with_border(Border::all(1.).with_border_fill(border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish(),
        )
        .with_width(width)
        .finish()
    })
    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(wrap(StageAction::BringToFront(id))))
    .finish();

    let mut stack = Stack::new().with_child(card);
    stack.add_positioned_child(
        bar_button(
            Icon::X,
            "Close this preview",
            BarButtonState::Normal,
            close_state,
            Some(wrap(StageAction::Close(id))),
            appearance,
        ),
        OffsetPositioning::offset_from_parent(
            vec2f(-4., 4.),
            ParentOffsetBounds::ParentByPosition,
            ParentAnchor::TopRight,
            ChildAnchor::TopRight,
        ),
    );
    stack.finish()
}
