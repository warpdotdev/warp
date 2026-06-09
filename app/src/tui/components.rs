//! Reusable, TUI-specific UI components, authored in *cell units* (1 cell == 1
//! "pixel" in the TUI backend).
//!
//! Everything here is laid out in cells: a padding of `1.0` is one terminal
//! cell, a border of `1.0` is one cell of box-drawing, and all text is rendered
//! at [`FONT_SIZE`] with a line-height ratio of `1.0` so each line occupies
//! exactly one row. Colors are 24-bit truecolor ([`palette`]).
//!
//! The public surface ([`panel`], [`transcript`], [`input_line`],
//! [`status_bar`], [`header`], [`key_capture`], [`KeyPress`]) is composed by
//! [`crate::tui::agent_view`]. Key capture is handled by a small custom
//! [`Element`] that forwards each keypress to a closure (which dispatches a
//! typed action on the owning view).

use std::borrow::Cow;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use warpui::color::ColorU;
use warpui::elements::{
    Border, Container, CornerRadius, CrossAxisAlignment, Empty, Expanded, Flex, LiveElement,
    MainAxisSize, ParentElement as _, Point, Radius, Text,
};
use warpui::event::DispatchedEvent;
use warpui::fonts::{FamilyId, Properties, Weight};
use warpui::geometry::vector::Vector2F;
use warpui::{
    AfterLayoutContext, AppContext, ClipBounds, Element, Event, EventContext, LayoutContext,
    PaintContext, SizeConstraint,
};

use crate::tui::agent_bridge::{TuiToolStatus, TuiTranscriptEntry};

/// Every glyph advances one cell in the TUI backend regardless of font size, so
/// the value is mostly cosmetic; `1.0` keeps text in the same "1 cell == 1 px"
/// space as the rest of the layout.
const FONT_SIZE: f32 = 1.0;

/// Braille spinner frames, advanced from elapsed wall-clock time.
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// A cohesive modern dark palette (24-bit truecolor).
pub(crate) mod palette {
    use warpui::color::ColorU;

    const fn rgb(r: u8, g: u8, b: u8) -> ColorU {
        ColorU { r, g, b, a: 255 }
    }

    /// Window background.
    pub(crate) const BG: ColorU = rgb(13, 14, 22);
    /// Panel background (slightly lifted from [`BG`]).
    pub(crate) const SURFACE: ColorU = rgb(20, 22, 33);
    /// Chips, header and input background.
    pub(crate) const SURFACE_RAISED: ColorU = rgb(29, 31, 47);
    /// Default border color.
    pub(crate) const BORDER: ColorU = rgb(46, 49, 73);

    /// Primary text.
    pub(crate) const FG: ColorU = rgb(224, 226, 238);
    /// Secondary / dimmed text.
    pub(crate) const FG_DIM: ColorU = rgb(167, 172, 194);
    /// Muted text (notices, hints).
    pub(crate) const MUTED: ColorU = rgb(112, 118, 144);

    /// Brand accent (periwinkle blue) — user prompts, focus.
    pub(crate) const ACCENT: ColorU = rgb(125, 162, 255);
    /// Secondary accent (violet) — agent marker.
    pub(crate) const ACCENT_ALT: ColorU = rgb(167, 139, 250);

    /// Tool succeeded.
    pub(crate) const SUCCESS: ColorU = rgb(74, 222, 128);
    /// In-progress / streaming.
    pub(crate) const WARNING: ColorU = rgb(245, 191, 66);
    /// Tool failed.
    pub(crate) const DANGER: ColorU = rgb(248, 113, 113);
}

/// A monospace text run at one cell per line.
fn text(content: impl Into<Cow<'static, str>>, family: FamilyId, color: ColorU) -> Text {
    Text::new(content, family, FONT_SIZE)
        .with_color(color)
        .with_line_height_ratio(1.0)
}

/// [`text`] with a bold weight (the renderer may map this to ANSI bold).
fn text_bold(content: impl Into<Cow<'static, str>>, family: FamilyId, color: ColorU) -> Text {
    text(content, family, color).with_style(Properties::default().weight(Weight::Bold))
}

/// The current spinner glyph, derived from elapsed time since first use.
fn spinner_frame() -> &'static str {
    static START: OnceLock<Instant> = OnceLock::new();
    let elapsed = START.get_or_init(Instant::now).elapsed().as_millis();
    SPINNER_FRAMES[(elapsed / 80) as usize % SPINNER_FRAMES.len()]
}

