//! Renders the user query portion of the AI block, if there is one.
//!
//! Queries are not rendered in blocks corresponding to requested command or requested action responses.

use pathfinder_color::ColorU;
use warp_core::features::FeatureFlag;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Container, CornerRadius, DispatchEventResult, EventHandler, Flex, MainAxisAlignment,
    MainAxisSize, ParentElement, Radius, Shrinkable, Wrap,
};
use warpui::fonts::{Properties, Style, Weight};
use warpui::ui_components::chip::Chip;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, SingletonEntity};

use super::common::{FindContext, render_query_text, render_user_avatar};
use crate::ai::blocklist::AttachmentType;
use crate::ai::blocklist::block::view_impl::common::UserQueryProps;
use crate::ai::blocklist::block::{AIBlockAction, DetectedLinksState, SecretRedactionState};
use crate::appearance::Appearance;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;

/// Data required to render the AI block query component.
#[derive(Copy, Clone, Debug)]
pub(super) struct Props<'a> {
    pub(super) user_display_name: &'a String,
    pub(super) profile_image_path: Option<&'a String>,
    pub(super) avatar_color: Option<ColorU>,
    pub(super) query_and_index: Option<(&'a str, usize)>,
    pub(super) query_prefix_highlight_len: Option<usize>,
    pub(super) detected_links_state: &'a DetectedLinksState,
    pub(super) secret_redaction_state: &'a SecretRedactionState,
    pub(super) is_selecting_text: bool,
    pub(super) is_ai_input_enabled: bool,
    pub(super) attachments: &'a [(AttachmentType, String)],
    pub(super) find_context: Option<FindContext<'a>>,
}

pub(super) fn maybe_render(props: Props, app: &AppContext) -> Option<Box<dyn Element>> {
    props.query_and_index.map(|(query, input_index)| {
        render_query(
            query,
            props.user_display_name,
            props.profile_image_path,
            props.avatar_color,
            props.detected_links_state,
            props.secret_redaction_state,
            input_index,
            props.query_prefix_highlight_len,
            props.is_selecting_text,
            props.is_ai_input_enabled,
            props.attachments,
            props.find_context,
            app,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_query(
    query: &str,
    user_display_name: &str,
    profile_image_path: Option<&String>,
    avatar_color: Option<ColorU>,
    detected_links_state: &DetectedLinksState,
    secret_redaction_state: &SecretRedactionState,
    input_index: usize,
    query_prefix_highlight_len: Option<usize>,
    is_selecting: bool,
    is_ai_input_enabled: bool,
    attachments: &[(AttachmentType, String)],
    find_context: Option<FindContext>,
    app: &AppContext,
) -> Box<dyn Element> {
    let avatar = Container::new(render_user_avatar(
        user_display_name,
        profile_image_path,
        avatar_color,
        app,
    ))
    .with_margin_right(16.)
    .finish();

    let properties = Properties {
        style: Style::Normal,
        weight: Weight::Bold,
    };
    // The query already includes the /plan prefix when in plan mode via display_user_query()
    let text_element = render_query_text(
        UserQueryProps {
            text: query.to_owned(),
            query_prefix_highlight_len,
            detected_links_state,
            secret_redaction_state,
            input_index,
            is_selecting,
            is_ai_input_enabled,
            find_context,
            font_properties: &properties,
        },
        app,
    );

    let appearance = Appearance::as_ref(app);
    let mut query = Flex::column().with_child(text_element.finish());

    if FeatureFlag::ImageAsContext.is_enabled() {
        query = query.with_child(render_attachments(attachments, appearance));
    }

    Flex::row()
        .with_cross_axis_alignment(warpui::elements::CrossAxisAlignment::Start)
        .with_child(avatar)
        .with_child(Shrinkable::new(1., query.finish()).finish())
        .finish()
}

fn render_attachments(
    attachments: &[(AttachmentType, String)],
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut image_index = 0;
    let chips = attachments.iter().map(|(attachment_type, file_name)| {
        let icon = match attachment_type {
            AttachmentType::Image => Icon::Image,
            AttachmentType::File => Icon::File,
        };
        let chip = Chip::new(
            file_name.clone(),
            UiComponentStyles {
                margin: Some(Coords {
                    top: 0.,
                    bottom: 0.,
                    left: 0.,
                    right: 6.,
                }),
                font_family_id: Some(appearance.ui_font_family()),
                font_size: Some(appearance.monospace_font_size()),
                font_color: Some(blended_colors::text_sub(
                    appearance.theme(),
                    appearance.theme().background(),
                )),
                border_width: Some(1.),
                border_color: Some(internal_colors::neutral_4(appearance.theme()).into()),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(5.))),
                ..Default::default()
            },
        )
        .with_icon(icon.to_warpui_icon(
            blended_colors::text_sub(appearance.theme(), appearance.theme().background()).into(),
        ))
        .build()
        .finish();

        if matches!(attachment_type, AttachmentType::Image) {
            let clicked_image_index = image_index;
            image_index += 1;
            EventHandler::new(chip)
                .on_left_mouse_down(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AIBlockAction::OpenSubmittedAttachmentLightbox {
                        image_index: clicked_image_index,
                    });
                    DispatchEventResult::StopPropagation
                })
                .finish()
        } else {
            chip
        }
    });

    if attachments.is_empty() {
        Flex::row().finish()
    } else {
        let wrapping_section = Wrap::row()
            .with_run_spacing(8.)
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Min)
            .with_children(chips)
            .finish();
        Container::new(wrapping_section)
            .with_padding_top(7.)
            .finish()
    }
}
