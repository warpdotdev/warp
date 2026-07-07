// Specific slide implementations
pub mod cta_button;
pub mod oz_launch;

use std::collections::HashMap;

use markdown_parser::{parse_markdown, FormattedText, FormattedTextLine};
// Re-export slide types for convenience
pub use oz_launch::OzLaunchSlide;
use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{
    Align, Border, CacheOption, ChildAnchor, Clipped, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, DropShadow, Empty, Expanded, Flex, FormattedTextElement,
    HighlightedHyperlink, Hoverable, HyperlinkLens, Image, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius,
    Shrinkable, SizeConstraintCondition, SizeConstraintSwitch, Stack,
};
use warpui::fonts::Weight;
use warpui::keymap::FixedBinding;
use warpui::platform::Cursor;
use warpui::presenter::ChildView;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::settings::PrivacySettings;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, PrimaryTheme, SecondaryTheme};
use crate::workspace::view::launch_modal::cta_button::{CTAButton, CTAButtonAction};

pub fn init<S: Slide>(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new("escape", LaunchModalAction::<S>::Close, id!("LaunchModal")),
        FixedBinding::new(
            "enter",
            LaunchModalAction::<S>::NextSlide,
            id!("LaunchModal"),
        ),
        FixedBinding::new(
            "left",
            LaunchModalAction::<S>::PrevSlide,
            id!("LaunchModal"),
        ),
        FixedBinding::new(
            "right",
            LaunchModalAction::<S>::NextSlide,
            id!("LaunchModal"),
        ),
        FixedBinding::new("up", LaunchModalAction::<S>::PrevSlide, id!("LaunchModal")),
        FixedBinding::new(
            "down",
            LaunchModalAction::<S>::NextSlide,
            id!("LaunchModal"),
        ),
    ]);
}

/// Configuration for an optional checkbox displayed in the modal's control panel.
pub struct CheckboxConfig {
    pub label: &'static str,
    pub description: &'static str,
}

pub trait Slide:
    'static + Send + Sync + std::fmt::Debug + PartialEq + Eq + std::hash::Hash + Copy + Clone
where
    Self: Sized,
{
    fn modal_title(&self) -> String;
    fn modal_subtext_paragraphs(&self) -> Vec<FormattedTextLine>;
    fn first() -> Self;
    fn next(&self) -> Option<Self>;
    fn prev(&self) -> Option<Self>;
    fn display_text(&self) -> Option<&'static str>;
    fn short_label(&self) -> &'static str;
    fn title(&self) -> &'static str;
    fn title_icon(&self) -> Option<Icon>;
    fn content(&self) -> &'static str;
    fn image(&self) -> AssetSource;
    fn all() -> Vec<Self>;
    fn cta_button(&self) -> CTAButton<Self>;

    /// Returns an optional secondary CTA button for the modal.
    /// When Some, a secondary button is rendered alongside the primary CTA.
    fn secondary_cta_button(&self) -> Option<CTAButton<Self>> {
        None
    }

    /// Returns an optional checkbox configuration for the modal.
    /// When Some, a checkbox is rendered at the bottom of the control panel.
    fn checkbox_config(&self) -> Option<CheckboxConfig> {
        None
    }

    /// Returns whether the checkbox should be shown.
    /// This is checked in addition to checkbox_config() returning Some.
    fn should_show_checkbox(&self, _app: &AppContext) -> bool {
        false
    }

    /// Called when the modal is closed via the X button or esc or close CTA.
    /// Not called if closed via another CTA.
    fn on_close(&self, _ctx: &mut ViewContext<LaunchModal<Self>>) {}
}

pub struct StateHandles<S: Slide> {
    pub close_button: MouseStateHandle,
    pub slides: HashMap<S, SlideStateHandles>,
    pub checkbox: MouseStateHandle,
}

#[derive(Default)]
pub struct SlideStateHandles {
    mouse: MouseStateHandle,
    content_hyperlink: HighlightedHyperlink,
}

impl<S: Slide> Default for StateHandles<S> {
    fn default() -> Self {
        let mut slide_handles = HashMap::new();
        for slide in S::all() {
            slide_handles.insert(slide, SlideStateHandles::default());
        }
        StateHandles {
            close_button: Default::default(),
            slides: slide_handles,
            checkbox: Default::default(),
        }
    }
}

