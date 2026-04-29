use std::{ops::Range, sync::Arc};

use crate::platform::Cursor;
use crate::{
    elements::{
        AnchorPair, ConstrainedBox, Container, CornerRadius, DragAxis, Draggable, DraggableState,
        DropShadow, Fill, Hoverable, MouseStateHandle, OffsetPositioning, OffsetType,
        ParentElement, ParentOffsetBounds, PositionedElementOffsetBounds, PositioningAxis, Radius,
        Rect, SavePosition, Stack, XAxisAnchor, YAxisAnchor,
    },
    ui_components::components::UiComponentStyles,
    AppContext, Element, EventContext,
};
use lazy_static::lazy_static;
use parking_lot::{Mutex, RwLock};
use pathfinder_color::ColorU;
use pathfinder_geometry::{rect::RectF, vector::vec2f};

use super::components::UiComponent;

const DEFAULT_THUMB_SIZE: f32 = 18.;
const DEFAULT_TRACK_HEIGHT: f32 = 4.;
const HOVER_OPACITY: u8 = 100;
const HOVER_BORDER_SIZE: f32 = 10.;

lazy_static! {
    pub static ref DEFAULT_TRACK_COLOR: ColorU = ColorU::new(170, 170, 170, 255);
    pub static ref DEFAULT_TRACK_FILL: Fill = Fill::Solid(ColorU::new(170, 170, 170, 255));
    pub static ref DEFAULT_THUMB_FILL: Fill = Fill::Solid(ColorU::white());
    static ref THUMB_DROP_SHADOW: DropShadow = DropShadow {
                            color: ColorU::black(),
                            offset: vec2f(-0.5, 2.),
                            blur_radius: 20.,
                            spread_radius: 0.,
                        };

    /// A static counter of the number of instantiated sliders, which is used to create a unique
    /// SavePosition ID to reference the position of the slider track, which is used to position
    /// the slider thumb.
    static ref TRACK_POSITION_ID_COUNT: RwLock<usize> = RwLock::new(0);
}

#[derive(Clone, Copy, Default)]
struct SliderState {
    // The thumb's current offset from the "beginning" (minimum) x-axis coordinate of the track.
    thumb_offset_x: Option<f32>,
}

#[derive(Clone, Default)]
pub struct SliderStateHandle {
    thumb_hoverable_state: MouseStateHandle,
    thumb_draggable_state: DraggableState,
    track_hoverable_state: MouseStateHandle,
    inner: Arc<Mutex<SliderState>>,
}

impl SliderStateHandle {
    // Returns the thumb's current offset from the "beginning" (minimum) x-axis coordinate of the
    // track.
    fn thumb_offset_x(&self) -> Option<f32> {
        self.inner.lock().thumb_offset_x
    }

    // Returns the 'value' represented by the slider's current position along the track. The
    // returned value is normalized to the given value_range.
    fn get_value(&self, draggable_width: f32, value_range: &Range<f32>) -> f32 {
        let state = self.inner.lock();
        let thumb_offset_x = state.thumb_offset_x.unwrap_or(0.);
        let canonical_value = thumb_offset_x / draggable_width;
        canonical_value * (value_range.end - value_range.start) + value_range.start
    }

    /// Sets the inner [`SliderState`] to `new_state`.
    fn store(&self, new_state: SliderState) {
        let mut guard = self.inner.lock();
        *guard = new_state;
    }

    /// Resets the thumb's offset to `None`, which causes the default value to be
    /// used when the slider is next rendered.
    pub fn reset_offset(&self) {
        self.store(SliderState {
            thumb_offset_x: None,
        });
    }
}

/// Type alias for `on_drag` and `on_change` callbacks, either of which is executed when the slider's
/// value has changed.
type OnValueChangedFn = dyn Fn(&mut EventContext, &AppContext, f32) + 'static;

