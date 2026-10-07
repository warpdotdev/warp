use warp_core::ui::Icon;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Axis, Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty,
    Expanded, Flex, Hoverable, MouseState, ParentElement, Radius, Text, Wrap, WrapFillEntireRun,
};
use warpui::{AppContext, Element, SingletonEntity};

use crate::ai::agent::comment::ReviewComment;
use crate::ai::agent::icons::addressed_comment_icon;
use crate::ai::blocklist::block::CommentElementState;
use crate::code_review::comments::ReviewCommentBatch;

const COMMENT_CHIP_MAX_HEIGHT: f32 = 200.;

/// Displays a series of chips for the "Address Comments" input type.
pub fn address_comment_chips(
    review_request: &ReviewCommentBatch,
    props: super::input::Props,
    app_context: &AppContext,
) -> Box<dyn Element> {
    Wrap::new(Axis::Horizontal)
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_spacing(4.)
        .with_run_spacing(4.)
        .with_children(review_request.comments.iter().map(|comment| {
            let agent_comment: ReviewComment = comment.clone().into();
            comment_chip(agent_comment, props, app_context)
        }))
        .finish()
}

fn comment_chip(
    review_comment: ReviewComment,
    props: super::input::Props,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let review_comment_id = review_comment.id;
    let is_addressed = props.addressed_comment_ids.contains(&review_comment_id);

    let Some(comment_element_state) = props.comments.get(&review_comment_id) else {
        log::warn!(
            "Missing CommentElementState for review comment id {}",
            review_comment.id
        );
        return Empty::new().finish();
    };

    let comment_chip = Hoverable::new(
        comment_element_state.header_toggle_mouse_state.clone(),
        |state| {
            render_comment_chip_internal(
                appearance,
                review_comment,
                comment_element_state,
                state,
                is_addressed,
                app,
            )
        },
    )
    .finish();

    WrapFillEntireRun::new(comment_chip).finish()
}

fn render_comment_chip_internal(
    appearance: &Appearance,
    review_comment: ReviewComment,
    comment_element_state: &CommentElementState,
    mouse_state: &MouseState,
    is_addressed: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let flex_column = Flex::column()
        .with_children([
            changes_chip(
                &review_comment,
                comment_element_state,
                mouse_state,
                is_addressed,
                appearance,
                app,
            ),
            comment_text(comment_element_state),
        ])
        .finish();

    let background_color = if mouse_state.is_hovered() {
        internal_colors::neutral_2(appearance.theme())
    } else {
        internal_colors::neutral_1(appearance.theme())
    };

    Container::new(flex_column)
        .with_background(background_color)
        .with_uniform_padding(8.)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_border(
            Border::all(1.).with_border_fill(internal_colors::neutral_2(appearance.theme())),
        )
        .finish()
}

/// Returns the "changes" chip that displays the file / line number that was changed.
fn changes_chip(
    review_comment: &ReviewComment,
    element_state: &CommentElementState,
    mouse_state: &MouseState,
    is_addressed: bool,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let text_sub = appearance
        .theme()
        .sub_text_color(appearance.theme().background())
        .into_solid();

    let comment_title = Text::new(
        review_comment.title(),
        appearance.ui_font_family(),
        appearance.monospace_font_size() - 2.,
    )
    .with_color(text_sub)
    .finish();

    // Show collapse/expand button on hover
    let max_min_button = if mouse_state.is_hovered() {
        ChildView::new(&element_state.maximize_minimize_button).finish()
    } else {
        // Render an empty element the exact size of the button to ensure there's no jitter
        // as the user hovers over a chip.
        let size = element_state
            .maximize_minimize_button
            .as_ref(app)
            .height(app);
        ConstrainedBox::new(Empty::new().finish())
            .with_height(size)
            .with_width(size)
            .finish()
    };

    Flex::row()
        .with_spacing(4.)
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_children([
            comment_icon(is_addressed, appearance),
            Expanded::new(1., comment_title).finish(),
            Container::new(max_min_button)
                .with_padding_top(-8.)
                .with_padding_right(-8.)
                .finish(),
        ])
        .finish()
}

fn comment_icon(is_addressed: bool, appearance: &Appearance) -> Box<dyn Element> {
    let icon_size = appearance.monospace_font_size() - 2.;

    let icon = if is_addressed {
        addressed_comment_icon(appearance).finish()
    } else {
        Icon::MessageText
            .to_warpui_icon(
                appearance
                    .theme()
                    .sub_text_color(appearance.theme().background()),
            )
            .finish()
    };

    let sized_icon = ConstrainedBox::new(icon)
        .with_width(icon_size)
        .with_height(icon_size)
        .finish();

    Container::new(sized_icon)
        .with_background(internal_colors::fg_overlay_1(appearance.theme()))
        .with_vertical_padding(1.)
        .with_horizontal_padding(2.)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
        .finish()
}

fn comment_text(comment_element_state: &CommentElementState) -> Box<dyn Element> {
    let editor_child_view =
        Container::new(ChildView::new(&comment_element_state.rich_text_editor).finish())
            .with_padding_top(4.)
            .finish();
    if comment_element_state.is_expanded {
        editor_child_view
    } else {
        ConstrainedBox::new(editor_child_view)
            .with_max_height(COMMENT_CHIP_MAX_HEIGHT)
            .finish()
    }
}