pub struct LaunchModal<S: Slide> {
    slide: S,
    next_button: ViewHandle<ActionButton>,
    secondary_button: ViewHandle<ActionButton>,
    state_handles: StateHandles<S>,
}

impl<S: Slide> LaunchModal<S> {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let next_button = ctx.add_view(|_| ActionButton::new("", PrimaryTheme));
        let secondary_button = ctx.add_view(|_| ActionButton::new("", SecondaryTheme));

        let mut me = LaunchModal {
            slide: S::first(),
            next_button,
            secondary_button,
            state_handles: Default::default(),
        };
        me.update_buttons_based_on_slide(ctx);
        me
    }

    fn update_buttons_based_on_slide(&mut self, ctx: &mut ViewContext<Self>) {
        self.next_button
            .update(ctx, |next_button, ctx| match self.slide.cta_button() {
                CTAButton {
                    label,
                    action: CTAButtonAction::NextSlide(next),
                    ..
                } => {
                    next_button.set_label(label, ctx);
                    next_button.set_on_click(
                        move |ctx| ctx.dispatch_typed_action(LaunchModalAction::SelectSlide(next)),
                        ctx,
                    );
                }
                CTAButton { label, .. } => {
                    next_button.set_label(label, ctx);
                    next_button.set_on_click(
                        move |ctx| ctx.dispatch_typed_action(LaunchModalAction::<S>::Finish),
                        ctx,
                    );
                }
            });

        // Update secondary button if present.
        if let Some(secondary_cta) = self.slide.secondary_cta_button() {
            self.secondary_button
                .update(ctx, |secondary_button, ctx| match secondary_cta {
                    CTAButton {
                        label,
                        action: CTAButtonAction::NextSlide(next),
                        ..
                    } => {
                        secondary_button.set_label(label, ctx);
                        secondary_button.set_on_click(
                            move |ctx| {
                                ctx.dispatch_typed_action(LaunchModalAction::SelectSlide(next))
                            },
                            ctx,
                        );
                    }
                    CTAButton { label, .. } => {
                        secondary_button.set_label(label, ctx);
                        secondary_button.set_on_click(
                            move |ctx| {
                                ctx.dispatch_typed_action(LaunchModalAction::<S>::FinishSecondary)
                            },
                            ctx,
                        );
                    }
                });
        }

        ctx.notify();
    }

    fn render_checkbox(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if !self.slide.should_show_checkbox(app) {
            return None;
        }
        let checkbox_config = self.slide.checkbox_config()?;
        let appearance = Appearance::handle(app).as_ref(app);
        let theme = appearance.theme();

        let is_checked = PrivacySettings::handle(app)
            .as_ref(app)
            .is_cloud_conversation_storage_enabled;

        let checkbox = appearance
            .ui_builder()
            .checkbox(self.state_handles.checkbox.clone(), Some(10.5))
            .check(is_checked)
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(LaunchModalAction::<S>::ToggleCheckbox))
            .finish();

        let label =
            FormattedTextElement::from_str(checkbox_config.label, appearance.ui_font_family(), 12.)
                .with_color(blended_colors::text_sub(
                    theme,
                    blended_colors::neutral_1(theme),
                ))
                .finish();

        let description = FormattedTextElement::from_str(
            checkbox_config.description,
            appearance.ui_font_family(),
            12.,
        )
        .with_color(blended_colors::text_disabled(
            theme,
            blended_colors::neutral_1(theme),
        ))
        .finish();

        Some(
            Container::new(
                Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_child(
                        Flex::row()
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(checkbox)
                            .with_child(Container::new(label).with_margin_left(4.).finish())
                            .finish(),
                    )
                    .with_child(Container::new(description).with_margin_top(4.).finish())
                    .finish(),
            )
            .with_margin_top(24.)
            .finish(),
        )
    }

    fn render_slide_controls(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // Only show slide controls if there are multiple slides or if slides have display text
        let slides_with_display_text: Vec<_> = S::all()
            .into_iter()
            .filter_map(|slide| slide.display_text().map(|text| (slide, text)))
            .collect();

        if slides_with_display_text.len() <= 1 {
            // For single-slide modals or slides without display text, return empty container
            return Container::new(Flex::column().finish()).finish();
        }

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        for (i, (slide, display_text)) in slides_with_display_text.into_iter().enumerate() {
            let mut label =
                FormattedTextElement::from_str(display_text, appearance.ui_font_family(), 14.)
                    .with_color(blended_colors::text_main(
                        theme,
                        blended_colors::neutral_1(theme),
                    ));
            if slide == self.slide {
                label = label.with_weight(Weight::Bold);
            }

            let mut container = Container::new(Align::new(label.finish()).left().finish())
                .with_horizontal_padding(12.)
                .with_vertical_padding(8.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_background(blended_colors::neutral_1(theme));

            if slide == self.slide {
                container = container.with_background(blended_colors::fg_overlay_3(theme))
            }

            if i < S::all().len() {
                container = container.with_margin_bottom(8.)
            }

            column.add_child(if slide == self.slide {
                container.finish()
            } else {
                Hoverable::new(
                    self.state_handles.slides[&slide].mouse.clone(),
                    move |state| {
                        if state.is_hovered() {
                            container
                                .with_background(blended_colors::fg_overlay_3(theme))
                                .finish()
                        } else {
                            container.finish()
                        }
                    },
                )
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(LaunchModalAction::SelectSlide(slide))
                })
                .finish()
            });
        }

        column.finish()
    }

    fn render_current_slide(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let text_container = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(
                Shrinkable::new(
                    1.,
                    Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_child(
                            Container::new({
                                let text = FormattedTextElement::from_str(
                                    self.slide.title(),
                                    appearance.ui_font_family(),
                                    16.,
                                )
                                .with_color(blended_colors::text_main(
                                    theme,
                                    blended_colors::neutral_2(theme),
                                ))
                                .with_weight(Weight::Bold)
                                .finish();
                                if let Some(icon) = self.slide.title_icon() {
                                    Flex::row()
                                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                        .with_child(text)
                                        .with_child(
                                            Container::new(
                                                ConstrainedBox::new(
                                                    icon.to_warpui_icon(Fill::Solid(
                                                        blended_colors::text_main(
                                                            theme,
                                                            blended_colors::neutral_2(theme),
                                                        ),
                                                    ))
                                                    .finish(),
                                                )
                                                .with_width(16.)
                                                .with_height(16.)
                                                .finish(),
                                            )
                                            .with_margin_left(6.)
                                            // Agent icon's bounding box makes the icon look too
                                            // high relative to the text.
                                            .with_margin_top(-2.)
                                            .finish(),
                                        )
                                        .finish()
                                } else {
                                    text
                                }
                            })
                            .with_margin_bottom(8.)
                            .finish(),
                        )
                        .with_child(
                            Container::new(
                                Shrinkable::new(
                                    1.,
                                    FormattedTextElement::new(
                                        parse_markdown(self.slide.content()).unwrap(),
                                        14.,
                                        appearance.ui_font_family(),
                                        appearance.ui_font_family(),
                                        blended_colors::text_sub(
                                            theme,
                                            blended_colors::neutral_4(theme),
                                        ),
                                        self.state_handles.slides[&self.slide]
                                            .content_hyperlink
                                            .clone(),
                                    )
                                    .with_hyperlink_font_color(theme.accent().into_solid())
                                    .register_default_click_handlers_with_action_support(
                                        |hyperlink_lens, _event, ctx| {
                                            if let HyperlinkLens::Url(url) = hyperlink_lens {
                                                ctx.open_url(url);
                                            }
                                        },
                                    )
                                    .finish(),
                                )
                                .finish(),
                            )
                            .with_margin_bottom(8.)
                            .finish(),
                        )
                        .finish(),
                )
                .finish(),
            )
            .with_child(
                Align::new(
                    Flex::row()
                        .with_main_axis_alignment(MainAxisAlignment::End)
                        .with_children(self.slide.secondary_cta_button().map(|_| {
                            Container::new(ChildView::new(&self.secondary_button).finish())
                                .with_margin_right(8.)
                                .finish()
                        }))
                        .with_child(ChildView::new(&self.next_button).finish())
                        .finish(),
                )
                .bottom_right()
                .finish(),
            )
            .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(
                Clipped::new(
                    ConstrainedBox::new(
                        Image::new(self.slide.image(), CacheOption::Original)
                            .with_corner_radius(CornerRadius::with_top_right(Radius::Pixels(10.)))
                            .cover()
                            .finish(),
                    )
                    .with_max_width(MAX_SLIDE_WIDTH)
                    .with_min_height(100.)
                    .with_max_height(MAX_IMAGE_HEIGHT)
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    Container::new(text_container)
                        .with_uniform_padding(24.)
                        .with_background(blended_colors::neutral_2(theme))
                        .with_border(
                            Border::left(1.).with_border_color(blended_colors::neutral_4(theme)),
                        )
                        .with_corner_radius(CornerRadius::with_bottom_right(Radius::Pixels(10.)))
                        .finish(),
                )
                .finish(),
            )
            .finish()
    }

    fn handle_cta_button_action(&self, ctx: &mut ViewContext<Self>) {
        let cta_button = self.slide.cta_button();
        match cta_button.action {
            CTAButtonAction::NextSlide(_) => {}
            CTAButtonAction::Close => {
                self.slide.on_close(ctx);
                ctx.emit(LaunchModalEvent::Close);
            }
            CTAButtonAction::OpenUrl(url) => {
                ctx.open_url(&url);
                ctx.emit(LaunchModalEvent::Close);
            }
            CTAButtonAction::Custom(callback) => {
                callback(ctx);
            }
        }
    }

    fn handle_secondary_cta_button_action(&self, ctx: &mut ViewContext<Self>) {
        let Some(cta_button) = self.slide.secondary_cta_button() else {
            return;
        };
        match cta_button.action {
            CTAButtonAction::NextSlide(_) => {}
            CTAButtonAction::Close => {
                self.slide.on_close(ctx);
                ctx.emit(LaunchModalEvent::Close);
            }
            CTAButtonAction::OpenUrl(url) => {
                ctx.open_url(&url);
                ctx.emit(LaunchModalEvent::Close);
            }
            CTAButtonAction::Custom(callback) => {
                callback(ctx);
            }
        }
    }
}