/// Slider UiComponent for modulating a value between given bounds.
///
/// Builder methods allow the caller to configure the styling of the slider, as well as set a
/// callback to be executed when the slider 'thumb' (handle) is dragged, as well as when the thumb
/// is dropped (marking the end of a 'drag').
pub struct Slider {
    state_handle: SliderStateHandle,
    track_position_id: String,
    on_drag_callback: Option<Box<OnValueChangedFn>>,
    on_change_callback: Option<Arc<OnValueChangedFn>>,
    thumb_size: f32,
    track_height: f32,
    track_fill: Fill,
    thumb_fill: Fill,
    styles: UiComponentStyles,
    value_range: Range<f32>,
    default_value: Option<f32>,
}

impl Slider {
    pub fn new(slider_state_handle: SliderStateHandle) -> Self {
        Self {
            track_position_id: new_track_position_id(),
            state_handle: slider_state_handle,
            on_drag_callback: None,
            on_change_callback: None,
            thumb_size: DEFAULT_THUMB_SIZE,
            track_height: DEFAULT_TRACK_HEIGHT,
            track_fill: *DEFAULT_TRACK_FILL,
            thumb_fill: *DEFAULT_THUMB_FILL,
            value_range: 0.0..1.,
            default_value: None,
            styles: UiComponentStyles {
                ..Default::default()
            },
        }
    }

    pub fn with_thumb_size(mut self, thumb_size: f32) -> Self {
        self.thumb_size = thumb_size;
        self
    }

    pub fn with_thumb_fill(mut self, fill: Fill) -> Self {
        self.thumb_fill = fill;
        self
    }

    pub fn with_track_fill(mut self, fill: Fill) -> Self {
        self.track_fill = fill;
        self
    }

    pub fn with_track_height(mut self, height: f32) -> Self {
        self.track_height = height;
        self
    }

    /// Sets the slider's value range. If set, values passed to the `on_change` callback are
    /// normalized to the given range.
    pub fn with_range(mut self, range: Range<f32>) -> Self {
        self.value_range = range;
        self
    }

    pub fn with_default_value(mut self, value: f32) -> Self {
        self.default_value = Some(value);
        self
    }

    /// Called when the value represented by the slider changes when the user drags the slider
    /// thumb.  The emitted value is normalized to the slider's value range, the default for
    /// which is [0, 1].
    pub fn on_drag<F>(mut self, callback: F) -> Self
    where
        F: Fn(&mut EventContext, &AppContext, f32) + 'static,
    {
        self.on_drag_callback = Some(Box::new(callback));
        self
    }

    /// Called when the slider thumb is 'dropped' at the end of a drag. The emitted value is
    /// normalized to the slider's value range, the default for which is [0, 1].
    pub fn on_change<F>(mut self, callback: F) -> Self
    where
        F: Fn(&mut EventContext, &AppContext, f32) + 'static,
    {
        self.on_change_callback = Some(Arc::new(callback));
        self
    }

    /// Registers the 'on_drag_start` callback on the `Draggable` element representing the slider
    /// thumb.
    ///
    /// This callback stores the thumb's x-axis offset from the start of the track in the
    /// given `SliderStateHandle`.
    fn register_on_drag_start_callback(
        thumb_draggable: &mut Draggable,
        track_position_id: String,
        state_handle: SliderStateHandle,
    ) {
        thumb_draggable.set_on_drag_start(move |event_ctx, _app, thumb_position| {
            let track_position = event_ctx
                .element_position_by_id(track_position_id.as_str())
                .expect("Track should be laid out by the time the slider is dragged.");

            // Save the position along the x-axis of the thumb when the drag started.
            state_handle.store(SliderState {
                thumb_offset_x: Some(thumb_position.origin_x() - track_position.origin_x()),
            });
        });
    }

