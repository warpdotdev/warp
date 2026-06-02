use std::fmt::Debug;

use itertools::{Itertools, Position};
use warpui::elements::{
    CrossAxisAlignment, Flex, Hoverable, MainAxisSize, MouseStateHandle, ParentElement, Shrinkable,
};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, EventContext};

use crate::appearance::Appearance;

/// A value which may be rendered as a breadcrumb.
pub trait Breadcrumb: Debug + 'static {
    /// The label to display for this breadcrumb.
    fn label(&self) -> String;

    /// Whether or not this breadcrumb is enabled and interactive.
    fn enabled(&self) -> bool;
}

impl Breadcrumb for String {
    fn label(&self) -> String {
        self.clone()
    }

    fn enabled(&self) -> bool {
        false
    }
}

/// This implementation is for cases where a breadcrumb type is required but unused, such as panes
/// that do not have any breadcrumbs.
impl Breadcrumb for () {
    fn label(&self) -> String {
        String::new()
    }

    fn enabled(&self) -> bool {
        false
    }
}

/// State for a breadcrumb component.
#[derive(Clone)]
pub struct BreadcrumbState<T: Breadcrumb> {
    breadcrumb: T,
    mouse_state_handle: MouseStateHandle,
}

impl<T: Breadcrumb> BreadcrumbState<T> {
    pub fn new(breadcrumb: T) -> Self {
        Self {
            breadcrumb,
            mouse_state_handle: Default::default(),
        }
    }
}

/// Render a single breadcrumb.
fn render_breadcrumb<T: Breadcrumb>(
    state: &BreadcrumbState<T>,
    is_last_item: bool,
    appearance: &Appearance,
) -> Hoverable {
    let suffix = if is_last_item { "" } else { " / " };
    let name = state.breadcrumb.label() + suffix;

    let hoverable = Hoverable::new(state.mouse_state_handle.clone(), |mouse_state| {
        let font_color = if mouse_state.is_hovered() || mouse_state.is_clicked() {
            appearance.theme().active_ui_text_color()
        } else {
            appearance
                .theme()
                .sub_text_color(appearance.theme().background())
        };

        appearance
            .ui_builder()
            .span(name)
            .with_style(UiComponentStyles {
                font_color: Some(font_color.into()),
                ..Default::default()
            })
            .build()
            .finish()
    });
    if state.breadcrumb.enabled() {
        hoverable
    } else {
        hoverable.disable()
    }
}

/// Render a row of interactive breadcrumbs.
pub fn render_breadcrumbs<T, I>(
    breadcrumbs: I,
    appearance: &Appearance,
    on_click: fn(&mut EventContext, &AppContext, &T) -> (),
) -> Box<dyn Element>
where
    T: Breadcrumb,
    I: IntoIterator<Item = BreadcrumbState<T>>,
{
    let children = breadcrumbs
        .into_iter()
        .with_position()
        .map(|(position, breadcrumb)| {
            // Each breadcrumb is expanded so that it inherits the parent `Flex`'s size constraint.
            Shrinkable::new(
                1.,
                render_breadcrumb(
                    &breadcrumb,
                    matches!(position, Position::Last | Position::Only),
                    appearance,
                )
                .on_click(move |ctx, app, _| on_click(ctx, app, &breadcrumb.breadcrumb))
                .finish(),
            )
            .finish()
        });

    Flex::row()
        .with_children(children)
        .with_main_axis_size(MainAxisSize::Min)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .finish()
}