impl<S: Slide> Entity for LaunchModal<S> {
    type Event = LaunchModalEvent;
}

// Modal dimension constants.
const MAX_MODAL_WIDTH: f32 = 876.;
const MIN_MODAL_HEIGHT: f32 = 300.;
const MAX_MODAL_HEIGHT: f32 = 540.;
const MAX_CONTROL_PANEL_WIDTH: f32 = 333.;
const MIN_CONTROL_PANEL_WIDTH: f32 = 220.;
const MAX_SLIDE_WIDTH: f32 = 543.;
const MAX_IMAGE_HEIGHT: f32 = 355.;
/// Minimum width below which the modal is hidden.
const MIN_MODAL_WIDTH: f32 = 600.;

impl<S: Slide> View for LaunchModal<S> {
    fn ui_name() -> &'static str {
        "LaunchModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        const BUTTON_DIAMETER: f32 = 20.;

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let control_panel = Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(
                    Container::new(
                        FormattedTextElement::from_str(
                            self.slide.modal_title(),
                            appearance.ui_font_family(),
                            24.,
                        )
                        .with_color(blended_colors::text_main(
                            theme,
                            blended_colors::neutral_1(theme),
                        ))
                        .with_weight(Weight::Bold)
                        .finish(),
                    )
                    .with_margin_bottom(12.)
                    .finish(),
                )
                .with_children(
                    self.slide
                        .modal_subtext_paragraphs()
                        .iter()
                        .enumerate()
                        .map(|(index, line)| {
                            let is_last = index == self.slide.modal_subtext_paragraphs().len() - 1;

                            let text_element = FormattedTextElement::new(
                                FormattedText::new([line.clone()]),
                                14.,
                                appearance.ui_font_family(),
                                appearance.ui_font_family(),
                                blended_colors::text_main(theme, blended_colors::neutral_1(theme)),
                                Default::default(), // no hyperlink highlighting needed
                            )
                            .disable_mouse_interaction()
                            .finish();

                            Container::new(text_element)
                                .with_margin_bottom(if is_last { 40. } else { 8. })
                                .finish()
                        }),
                )
                .with_child(Expanded::new(1., self.render_slide_controls(app)).finish())
                .with_children(self.render_checkbox(app))
                .finish(),
        )
        .with_background_color(blended_colors::neutral_1(theme))
        .with_corner_radius(CornerRadius::with_left(Radius::Pixels(10.)))
        .with_uniform_padding(24.)
        .finish();

        let close_button = appearance
            .ui_builder()
            .close_button(BUTTON_DIAMETER, self.state_handles.close_button.clone())
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(LaunchModalAction::<S>::Close))
            .finish();

        let mut modal = Stack::new();
        modal.add_child(
            Container::new(
                ConstrainedBox::new(
                    Flex::row()
                        .with_child(
                            Shrinkable::new(
                                MAX_CONTROL_PANEL_WIDTH,
                                ConstrainedBox::new(control_panel)
                                    .with_min_width(MIN_CONTROL_PANEL_WIDTH)
                                    .with_max_width(MAX_CONTROL_PANEL_WIDTH)
                                    .with_height(MAX_MODAL_HEIGHT)
                                    .finish(),
                            )
                            .finish(),
                        )
                        .with_child(
                            Shrinkable::new(
                                MAX_SLIDE_WIDTH,
                                ConstrainedBox::new(self.render_current_slide(app))
                                    .with_max_width(MAX_SLIDE_WIDTH)
                                    .with_height(MAX_MODAL_HEIGHT)
                                    .finish(),
                            )
                            .finish(),
                        )
                        .finish(),
                )
                .with_max_width(MAX_MODAL_WIDTH)
                .with_min_height(MIN_MODAL_HEIGHT)
                .finish(),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
            .with_border(Border::all(1.).with_border_fill(theme.outline()))
            .with_drop_shadow(DropShadow::default())
            .finish(),
        );
        modal.add_positioned_child(
            close_button,
            OffsetPositioning::offset_from_parent(
                pathfinder_geometry::vector::vec2f(-8., 8.),
                ParentOffsetBounds::ParentByPosition,
                ParentAnchor::TopRight,
                ChildAnchor::TopRight,
            ),
        );

        // Stack needed so that modal can get bounds information,
        // specifically to ensure no overlap with the window's traffic lights.
        let mut stack = Stack::new();
        stack.add_positioned_child(
            modal.finish(),
            OffsetPositioning::offset_from_parent(
                pathfinder_geometry::vector::vec2f(0., 0.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::Center,
                ChildAnchor::Center,
            ),
        );

        // Hide the modal if the window is too narrow to display it properly.
        SizeConstraintSwitch::new(
            Container::new(Align::new(stack.finish()).finish())
                .with_background(Fill::Solid(ColorU::new(97, 97, 97, 255)).with_opacity(50))
                .finish(),
            [(
                SizeConstraintCondition::WidthLessThan(MIN_MODAL_WIDTH),
                Empty::new().finish(),
            )],
        )
        .finish()
    }
}