    /// Registers the 'on_drag` callback on the `Draggable` element representing the slider thumb.
    ///
    /// The registered callback calls the user's supplied `on_drag` callback if the slider's x-axis
    /// position has changed since the last time it was called. The user's callback is called with
    /// the slider's current value, which is basically the slider thumb's offset x normalized to
    /// the slider's `value_range`. In addition, it updates the `thumb_offset_x` in the slider's
    /// state.
    fn register_on_drag_callback(
        thumb_draggable: &mut Draggable,
        track_position_id: String,
        thumb_size: f32,
        value_range: Range<f32>,
        state_handle: SliderStateHandle,
        on_drag_callback: Option<Box<OnValueChangedFn>>,
    ) {
        thumb_draggable.set_on_drag(move |event_ctx, app, thumb_position, _| {
            let track_position = event_ctx
                .element_position_by_id(track_position_id.as_str())
                .expect("Track should be laid out by the time the slider is dragged.");

            let current_thumb_offset_x = thumb_position.origin_x() - track_position.origin_x();

            // The on_drag callback is called even if the draggable element's position
            // hasn't changed -- only call the on_change callback if the slider's
            // position has changed.
            if Some(current_thumb_offset_x) != state_handle.thumb_offset_x() {
                state_handle.store(SliderState {
                    thumb_offset_x: Some(current_thumb_offset_x),
                });

                if let Some(callback) = &on_drag_callback {
                    let draggable_width = draggable_width(track_position, thumb_size);
                    let updated_value = state_handle.get_value(draggable_width, &value_range);
                    callback(event_ctx, app, updated_value);
                }
            }
        });
    }

    /// Registers the 'on_change` callback on the `Draggable` element representing the slider thumb.
    ///
    /// The registered callback unconditinoally calls the user's supplied `on_change` callback.  The
    /// user's callback is called with the slider's current value, which is basically the slider
    /// thumb's offset x normalized to the slider's `value_range`. In addition, it updates the
    /// `thumb_offset_x` in the slider's state.
    fn register_on_drop_callback(
        thumb_draggable: &mut Draggable,
        track_position_id: String,
        thumb_size: f32,
        value_range: Range<f32>,
        state_handle: SliderStateHandle,
        on_change_callback: Option<Arc<OnValueChangedFn>>,
    ) {
        thumb_draggable.set_on_drop(move |event_ctx, app, thumb_position, _| {
            let track_position = event_ctx
                .element_position_by_id(track_position_id.as_str())
                .expect("Track should be laid out by the time the slider is dropped.");
            state_handle.store(SliderState {
                thumb_offset_x: Some(thumb_position.origin_x() - track_position.origin_x()),
            });

            if let Some(callback) = &on_change_callback {
                let draggable_width = draggable_width(track_position, thumb_size);
                let updated_value = state_handle.get_value(draggable_width, &value_range);
                callback(event_ctx, app, updated_value);
            }
        });
    }

    /// Registers the 'on_change_callback` callback on the `Hoverable` element representing the slider track.
    ///
    /// Whenever the underlying track is clicked, we set the thumb offset to the location of the click,
    /// and then call the on_change_callback with the updated value. Basically works as if a user immediately
    /// dragged the thumb to that location, without all the intermediate on_drag calls.
    fn register_on_click_callback(
        track_hoverable: Hoverable,
        track_position_id: String,
        thumb_size: f32,
        value_range: Range<f32>,
        state_handle: SliderStateHandle,
        on_change_callback: Option<Arc<OnValueChangedFn>>,
    ) -> Hoverable {
        track_hoverable.on_click(move |event_ctx, app, click_position| {
            let Some(track_position) = event_ctx.element_position_by_id(track_position_id.as_str())
            else {
                return;
            };

            let click_position_x = click_position.x();
            let padding = thumb_size / 2.;
            let min_x = track_position.min_x() + padding;
            let max_x = track_position.max_x() - padding;

            // If the user clicks outside of the actual visible portion of the track,
            // we do not proceed.
            if min_x > click_position_x || max_x < click_position_x {
                return;
            }

            state_handle.store(SliderState {
                thumb_offset_x: Some(click_position_x - min_x),
            });

            if let Some(callback) = &on_change_callback {
                let draggable_width = draggable_width(track_position, thumb_size);
                let updated_value = state_handle.get_value(draggable_width, &value_range);
                callback(event_ctx, app, updated_value);
            }
        })
    }
}

