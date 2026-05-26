//! This module contains logic to render inline banners for various use cases in the Blocklist. An
//! inline banner is distinct from a typical app banner in that inline banner are rendered within
//! the Blocklist (between blocks) while app banners are pinned to the top of the window.
mod agent_mode_setup;
mod alias_expansion;
mod anonymous_user_ai_sign_up;
mod aws_bedrock_login;
mod aws_cli_not_installed;
mod notifications_discovery;
mod notifications_error;
mod open_in_warp;
mod passive_code_diff;
pub(crate) mod prompt_suggestions;
mod session_state;
mod shared_sessions;
mod shell_process_terminated;
mod ssh;
mod vim_mode;

pub use agent_mode_setup::*;
pub use alias_expansion::*;
pub use anonymous_user_ai_sign_up::*;
pub use aws_bedrock_login::*;
pub use aws_cli_not_installed::*;
pub use notifications_discovery::*;
pub use notifications_error::*;
pub use open_in_warp::*;
pub use passive_code_diff::*;
use pathfinder_color::ColorU;
pub use session_state::*;
pub use shared_sessions::*;
pub use shell_process_terminated::*;
pub use ssh::*;
pub use vim_mode::*;
use warpui::elements::{
    Align, Clipped, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, Icon,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, SavePosition,
    Shrinkable, Text,
};
use warpui::fonts::{FamilyId, Properties, Weight};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::Element;

pub use self::prompt_suggestions::*;
use crate::appearance::Appearance;
use crate::terminal::view::TerminalAction;
use crate::themes::theme::Blend;
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons::Icon as UiIcon;
use crate::util::color::{coloru_with_opacity, ContrastingColor, MinimumAllowedContrast, Opacity};

pub const INLINE_BANNER_BUTTON_PADDING: f32 = 4.0;
const INLINE_BANNER_MARGIN: f32 = 20.0;
pub const INLINE_BANNER_RIGHT_MARGIN: f32 = 15.0;
pub const INLINE_BANNER_MARGIN_BETWEEN_BUTTONS: f32 = 8.;
pub const INLINE_BANNER_BUTTON_HOVER_OPACITY: u8 = 25;

/// Styling variations for inline banners.
#[derive(Clone, Copy, Debug, PartialEq)]
enum InlineBannerStyle {
    /// Styling for an inline banner that is requesting user action.
    /// Mock: https://www.figma.com/file/vgZqQ1YvHgFrAX83QO9DkB/SSH-wrapper?node-id=2%3A95
    CallToAction,
    /// Styling for an inline banner that is recommending an action, but at lower priority than a
    /// CTA.
    /// Mock: https://www.figma.com/file/hCvzJWMtWq38rDMPNOLc2l/Notebooks-UX?node-id=1198:559&mode=dev
    Recommendation,
    /// Styling for an inline banner that is giving the user low-priority
    /// information.
    /// Mock: https://www.figma.com/file/vgZqQ1YvHgFrAX83QO9DkB/SSH-wrapper?node-id=201%3A418
    LowPriority,
    /// Styling for an inline banner that is giving the user very low-priority
    /// information.
    /// Mock: https://www.figma.com/file/vgZqQ1YvHgFrAX83QO9DkB/SSH-wrapper?node-id=201%3A570
    VeryLowPriority,
}

struct InlineBannerButtonState {
    pub on_click_event: TerminalAction,
    pub mouse_state_handle: MouseStateHandle,
}

#[derive(Clone)]
enum InlineBannerTextButtonVariant {
    /// Text with outline.
    Primary,
    /// Text only.
    Secondary,
}

/// Currently, we only support dynamic text-based buttons for inline banners.
struct InlineBannerTextButton {
    pub text: String,
    pub text_color: ColorU,
    pub button_state: InlineBannerButtonState,
    /// The font properties the button text should be rendered with. Defaults to the UI font at normal
    /// size and semi-bold weight.
    pub font: InlineBannerTextButtonFont,
    /// ID to save the button location in the position cache. This is mostly useful for integration
    /// tests.
    pub position_id: Option<String>,
    pub variant: InlineBannerTextButtonVariant,
}

struct InlineBannerTextButtonFont {
    pub family: Option<FamilyId>,
    pub weight: Option<Weight>,
}

impl Default for InlineBannerTextButtonFont {
    fn default() -> Self {
        Self {
            family: None,
            weight: Some(Weight::Semibold),
        }
    }
}

/// The close button is special since it's a singleton.
struct InlineBannerCloseButton(pub InlineBannerButtonState);

/// Icon to render within the banner.
#[derive(Default)]
struct InlineBannerIcon {
    /// The path of the image to render.
    pub asset_path: &'static str,

    /// Aspect ratio of the icon. Necessary to center the icon within the banner.
    pub aspect_ratio: f32,

    // Optional override for icon color. If `None`, defaults to the theme accent color.
    pub color_override: Option<ColorU>,
}

