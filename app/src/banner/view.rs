use std::marker::PhantomData;
use std::rc::Rc;

use markdown_parser::{
    FormattedText, FormattedTextFragment, FormattedTextInline, FormattedTextLine,
};
use pathfinder_geometry::vector::Vector2F;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Flex, FormattedTextElement,
    HighlightedHyperlink, HyperlinkLens, HyperlinkUrl, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, ParentElement, Shrinkable,
};
use warpui::fonts::Weight;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    Action, AppContext, Element, Entity, EventContext, SingletonEntity, TypedActionView, View,
    ViewContext,
};

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;

const CLOSE_BUTTON_DIAMETER: f32 = 20.;
const INNER_MARGIN: f32 = 12.;
const HORIZ_PADDING: f32 = 16.;
const VERT_PADDING: f32 = 8.;

#[derive(Clone, Copy, Debug)]
pub enum DismissalType {
    /// The banner may be re-shown to the user multiple times in new or existing sessions.
    Temporary,

    /// The banner should not be shown again to the user, whether in a new or existing session.
    /// Dismissal state should also persist across app sessions (e.g. when Warp is restarted).
    Permanent,
}

pub enum BannerEvent<T> {
    Dismiss(DismissalType),
    Action(T),
}

pub struct BannerTextContent<T: Action + Clone> {
    /// The text (with optional formatting and links) to be displayed as the
    /// primary banner content.
    text: FormattedTextInline,
    /// Information about which link in the text is under the cursor, if any.
    highlighted_link: HighlightedHyperlink,
    phantom_data: PhantomData<T>,
}

impl<T: Action + Clone> BannerTextContent<T> {
    #[allow(dead_code)]
    pub fn plain_text(text: impl Into<String>) -> Self {
        Self {
            text: vec![FormattedTextFragment::plain_text(text)],
            highlighted_link: Default::default(),
            phantom_data: PhantomData,
        }
    }

    pub fn formatted_text(text: FormattedTextInline) -> Self {
        Self {
            text,
            highlighted_link: Default::default(),
            phantom_data: PhantomData,
        }
    }

    fn render(&self, appearance: &Appearance) -> Box<dyn Element> {
        let font_size = font_size(appearance);
        let font_family = appearance.ui_font_family();
        let code_font_family = appearance.monospace_font_family();
        let font_color = appearance.theme().active_ui_text_color().into();

        FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(self.text.clone())]),
            font_size,
            font_family,
            code_font_family,
            font_color,
            self.highlighted_link.clone(),
        )
        .register_default_click_handlers_with_action_support(|hyperlink_lens, evt, _ctx| {
            match hyperlink_lens {
                HyperlinkLens::Url(url) => {
                    evt.dispatch_typed_action(BannerAction::<T>::HyperlinkClick(HyperlinkUrl {
                        url: url.to_owned(),
                    }));
                }
                HyperlinkLens::Action(action_ref) => {
                    if let Some(action) = action_ref.as_any().downcast_ref::<T>() {
                        evt.dispatch_typed_action(action.clone());
                    }
                }
            }
        })
        .finish()
    }
}

type BannerClickCallback = dyn Fn(&mut EventContext, &AppContext, Vector2F) + 'static;

pub struct BannerTextButton {
    text: String,
    mouse_state_handle: MouseStateHandle,
    on_click: Rc<BannerClickCallback>,
}

impl BannerTextButton {
    pub fn new(text: String, on_click: Rc<BannerClickCallback>) -> Self {
        Self {
            text,
            mouse_state_handle: Default::default(),
            on_click,
        }
    }
}

/// Informational banner for the terminal.
pub struct Banner<T: Action + Clone> {
    /// Optional icon to render at the start of the banner,
    /// before the text content.
    icon: Option<Icon>,

    text_content: BannerTextContent<T>,

    /// Optional buttons to render at the end,
    /// after the text content and before the close button.
    end_buttons: Vec<BannerTextButton>,

    /// Optional close button (X) rendered at the very end of the banner.
    /// Clicking the close button temporarily dismisses the banner.
    close_button_hover_state: Option<MouseStateHandle>,
}

#[derive(Clone, Debug)]
pub enum BannerAction<T: Action + Clone> {
    Dismiss(DismissalType),
    HyperlinkClick(HyperlinkUrl),
    Action(T),
}

impl<T: Action + Clone> Banner<T> {
    /// Creates a plain banner with a close button.
    pub fn new(content: BannerTextContent<T>) -> Self {
        Self::new_internal(content, vec![], /* with_close_button */ true)
    }

    /// Creates a plain banner without a close button.
    pub fn new_without_close(content: BannerTextContent<T>) -> Self {
        Self::new_internal(content, vec![], /* with_close_button */ false)
    }

    /// Creates a banner with the given text buttons and a close button.
    pub fn new_with_buttons(
        content: BannerTextContent<T>,
        buttons: Vec<BannerTextButton>,
        with_close_button: bool,
    ) -> Self {
        Self::new_internal(content, buttons, with_close_button)
    }