/// An animated spinner. Wrapped in a [`LiveElement`] so the window keeps
/// repainting (only while a spinner is actually on screen, e.g. streaming or a
/// running tool call).
fn spinner(family: FamilyId, color: ColorU) -> Box<dyn Element> {
    LiveElement::new(
        text(spinner_frame(), family, color)
            .soft_wrap(false)
            .finish(),
        Duration::from_millis(80),
    )
    .finish()
}

/// A bordered, rounded panel with a muted title and an expanding body. Intended
/// to be placed in a bounded (e.g. [`Expanded`]) slot so the body fills it.
pub(crate) fn panel(title: &str, child: Box<dyn Element>, family: FamilyId) -> Box<dyn Element> {
    let title_row = text(title.to_owned(), family, palette::MUTED)
        .soft_wrap(false)
        .finish();

    let inner = Flex::column()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(1.0)
        .with_child(title_row)
        .with_child(Expanded::new(1.0, child).finish())
        .finish();

    Container::new(inner)
        .with_border(Border::all(1.0).with_border_fill(palette::BORDER))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(1.0)))
        .with_background_color(palette::SURFACE)
        .with_uniform_padding(1.0)
        .finish()
}

/// The conversation transcript: a column of styled entries pinned to the bottom
/// (newest just above the input) and clipped to its slot, so older entries
/// scroll off the top instead of overflowing the panel. Must be placed in a
/// bounded slot.
pub(crate) fn transcript(entries: &[TuiTranscriptEntry], family: FamilyId) -> Box<dyn Element> {
    let mut column = Flex::column()
        .with_main_axis_size(MainAxisSize::Min)
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(1.0);

    if entries.is_empty() {
        column = column.with_child(
            text("No messages yet.", family, palette::MUTED)
                .soft_wrap(false)
                .finish(),
        );
    } else {
        for entry in entries {
            column = column.with_child(entry_view(entry, family));
        }
    }

    bottom_anchored(column.finish())
}

/// Renders a single transcript entry distinctly by kind.
fn entry_view(entry: &TuiTranscriptEntry, family: FamilyId) -> Box<dyn Element> {
    match entry {
        TuiTranscriptEntry::User { text } => {
            prefixed_row("› ", palette::ACCENT, text, palette::FG, family)
        }
        TuiTranscriptEntry::Agent { text } => {
            prefixed_row("◆ ", palette::ACCENT_ALT, text, palette::FG_DIM, family)
        }
        TuiTranscriptEntry::Notice { text } => {
            prefixed_row("· ", palette::MUTED, text, palette::MUTED, family)
        }
        TuiTranscriptEntry::ToolCall {
            title,
            detail,
            status,
        } => tool_call_chip(title, detail, *status, family),
    }
}

/// A marker glyph (fixed width, one line) followed by wrapping body text.
fn prefixed_row(
    marker: &'static str,
    marker_color: ColorU,
    body: &str,
    body_color: ColorU,
    family: FamilyId,
) -> Box<dyn Element> {
    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(
            text_bold(marker, family, marker_color)
                .soft_wrap(false)
                .finish(),
        )
        .with_child(Expanded::new(1.0, text(body.to_owned(), family, body_color).finish()).finish())
        .finish()
}

/// A compact bordered card for a tool/command invocation, with a status glyph.
fn tool_call_chip(
    title: &str,
    detail: &str,
    status: TuiToolStatus,
    family: FamilyId,
) -> Box<dyn Element> {
    let (glyph, accent) = match status {
        TuiToolStatus::Running => (spinner(family, palette::WARNING), palette::WARNING),
        TuiToolStatus::Succeeded => (
            text("✓", family, palette::SUCCESS)
                .soft_wrap(false)
                .finish(),
            palette::SUCCESS,
        ),
        TuiToolStatus::Failed => (
            text("✗", family, palette::DANGER).soft_wrap(false).finish(),
            palette::DANGER,
        ),
    };

    let mut row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(1.0)
        .with_child(glyph)
        .with_child(
            text_bold(title.to_owned(), family, palette::FG)
                .soft_wrap(false)
                .finish(),
        );

    if !detail.is_empty() {
        row = row.with_child(
            Expanded::new(
                1.0,
                text(detail.to_owned(), family, palette::MUTED).finish(),
            )
            .finish(),
        );
    }

    Container::new(row.finish())
        .with_border(Border::all(1.0).with_border_fill(accent))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(1.0)))
        .with_background_color(palette::SURFACE_RAISED)
        .with_horizontal_padding(1.0)
        .finish()
}