impl UiComponent for Slider {
    type ElementType = Container;

    fn build(self) -> Self::ElementType {
        let Slider {
            state_handle,
            track_position_id: slider_track_position_id,
            on_drag_callback,
            on_change_callback,
            thumb_size,
            track_height,
            track_fill,
            thumb_fill,
            styles,
            value_range,
            default_value,
        } = self;

        let track_position_id = slider_track_position_id.clone();
        let mut slider_thumb = Draggable::new(
            state_handle.thumb_draggable_state.clone(),
            render_thumb(
                thumb_fill,
                thumb_size,
                state_handle.thumb_hoverable_state.clone(),
            ),
        )
        .with_drag_axis(DragAxis::HorizontalOnly)
        .with_drag_bounds_callback(move |position_cache, _| {
            position_cache
                .get_position(track_position_id.as_str())
                .map(|track_position| {
                    // Set drag bounds so the thumb may only be dragged along the track.
                    RectF::new(
                        vec2f(track_position.origin_x(), track_position.origin_y()),
                        vec2f(track_position.width(), 0.),
                    )
                })
        });

        Self::register_on_drag_start_callback(
            &mut slider_thumb,
            slider_track_position_id.clone(),
            state_handle.clone(),
        );
        Self::register_on_drag_callback(
            &mut slider_thumb,
            slider_track_position_id.clone(),
            thumb_size,
            value_range.clone(),
            state_handle.clone(),
            on_drag_callback,
        );
        Self::register_on_drop_callback(
            &mut slider_thumb,
            slider_track_position_id.clone(),
            thumb_size,
            value_range.clone(),
            state_handle.clone(),
            on_change_callback.clone(),
        );

        let track = Hoverable::new(state_handle.track_hoverable_state.clone(), |_| {
            render_track(thumb_size, styles.width, track_height, track_fill)
        });

        let track = Self::register_on_click_callback(
            track,
            slider_track_position_id.clone(),
            thumb_size,
            value_range.clone(),
            state_handle.clone(),
            on_change_callback.clone(),
        );

        let mut slider = Stack::new();

        slider.add_child(
            SavePosition::new(track.finish(), slider_track_position_id.as_str()).finish(),
        );

        let offset = match state_handle.thumb_offset_x() {
            Some(offset_x) => OffsetType::Pixel(offset_x),
            None => OffsetType::Percentage(
                default_value
                    .map(|value| {
                        ((value - value_range.start) / (value_range.end - value_range.start))
                            .clamp(0., 1.)
                    })
                    .unwrap_or(0.),
            ),
        };

        slider.add_positioned_child(
            slider_thumb.finish(),
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    &slider_track_position_id,
                    PositionedElementOffsetBounds::AnchoredElement,
                    // Set the position of the thumb based on the slider's current value.
                    offset,
                    AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                ),
                PositioningAxis::relative_to_stack_child(
                    &slider_track_position_id,
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(YAxisAnchor::Middle, YAxisAnchor::Middle),
                ),
            ),
        );
        Container::new(slider.finish())
            .with_margin_top(styles.margin.map(|margin| margin.top).unwrap_or(0.))
            .with_margin_bottom(styles.margin.map(|margin| margin.bottom).unwrap_or(0.))
            .with_margin_left(styles.margin.map(|margin| margin.left).unwrap_or(0.))
            .with_margin_right(styles.margin.map(|margin| margin.right).unwrap_or(0.))
            .with_padding_top(styles.padding.map(|padding| padding.top).unwrap_or(0.))
            .with_padding_bottom(styles.padding.map(|padding| padding.bottom).unwrap_or(0.))
            .with_padding_left(styles.padding.map(|padding| padding.left).unwrap_or(0.))
            .with_padding_right(styles.padding.map(|padding| padding.right).unwrap_or(0.))
    }

    fn with_style(self, styles: UiComponentStyles) -> Self {
        Self {
            state_handle: self.state_handle,
            track_position_id: self.track_position_id,
            on_drag_callback: self.on_drag_callback,
            on_change_callback: self.on_change_callback,
            thumb_size: self.thumb_size,
            track_height: self.track_height,
            track_fill: self.track_fill,
            thumb_fill: self.thumb_fill,
            value_range: self.value_range,
            default_value: self.default_value,
            styles: self.styles.merge(styles),
        }
    }
}