/// Content to render within an inline banner within the block list.
#[derive(Default)]
struct InlineBannerContent {
    /// The title of the banner.
    pub title: String,
    /// Optional content to render after the title.
    pub content: Option<Vec<Text>>,
    /// Buttons to render after the title and content.
    pub buttons: Vec<InlineBannerTextButton>,
    /// An optional close button to render after the buttons.
    pub close_button: Option<InlineBannerCloseButton>,
    /// An optional icon to render _before_ the title (or any other content).
    pub header_icon: Option<InlineBannerIcon>,
    /// Whether to align title and content vertically instead of horizontally.
    pub vertical_align_title_content: bool,
}

fn render_title(
    title: String,
    appearance: &Appearance,
    title_font_size: f32,
    text_opacity: Opacity,
) -> Box<dyn Element> {
    Text::new_inline(title, appearance.ui_font_family(), title_font_size)
        .with_color(
            appearance
                .theme()
                .active_ui_text_color()
                .with_opacity(text_opacity)
                .into_solid(),
        )
        .with_style(Properties::default())
        .soft_wrap(true)
        .finish()
}

fn render_inline_block_list_banner(
    style: InlineBannerStyle,
    appearance: &Appearance,
    inline_banner_content: InlineBannerContent,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let title_font_size = appearance.ui_font_size() + 1.;
    let button_text_size = title_font_size;
    let hover_background_fill = warpui::elements::Fill::from(
        theme
            .active_ui_text_color()
            .with_opacity(INLINE_BANNER_BUTTON_HOVER_OPACITY),
    );

    let default_button_styles = UiComponentStyles {
        font_size: Some(button_text_size),
        font_family_id: Some(appearance.ui_font_family()),
        padding: Some(Coords {
            top: INLINE_BANNER_BUTTON_PADDING,
            bottom: INLINE_BANNER_BUTTON_PADDING,
            left: INLINE_BANNER_BUTTON_PADDING * 2.0,
            right: INLINE_BANNER_BUTTON_PADDING * 2.0,
        }),
        border_radius: Some(CornerRadius::with_all(Radius::Pixels(
            INLINE_BANNER_BUTTON_PADDING,
        ))),
        ..Default::default()
    };

    let hovered_and_clicked_styles = UiComponentStyles {
        background: Some(hover_background_fill),
        ..default_button_styles
    };

    let text_opacity = match style {
        InlineBannerStyle::CallToAction => 100,
        InlineBannerStyle::Recommendation => 100,
        InlineBannerStyle::LowPriority => 90,
        InlineBannerStyle::VeryLowPriority => 50,
    };

    let mut banner = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_main_axis_size(MainAxisSize::Max);

    // Create an inner flex that holds the text and all of the buttons except the close button
    // (which is right aligned within the banner).
    let mut inner_banner_flex = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

    if let Some(icon) = inline_banner_content.header_icon {
        let color = icon.color_override.unwrap_or(
            theme
                .accent()
                .on_background(theme.surface_2(), MinimumAllowedContrast::NonText)
                .into(),
        );
        inner_banner_flex.add_child(
            Container::new(
                ConstrainedBox::new(Icon::new(icon.asset_path, color).finish())
                    .with_width((button_text_size * 1.5) * icon.aspect_ratio)
                    .with_height(button_text_size * 1.5)
                    .finish(),
            )
            .with_padding_left(INLINE_BANNER_MARGIN)
            .finish(),
        );
    }

    // Add the title
    match inline_banner_content.content {
        Some(content) if !inline_banner_content.vertical_align_title_content => {
            inner_banner_flex.add_child(
                Shrinkable::new(
                    1.,
                    Container::new(render_title(
                        inline_banner_content.title,
                        appearance,
                        title_font_size,
                        text_opacity,
                    ))
                    .with_padding_left(INLINE_BANNER_MARGIN)
                    .with_padding_right(INLINE_BANNER_MARGIN_BETWEEN_BUTTONS)
                    .finish(),
                )
                .finish(),
            );
            inner_banner_flex.extend(
                content
                    .into_iter()
                    .map(|text| Shrinkable::new(1., text.finish()).finish()),
            );
        }
        Some(content) => {
            let mut col = Flex::column()
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(render_title(
                    inline_banner_content.title,
                    appearance,
                    title_font_size,
                    text_opacity,
                ));

            col.extend(content.into_iter().map(|text| text.finish()));

            inner_banner_flex.add_child(
                Shrinkable::new(
                    1.,
                    Container::new(col.finish())
                        .with_padding_left(INLINE_BANNER_MARGIN)
                        .with_padding_right(INLINE_BANNER_MARGIN_BETWEEN_BUTTONS)
                        .finish(),
                )
                .finish(),
            );
        }
        None => {
            inner_banner_flex.add_child(
                Shrinkable::new(
                    1.,
                    Container::new(render_title(
                        inline_banner_content.title,
                        appearance,
                        title_font_size,
                        text_opacity,
                    ))
                    .with_padding_left(INLINE_BANNER_MARGIN)
                    .with_padding_right(INLINE_BANNER_MARGIN_BETWEEN_BUTTONS)
                    .finish(),
                )
                .finish(),
            );
        }
    }

    banner.add_child(Shrinkable::new(1., inner_banner_flex.finish()).finish());

    let mut end_banner_flex = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

    // Add the main buttons, right-aligned
    end_banner_flex.extend(
        inline_banner_content
            .buttons
            .into_iter()
            .map(|button_info| {
                Container::new(render_inline_banner_text_button(
                    button_info,
                    text_opacity,
                    default_button_styles,
                    hovered_and_clicked_styles,
                    appearance,
                ))
                .with_margin_right(INLINE_BANNER_MARGIN_BETWEEN_BUTTONS)
                .finish()
            }),
    );

    // Add an optional close button at the end of the banner
    if let Some(InlineBannerCloseButton(InlineBannerButtonState {
        mouse_state_handle,
        on_click_event,
    })) = inline_banner_content.close_button
    {
        end_banner_flex.add_child(
            Align::new(
                Container::new(
                    icon_button(appearance, UiIcon::X, false, mouse_state_handle)
                        .with_style(UiComponentStyles {
                            padding: Some(Coords::uniform(INLINE_BANNER_BUTTON_PADDING)),
                            ..default_button_styles
                        })
                        .with_active_styles(UiComponentStyles {
                            padding: Some(Coords::uniform(INLINE_BANNER_BUTTON_PADDING)),
                            ..hovered_and_clicked_styles
                        })
                        .with_hovered_styles(UiComponentStyles {
                            padding: Some(Coords::uniform(INLINE_BANNER_BUTTON_PADDING)),
                            ..hovered_and_clicked_styles
                        })
                        .build()
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(on_click_event.clone());
                        })
                        .finish(),
                )
                .with_margin_right(INLINE_BANNER_RIGHT_MARGIN)
                .with_margin_left(INLINE_BANNER_MARGIN_BETWEEN_BUTTONS)
                .finish(),
            )
            .right()
            .finish(),
        );
    }

    banner.add_child(end_banner_flex.finish());

    let background = match style {
        InlineBannerStyle::CallToAction => theme.accent().with_opacity(80),
        InlineBannerStyle::Recommendation => theme.surface_2(),
        InlineBannerStyle::LowPriority => {
            theme
                .accent()
                .with_opacity(40)
                .blend(&crate::themes::theme::Fill::Solid(ColorU::new(
                    0, 0, 0, 153,
                )))
        }
        InlineBannerStyle::VeryLowPriority => {
            crate::themes::theme::Fill::Solid(ColorU::new(0, 0, 0, 51))
        }
    };
    Container::new(Clipped::new(banner.finish()).finish())
        .with_background(background)
        // Add 1px top padding to balance out the 1px overdraw on the bottom
        // and keep everything vertically centered.
        .with_padding_top(1.)
        .with_overdraw_bottom(1.)
        .finish()
}

