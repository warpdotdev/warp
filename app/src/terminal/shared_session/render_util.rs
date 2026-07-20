use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warpui::elements::{
    ChildAnchor, CornerRadius, Fill, Hoverable, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Stack,
};
use warpui::fonts::Weight;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, SingletonEntity};

use super::presence_manager::{MUTED_AVATAR_BORDER_COLOR, MUTED_PARTICIPANT_COLOR, Participant};
use crate::appearance::Appearance;
use crate::ui_components::avatar::{Avatar, AvatarContent};

pub fn shared_session_indicator_color(appearance: &Appearance) -> ColorU {
    appearance.theme().terminal_colors().normal.red.into()
}

/// Diameter including the border
pub const SHARED_SESSION_AVATAR_DIAMETER: f32 = 20.;
pub const SHARED_SESSION_AVATAR_EXECUTOR_DIAMETER: f32 = 16.;

const SHARED_SESSION_DIAMETER_BORDER_WIDTH: f32 = 1.;

/// Shared helper function for rendering avatar in pane header and selected blocks.
/// Actions on hover and click are handled separately.
pub fn non_hoverable_participant_avatar(
    display_name: String,
    image_url: Option<String>,
    participant_color: ColorU,
    is_muted: bool,
    is_executor: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let background = if is_muted {
        MUTED_PARTICIPANT_COLOR
    } else {
        participant_color
    };
    let border_color = if is_muted {
        MUTED_AVATAR_BORDER_COLOR.into()
    } else if image_url.is_none() {
        appearance.theme().surface_2()
    } else {
        participant_color.into()
    };
    let diameter = if is_executor {
        SHARED_SESSION_AVATAR_EXECUTOR_DIAMETER
    } else {
        SHARED_SESSION_AVATAR_DIAMETER
    };
    let font = if is_executor { 10. } else { 12. };
    Avatar::new(
        image_url
            .map(|url| AvatarContent::Image {
                url,
                display_name: display_name.clone(),
            })
            .unwrap_or(AvatarContent::DisplayName(display_name)),
        UiComponentStyles {
            width: Some(diameter - 2. * SHARED_SESSION_DIAMETER_BORDER_WIDTH),
            height: Some(diameter - 2. * SHARED_SESSION_DIAMETER_BORDER_WIDTH),
            border_radius: Some(CornerRadius::with_all(Radius::Percentage(50.))),
            border_width: Some(SHARED_SESSION_DIAMETER_BORDER_WIDTH),
            border_color: Some(border_color.into()),
            background: Some(background.into()),
            font_color: Some(ColorU::black()),
            font_family_id: Some(appearance.ui_font_family()),
            font_weight: Some(Weight::Bold),
            font_size: Some(font),
            ..Default::default()
        },
    )
    .build()
    .finish()
}

/// Struct containing just fields from the [`Participant`] needed for rendering the avatar,
/// to avoid unnecessary cloning of the other fields in the participant.
#[derive(Clone)]
pub struct ParticipantAvatarParams {
    pub display_name: String,
    pub image_url: Option<String>,
    pub participant_color: ColorU,
    pub is_muted: bool,
    pub tooltip_parent_anchor: ParentAnchor,
    pub tooltip_child_anchor: ChildAnchor,
}

impl ParticipantAvatarParams {
    pub fn new(participant: &Participant, is_self_reconnecting: bool) -> Self {
        Self {
            display_name: participant.info.profile_data.display_name.clone(),
            image_url: participant.info.profile_data.photo_url.clone(),
            participant_color: participant.color.to_owned(),
            is_muted: is_self_reconnecting,
            tooltip_parent_anchor: ParentAnchor::TopRight,
            tooltip_child_anchor: ChildAnchor::BottomRight,
        }
    }
}

/// Helper function to render participant avatar and handle hover in selected blocks.
pub fn participant_avatar_for_selected_block(
    params: ParticipantAvatarParams,
    mouse_state_handle: MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let avatar = non_hoverable_participant_avatar(
        params.display_name.clone(),
        params.image_url,
        params.participant_color,
        params.is_muted,
        false,
        app,
    );

    Hoverable::new(mouse_state_handle, |state| {
        let mut stack = Stack::new().with_child(avatar);
        if state.is_hovered() {
            let tooltip_background = appearance.theme().tooltip_background();
            let tool_tip = appearance
                .ui_builder()
                .tool_tip(params.display_name)
                .with_style(UiComponentStyles {
                    font_size: Some(12.),
                    background: Some(Fill::Solid(tooltip_background)),
                    font_color: Some(appearance.theme().background().into_solid()),
                    ..Default::default()
                });
            stack.add_positioned_overlay_child(
                tool_tip.build().finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    params.tooltip_parent_anchor,
                    params.tooltip_child_anchor,
                ),
            );
        }
        stack.finish()
    })
    .finish()
}