/// Renders the slider 'track', along which the thumb can be dragged.
fn render_track(thumb_size: f32, width: Option<f32>, height: f32, fill: Fill) -> Box<dyn Element> {
    let mut track = ConstrainedBox::new(
        Container::new(
            Rect::new()
                .with_background(fill)
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                .finish(),
        )
        .with_padding_left(thumb_size / 2.)
        .with_padding_right(thumb_size / 2.)
        .finish(),
    )
    .with_height(height);
    if let Some(width) = width {
        track = track.with_width(width);
    }

    // We add a container with extra padding to make the track
    // as tall (invisibly) as the thumb. This way, we can detect
    // clicks that are slightly above or below the track bar itself.
    let vertical_padding = ((thumb_size - height) / 2.).max(0.);
    Container::new(track.finish())
        .with_padding_top(vertical_padding)
        .with_padding_bottom(vertical_padding)
        .finish()
}

/// Renders the 'thumb' (handle) for the slider.
///
/// The thumb is a circle with diameter set to `size`.
fn render_thumb(fill: Fill, size: f32, state_handle: MouseStateHandle) -> Box<dyn Element> {
    Hoverable::new(state_handle, move |hover_state| {
        let thumb = Container::new(
            ConstrainedBox::new(
                Rect::new()
                    .with_background(fill)
                    .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                    .with_drop_shadow(*THUMB_DROP_SHADOW)
                    .finish(),
            )
            .with_width(size)
            .with_height(size)
            .finish(),
        )
        .finish();
        let mut stack = Stack::new();

        if hover_state.is_hovered() {
            let hover_size = size + HOVER_BORDER_SIZE;
            let mut hover_background = *DEFAULT_TRACK_COLOR;
            hover_background.a = HOVER_OPACITY;

            let thumb_hover = Container::new(
                ConstrainedBox::new(
                    Rect::new()
                        .with_background_color(hover_background)
                        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                        .finish(),
                )
                .with_width(hover_size)
                .with_height(hover_size)
                .finish(),
            )
            .finish();

            // Position the hover so that it's centered around the thumb. Since the hover
            // is guaranteed to be larger than the thumb, we position the hover at the top
            // left corner of the thumb and then translate it to the left and up so that it
            // is centered.
            stack.add_positioned_child(
                thumb_hover,
                OffsetPositioning::from_axes(
                    PositioningAxis::relative_to_parent(
                        ParentOffsetBounds::Unbounded,
                        OffsetType::Pixel(-((hover_size - size) / 2.)),
                        AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                    ),
                    PositioningAxis::relative_to_parent(
                        ParentOffsetBounds::Unbounded,
                        OffsetType::Pixel(-((hover_size - size) / 2.)),
                        AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
                    ),
                ),
            );
        }

        stack.add_child(thumb);
        stack.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .finish()
}

/// Returns a unique position ID for the slider track.
fn new_track_position_id() -> String {
    let current_count = *TRACK_POSITION_ID_COUNT.read();
    let position_id = format!("SliderTrack{current_count}");
    *TRACK_POSITION_ID_COUNT.write() = current_count + 1;
    position_id
}

/// Returns total width of the draggable area on the 'track'.
fn draggable_width(track_position: RectF, thumb_size: f32) -> f32 {
    track_position.max_x() - track_position.min_x() - thumb_size
}