/// The single-line input field with a prompt marker and a block caret.
pub(crate) fn input_line(
    buffer: &str,
    caret: usize,
    focused: bool,
    family: FamilyId,
) -> Box<dyn Element> {
    let border = if focused {
        palette::ACCENT
    } else {
        palette::BORDER
    };

    let row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(
            text_bold("❯ ", family, palette::ACCENT)
                .soft_wrap(false)
                .finish(),
        )
        .with_child(Expanded::new(1.0, input_content(buffer, caret, focused, family)).finish())
        .finish();

    Container::new(row)
        .with_border(Border::all(1.0).with_border_fill(border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(1.0)))
        .with_background_color(palette::SURFACE_RAISED)
        .with_horizontal_padding(1.0)
        .finish()
}

/// Splits the buffer at the caret and renders a block caret between the halves.
fn input_content(buffer: &str, caret: usize, focused: bool, family: FamilyId) -> Box<dyn Element> {
    if buffer.is_empty() && !focused {
        return text("Type a message and press Enter…", family, palette::MUTED)
            .soft_wrap(false)
            .finish();
    }

    let chars: Vec<char> = buffer.chars().collect();
    let caret = caret.min(chars.len());
    let before: String = chars[..caret].iter().collect();
    let after: String = chars[caret..].iter().collect();

    let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Start);
    if !before.is_empty() {
        row = row.with_child(text(before, family, palette::FG).soft_wrap(false).finish());
    }
    if focused {
        row = row.with_child(text("▌", family, palette::ACCENT).soft_wrap(false).finish());
    }
    if !after.is_empty() {
        row = row.with_child(text(after, family, palette::FG).soft_wrap(false).finish());
    }
    row.finish()
}

/// The top banner: brand mark + title on the left, status (with spinner while
/// streaming) on the right, separated from the body by a bottom rule.
pub(crate) fn header(
    title: &str,
    subtitle: &str,
    streaming: bool,
    family: FamilyId,
) -> Box<dyn Element> {
    let brand = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(1.0)
        .with_child(text("✦", family, palette::ACCENT).soft_wrap(false).finish())
        .with_child(
            text_bold(title.to_owned(), family, palette::FG)
                .soft_wrap(false)
                .finish(),
        )
        .finish();

    let mut right = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(1.0);
    if streaming {
        right = right.with_child(spinner(family, palette::WARNING));
    }
    right = right.with_child(
        text(subtitle.to_owned(), family, palette::MUTED)
            .soft_wrap(false)
            .finish(),
    );

    let row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(brand)
        .with_child(Expanded::new(1.0, Empty::new().finish()).finish())
        .with_child(right.finish())
        .finish();

    Container::new(row)
        .with_background_color(palette::SURFACE_RAISED)
        .with_horizontal_padding(1.0)
        .with_border(Border::bottom(1.0).with_border_fill(palette::BORDER))
        .finish()
}

/// The slim footer: live status on the left, key hints on the right.
pub(crate) fn status_bar(status: &str, streaming: bool, family: FamilyId) -> Box<dyn Element> {
    let mut left = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(1.0);
    if streaming {
        left = left.with_child(spinner(family, palette::WARNING));
    }
    let status_color = if streaming {
        palette::WARNING
    } else {
        palette::SUCCESS
    };
    left = left.with_child(
        text(status.to_owned(), family, status_color)
            .soft_wrap(false)
            .finish(),
    );

    let row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(left.finish())
        .with_child(Expanded::new(1.0, Empty::new().finish()).finish())
        .with_child(
            text("Enter ↵ send    Esc ⌫ clear", family, palette::MUTED)
                .soft_wrap(false)
                .finish(),
        )
        .finish();

    Container::new(row).with_horizontal_padding(1.0).finish()
}

/// A normalized keypress produced by [`KeyCapture`] and consumed by the view.
#[derive(Debug, Clone)]
pub(crate) enum KeyPress {
    Char(String),
    Enter,
    Backspace,
    Escape,
    Left,
    Right,
    Home,
    End,
}

/// Wraps `child` and forwards each keypress to `on_key`. The child is given a
/// chance to handle the event first.
pub(crate) fn key_capture<F>(child: Box<dyn Element>, on_key: F) -> Box<dyn Element>
where
    F: 'static + FnMut(&mut EventContext, &KeyPress),
{
    KeyCapture {
        child,
        on_key: Box::new(on_key),
    }
    .finish()
}

struct KeyCapture {
    child: Box<dyn Element>,
    on_key: Box<dyn FnMut(&mut EventContext, &KeyPress)>,
}