impl<S: Slide> TypedActionView for LaunchModal<S> {
    type Action = LaunchModalAction<S>;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            LaunchModalAction::SelectSlide(slide) => {
                self.slide = *slide;
                self.update_buttons_based_on_slide(ctx);
                ctx.notify();
            }
            LaunchModalAction::NextSlide => {
                if let Some(next_slide) = self.slide.next() {
                    self.slide = next_slide;
                    self.update_buttons_based_on_slide(ctx);
                    ctx.notify();
                } else {
                    // If we're on the last slide, trigger the CTA button action.
                    self.handle_cta_button_action(ctx);
                }
            }
            LaunchModalAction::PrevSlide => {
                if let Some(prev_slide) = self.slide.prev() {
                    self.slide = prev_slide;
                    self.update_buttons_based_on_slide(ctx);
                    ctx.notify();
                }
                // If we're on the first slide, do nothing.
            }
            LaunchModalAction::Close => {
                self.slide.on_close(ctx);
                ctx.emit(LaunchModalEvent::Close);
            }
            LaunchModalAction::Finish => {
                self.handle_cta_button_action(ctx);
            }
            LaunchModalAction::FinishSecondary => {
                self.handle_secondary_cta_button_action(ctx);
            }
            LaunchModalAction::ToggleCheckbox => {
                ctx.emit(LaunchModalEvent::ToggleCheckbox);
            }
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum LaunchModalEvent {
    Close,
    ToggleCheckbox,
}

#[derive(Copy, Clone, Debug)]
pub enum LaunchModalAction<S: Slide> {
    SelectSlide(S),
    NextSlide,
    PrevSlide,
    Close,
    Finish,
    FinishSecondary,
    ToggleCheckbox,
}