/// Helper for [`render_inline_block_list_banner`] to render a single text button.
fn render_inline_banner_text_button(
    button_info: InlineBannerTextButton,
    text_opacity: Opacity,
    default_button_styles: UiComponentStyles,
    hovered_and_clicked_styles: UiComponentStyles,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let button_styles = match button_info.variant {
        InlineBannerTextButtonVariant::Primary => UiComponentStyles {
            font_color: Some(coloru_with_opacity(button_info.text_color, text_opacity)),
            font_family_id: button_info.font.family,
            font_weight: button_info.font.weight,
            border_color: Some(warpui::elements::Fill::Solid(coloru_with_opacity(
                button_info.text_color,
                text_opacity,
            ))),
            border_width: Some(1.0),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(
                INLINE_BANNER_BUTTON_PADDING,
            ))),
            ..Default::default()
        },
        InlineBannerTextButtonVariant::Secondary => UiComponentStyles {
            font_color: Some(coloru_with_opacity(button_info.text_color, text_opacity)),
            font_family_id: button_info.font.family,
            font_weight: button_info.font.weight,
            ..Default::default()
        },
    };

    let button = appearance
        .ui_builder()
        .button_with_custom_styles(
            ButtonVariant::Text,
            button_info.button_state.mouse_state_handle,
            default_button_styles,
            Some(hovered_and_clicked_styles),
            Some(hovered_and_clicked_styles),
            Some(hovered_and_clicked_styles),
        )
        .with_text_label(button_info.text)
        .with_style(button_styles)
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(button_info.button_state.on_click_event.clone());
        })
        .finish();

    Container::new(
        Align::new(match button_info.position_id {
            Some(position_id) => SavePosition::new(button, &position_id).finish(),
            None => button,
        })
        .finish(),
    )
    .with_margin_left(INLINE_BANNER_MARGIN_BETWEEN_BUTTONS)
    .finish()
}
