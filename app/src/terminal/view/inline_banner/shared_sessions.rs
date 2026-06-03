//! The rendering logic for shared session banners.
use chrono::{DateTime, Datelike, Local};
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, MainAxisSize,
    ParentElement, Radius, Rect, Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::Element;

use crate::appearance::Appearance;

fn render_inline_shared_session_banner(
    is_active: bool,
    label: String,
    datetime: DateTime<Local>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let border_fill = if is_active {
        appearance.theme().terminal_colors().normal.red.into()
    } else {
        appearance.theme().surface_2()
    };

    let left_line = ConstrainedBox::new(Rect::new().with_background(border_fill).finish())
        .with_height(1.)
        .finish();

    let right_line = ConstrainedBox::new(Rect::new().with_background(border_fill).finish())
        .with_height(1.)
        .finish();

    let today = Local::now();
    let is_today = datetime.year() == today.year() && datetime.ordinal() == today.ordinal();
    let is_zh = i18n::current_locale().starts_with("zh");
    let day_str = if is_today {
        i18n::t("terminal.inline_banner.shared_session.today")
    } else if is_zh {
        format!("{}月{}日", datetime.month(), datetime.day())
    } else {
        // Formatted as "Month Day", e.g. "October 10".
        datetime.format("%B %e").to_string()
    };

    // TODO: look into using the OS's locale to format the time according
    // to user's preferences.
    let time_str = if is_zh {
        datetime.format("%H:%M").to_string()
    } else {
        datetime.format("%l:%M%P").to_string()
    };
    let datetime_str = if is_zh {
        format!("{day_str} {time_str}")
    } else {
        format!("{day_str}, {time_str}")
    };

    let pill = Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    Text::new_inline(
                        label,
                        appearance.ui_font_family(),
                        appearance.monospace_font_size(),
                    )
                    .with_color(appearance.theme().active_ui_text_color().into())
                    .with_style(Properties::default().weight(Weight::Bold))
                    .finish(),
                )
                .with_padding_right(8.)
                .finish(),
            )
            .with_child(
                Text::new_inline(
                    datetime_str,
                    appearance.ui_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_color(
                    appearance
                        .theme()
                        .sub_text_color(appearance.theme().surface_2())
                        .into(),
                )
                .finish(),
            )
            .finish(),
    )
    .with_border(Border::all(1.).with_border_fill(border_fill))
    .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
    .with_horizontal_padding(12.)
    .with_vertical_margin(4.)
    .finish();

    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Max)
        .with_child(Shrinkable::new(1., left_line).finish())
        .with_child(pill)
        .with_child(Shrinkable::new(1., right_line).finish())
        .finish()
}

pub fn render_inline_shared_session_started_banner(
    is_active: bool,
    is_shared_ambient_agent_session: bool,
    is_remote_control: bool,
    started_at: DateTime<Local>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let label = if is_shared_ambient_agent_session {
        i18n::t("terminal.inline_banner.shared_session.environment_started")
    } else if is_remote_control {
        i18n::t("terminal.inline_banner.shared_session.remote_control_active")
    } else {
        i18n::t("terminal.inline_banner.shared_session.sharing_started")
    };
    render_inline_shared_session_banner(is_active, label, started_at, appearance)
}

pub fn render_inline_shared_session_ended_banner(
    is_shared_ambient_agent_session: bool,
    is_remote_control: bool,
    ended_at: DateTime<Local>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let label = if is_shared_ambient_agent_session {
        i18n::t("terminal.inline_banner.shared_session.environment_ended")
    } else if is_remote_control {
        i18n::t("terminal.inline_banner.shared_session.remote_control_stopped")
    } else {
        i18n::t("terminal.inline_banner.shared_session.sharing_ended")
    };
    render_inline_shared_session_banner(false, label, ended_at, appearance)
}