    /// Creates a banner with a single "Don't show me again" button
    /// that will permanently dismiss the banner when clicked, as well
    /// as a close button that will temporarily dismiss it when clicked.
    pub fn new_permanently_dismissible(content: BannerTextContent<T>) -> Self {
        Self::new_with_buttons(
            content,
            vec![Self::permanent_dismissal_button()],
            /* with_close_button */ true,
        )
    }

    fn permanent_dismissal_button() -> BannerTextButton {
        BannerTextButton::new(
            String::from("Don't show me again"),
            Rc::new(|ctx, _, _| {
                ctx.dispatch_typed_action(BannerAction::<T>::Dismiss(DismissalType::Permanent));
            }),
        )
    }

    fn new_internal(
        text_content: BannerTextContent<T>,
        end_buttons: Vec<BannerTextButton>,
        with_close_button: bool,
    ) -> Self {
        Self {
            text_content,
            close_button_hover_state: if with_close_button {
                Some(Default::default())
            } else {
                None
            },
            end_buttons,
            icon: None,
        }
    }

    /// Replaces the banner's content with new items.
    pub fn set_content(&mut self, content: BannerTextContent<T>, ctx: &mut ViewContext<Self>) {
        self.text_content = content;
        ctx.notify();
    }

    /// Updates the label of an action button at the given index.
    pub fn set_action_button_label(
        &mut self,
        index: usize,
        label: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(button) = self.end_buttons.get_mut(index) {
            button.text = label.to_owned();
            ctx.notify();
        }
    }

    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    fn render_icon(appearance: &Appearance, icon: &Icon) -> Box<dyn Element> {
        let icon_size = font_size(appearance);
        ConstrainedBox::new(icon.to_warpui_icon(appearance.theme().accent()).finish())
            .with_width(icon_size)
            .with_height(icon_size)
            .finish()
    }

    fn render_close_button(
        &self,
        appearance: &Appearance,
        hover_state: &MouseStateHandle,
    ) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .close_button(CLOSE_BUTTON_DIAMETER, hover_state.clone())
            .with_style(UiComponentStyles {
                font_color: Some(appearance.theme().active_ui_text_color().into()),
                ..Default::default()
            })
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(BannerAction::<T>::Dismiss(DismissalType::Temporary));
            })
            .finish()
    }

    fn render_text_button(
        appearance: &Appearance,
        button_state: &BannerTextButton,
    ) -> Box<dyn Element> {
        let font_size = font_size(appearance);
        let on_click_fn = button_state.on_click.clone();
        appearance
            .ui_builder()
            .button(ButtonVariant::Text, button_state.mouse_state_handle.clone())
            .with_text_label(button_state.text.clone())
            .with_style(UiComponentStyles {
                font_size: Some(font_size),
                font_weight: Some(Weight::Semibold),
                ..Default::default()
            })
            .build()
            .on_click(move |event_ctx, app_ctx, v2f| on_click_fn(event_ctx, app_ctx, v2f))
            .finish()
    }
}

impl<T: Action + Clone> Entity for Banner<T> {
    type Event = BannerEvent<T>;
}

impl<T: Action + Clone> TypedActionView for Banner<T> {
    type Action = BannerAction<T>;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            BannerAction::Dismiss(dismissal_type) => {
                ctx.emit(BannerEvent::Dismiss(*dismissal_type));
            }
            BannerAction::HyperlinkClick(hyperlink) => {
                ctx.notify();
                ctx.open_url(&hyperlink.url);
            }
            BannerAction::Action(action) => {
                ctx.emit(BannerEvent::Action(action.clone()));
            }
        }
    }
}

impl<T: Action + Clone> View for Banner<T> {
    fn ui_name() -> &'static str {
        "Banner"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mut left_side = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_constrain_horizontal_bounds_to_parent(true);

        if let Some(icon) = &self.icon {
            left_side.add_child(
                Container::new(Banner::<T>::render_icon(appearance, icon))
                    .with_margin_right(INNER_MARGIN)
                    .finish(),
            );
        }

        left_side.add_child(Shrinkable::new(1., self.text_content.render(appearance)).finish());

        let mut right_side_banner_actions = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::End);

        for text_button in &self.end_buttons {
            right_side_banner_actions.add_child(
                Container::new(Banner::<T>::render_text_button(appearance, text_button))
                    .with_margin_left(INNER_MARGIN)
                    .finish(),
            );
        }

        if let Some(hover_state) = &self.close_button_hover_state {
            right_side_banner_actions.add_child(
                Container::new(self.render_close_button(appearance, hover_state))
                    .with_margin_left(INNER_MARGIN)
                    .finish(),
            );
        }

        let banner = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_children([
                Shrinkable::new(1., left_side.finish()).finish(),
                right_side_banner_actions.finish(),
            ]);

        Container::new(banner.finish())
            .with_padding_left(HORIZ_PADDING)
            .with_padding_right(HORIZ_PADDING)
            .with_padding_top(VERT_PADDING)
            .with_padding_bottom(VERT_PADDING)
            .with_background(appearance.theme().surface_2())
            .finish()
    }
}

fn font_size(appearance: &Appearance) -> f32 {
    appearance.ui_font_size()
}