impl Element for KeyCapture {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.child.paint(origin, ctx, app);
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if self.child.dispatch_event(event, ctx, app) {
            return true;
        }
        let key_press = match event.raw_event() {
            Event::KeyDown {
                keystroke, chars, ..
            } => keydown_to_key(
                &keystroke.key,
                chars,
                keystroke.ctrl,
                keystroke.alt,
                keystroke.cmd,
                keystroke.meta,
            ),
            Event::TypedCharacters { chars } => typed_to_key(chars),
            _ => None,
        };
        match key_press {
            Some(key_press) => {
                (self.on_key)(ctx, &key_press);
                true
            }
            None => false,
        }
    }
}

/// Maps an [`Event::KeyDown`] to a [`KeyPress`]. Handles named special keys and,
/// when no command modifier is held, printable text (preferring the produced
/// `chars`, falling back to the keystroke's key).
fn keydown_to_key(
    key: &str,
    chars: &str,
    ctrl: bool,
    alt: bool,
    cmd: bool,
    meta: bool,
) -> Option<KeyPress> {
    match key.to_ascii_lowercase().as_str() {
        "enter" | "return" | "\r" | "\n" => return Some(KeyPress::Enter),
        "backspace" | "\u{7f}" | "\u{8}" => return Some(KeyPress::Backspace),
        "escape" | "esc" | "\u{1b}" => return Some(KeyPress::Escape),
        "left" => return Some(KeyPress::Left),
        "right" => return Some(KeyPress::Right),
        "home" => return Some(KeyPress::Home),
        "end" => return Some(KeyPress::End),
        "space" => return Some(KeyPress::Char(" ".to_owned())),
        "tab" => return None,
        _ => {}
    }

    // A command-like modifier means this is a shortcut, not text input.
    if ctrl || cmd || alt || meta {
        return None;
    }

    // In the TUI, KeyDown carries no text and `key` is the key's name. Only a
    // single-character name is text; multi-char names ("down", "f1", …) are
    // navigation keys with no textual value and must not be inserted.
    let candidate = if chars.is_empty() { key } else { chars };
    let mut candidate_chars = candidate.chars();
    match (candidate_chars.next(), candidate_chars.next()) {
        (Some(c), None) if !c.is_control() => Some(KeyPress::Char(c.to_string())),
        _ => None,
    }
}

/// Maps an [`Event::TypedCharacters`] to printable text.
fn typed_to_key(chars: &str) -> Option<KeyPress> {
    if !chars.is_empty() && chars.chars().all(|c| !c.is_control()) {
        Some(KeyPress::Char(chars.to_owned()))
    } else {
        None
    }
}

/// Pins `child` to the bottom of its slot and clips the overflow, so the newest
/// content stays visible while older content scrolls off the clipped top.
fn bottom_anchored(child: Box<dyn Element>) -> Box<dyn Element> {
    BottomAnchoredClip {
        child,
        size: None,
        child_size: None,
        origin: None,
    }
    .finish()
}

struct BottomAnchoredClip {
    child: Box<dyn Element>,
    size: Option<Vector2F>,
    child_size: Option<Vector2F>,
    origin: Option<Point>,
}

impl Element for BottomAnchoredClip {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        // Measure the child at its natural height (full slot width) so we know
        // how far it overflows the slot.
        let child_size = self.child.layout(
            SizeConstraint {
                min: Vector2F::new(constraint.max.x(), 0.0),
                max: Vector2F::new(constraint.max.x(), f32::INFINITY),
            },
            ctx,
            app,
        );
        self.child_size = Some(child_size);
        // Occupy the full (bounded) slot height so siblings lay out correctly.
        let height = if constraint.max.y().is_finite() {
            constraint.max.y()
        } else {
            child_size.y()
        };
        let size = Vector2F::new(constraint.max.x(), height);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let size = self.size.expect("laid out before paint");
        let child_size = self.child_size.expect("child laid out before paint");
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

        // Positive when the child fits (push it to the bottom); negative when it
        // overflows (shift it up so its bottom aligns and the top is clipped).
        let dy = size.y() - child_size.y();
        let child_origin = origin + Vector2F::new(0.0, dy);

        let clip_anchor = Point::from_vec2f(origin, ctx.scene.z_index());
        if let Some(bounds) = ctx.scene.visible_rect(clip_anchor, size) {
            ctx.scene.start_layer(ClipBounds::BoundedBy(bounds));
            self.child.paint(child_origin, ctx, app);
            ctx.scene.stop_layer();
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }
}
