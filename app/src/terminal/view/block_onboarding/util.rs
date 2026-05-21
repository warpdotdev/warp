use crate::appearance::Appearance;
use crate::editor::EditorView;
use crate::ui_components::icons::Icon;
use pathfinder_color::ColorU;
use warp_core::ui::theme::Fill;
use warpui::elements::{Flex, ParentElement};
use warpui::ui_components::components::Coords;
use warpui::ui_components::text_input::TextInput;
use warpui::{
    elements::{
        ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, MouseStateHandle, Radius,
        Shrinkable,
    },
    fonts::Weight,
    platform::Cursor,
    ui_components::{
        button::ButtonVariant,
        components::{UiComponent, UiComponentStyles},
    },
    Action, AppContext, Element,
};
use warpui::{SingletonEntity, ViewHandle};

pub const INPUT_BOX_FONT_SIZE: f32 = 14.;
pub const TEAM_BLOCK_INITIAL_HEIGHT: f32 = 92.;
pub const SKIP_BUTTON_WIDTH: f32 = 60.;
pub const SKIP_BUTTON_HEIGHT: f32 = 40.;
const CREATE_BUTTON_WIDTH: f32 = 120.;
pub const BUTTON_GAP: f32 = 8.;

pub fn render_skip_button<A: Action + Clone>(
    action: A,
    mouse_state_handle: MouseStateHandle,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    appearance
        .ui_builder()
        .button(ButtonVariant::Secondary, mouse_state_handle.clone())
        .with_style(UiComponentStyles {
            font_color: Some(appearance.theme().surface_3().into()),
            font_weight: Some(Weight::Medium),
            width: Some(SKIP_BUTTON_WIDTH),
            height: Some(SKIP_BUTTON_HEIGHT),
            font_size: Some(14.),
            ..Default::default()
        })
        .with_hovered_styles(UiComponentStyles {
            background: Some(appearance.theme().outline().into()),
            ..Default::default()
        })
        .with_centered_text_label(crate::i18n::tr_static(app, "Skip").to_owned())
        .build()
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish()
}

pub fn name(app: &AppContext, team_name_editor: ViewHandle<EditorView>) -> Option<String> {
    let name = team_name_editor.as_ref(app).buffer_text(app);
    if !name.trim().is_empty() {
        Some(name)
    } else {
        None
    }
}

pub fn render_input_row<A: Action + Clone>(
    create_team_action: A,
    skip_action: A,
    is_block_completed: bool,
    mouse_state_handle_create_team_button: MouseStateHandle,
    mouse_state_handle_skip_button: MouseStateHandle,
    ctx: &AppContext,
    team_name_editor: ViewHandle<EditorView>,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(ctx);
    let background_color = appearance.theme().background();
    let font_color = appearance.theme().main_text_color(background_color);
    let chip_editor_style = UiComponentStyles::default()
        .set_background(background_color.into())
        .set_border_radius(CornerRadius::with_all(Radius::Pixels(3.)))
        .set_border_width(1.)
        .set_border_color(appearance.theme().foreground().with_opacity(20).into())
        .set_padding(Coords::uniform(0.).top(4.).right(5.));

    let input_box = Container::new(
        TextInput::new(team_name_editor.clone(), chip_editor_style)
            .with_style(UiComponentStyles {
                width: Some(350.),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
                border_width: Some(1.),
                border_color: Some(font_color.into()),
                padding: Some(Coords::uniform(10.)),
                background: Some(background_color.into()),
                ..Default::default()
            })
            .build()
            .finish(),
    )
    .with_margin_right(8.)
    .finish();

    let mut create_team_button = appearance
        .ui_builder()
        .button(
            ButtonVariant::Accent,
            mouse_state_handle_create_team_button.clone(),
        )
        .with_style(UiComponentStyles {
            font_color: Some(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().accent())
                    .into_solid(),
            ),
            font_weight: Some(Weight::Medium),
            font_size: Some(14.),
            height: Some(SKIP_BUTTON_HEIGHT),
            ..Default::default()
        })
        .with_centered_text_label(crate::i18n::tr_static(ctx, "Create team").to_owned());
    if name(ctx, team_name_editor).is_none() {
        create_team_button = create_team_button
            .with_style(UiComponentStyles {
                font_color: Some(
                    appearance
                        .theme()
                        .disabled_text_color(appearance.theme().background())
                        .into(),
                ),
                ..Default::default()
            })
            .disabled();
    }

    let rendered_buttons = if !is_block_completed {
        let mut row = Flex::row();
        row.add_children([
            Container::new(
                ConstrainedBox::new(
                    create_team_button
                        .build()
                        .with_cursor(Cursor::PointingHand)
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(create_team_action.clone())
                        })
                        .finish(),
                )
                .with_width(CREATE_BUTTON_WIDTH)
                .finish(),
            )
            .with_margin_right(BUTTON_GAP)
            .finish(),
            render_skip_button(
                skip_action.clone(),
                mouse_state_handle_skip_button.clone(),
                appearance,
                ctx,
            ),
        ]);
        row.finish()
    } else {
        ConstrainedBox::new(
            Icon::Check
                .to_warpui_icon(Fill::Solid(ColorU::new(11, 142, 71, 255)))
                .finish(),
        )
        .with_width(24.)
        .with_height(24.)
        .finish()
    };

    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(input_box)
        .with_child(Shrinkable::new(1., rendered_buttons).finish())
        .finish()
}
