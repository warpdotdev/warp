use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::AnsiColorIdentifier;

use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;

pub fn todo_list_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        Icon::BulletedListBlock.into(),
        blended_colors::neutral_7(appearance.theme()),
    )
}

pub fn pending_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        Icon::Queued.into(),
        blended_colors::neutral_5(appearance.theme()),
    )
}

pub fn in_progress_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        Icon::Circle.into(),
        AnsiColorIdentifier::Magenta.to_ansi_color(&appearance.theme().terminal_colors().normal),
    )
}

pub fn succeeded_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        Icon::Check.into(),
        AnsiColorIdentifier::Green.to_ansi_color(&appearance.theme().terminal_colors().normal),
    )
}

pub fn addressed_comment_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        Icon::AddressedComment.into(),
        AnsiColorIdentifier::Green.to_ansi_color(&appearance.theme().terminal_colors().normal),
    )
}

pub fn failed_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        Icon::Triangle.into(),
        AnsiColorIdentifier::Red.to_ansi_color(&appearance.theme().terminal_colors().normal),
    )
}

/// Not running, does not need user's attention
pub fn gray_stop_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        Icon::StopFilled.into(),
        blended_colors::neutral_5(appearance.theme()),
    )
}

/// Agent is waiting for user to follow-up with next prompt.
pub fn gray_clock_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        Icon::ClockSnooze.into(),
        blended_colors::neutral_5(appearance.theme()),
    )
}

/// Loading but not actionable yet.
pub fn gray_circle_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        Icon::Circle.into(),
        blended_colors::neutral_5(appearance.theme()),
    )
}

/// Not running, requires user's attention
pub fn yellow_stop_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        Icon::StopFilled.into(),
        AnsiColorIdentifier::Yellow.to_ansi_color(&appearance.theme().terminal_colors().normal),
    )
}

/// To be used for actions (like running commands/reading files) that are long-running and executing.
pub fn yellow_running_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        Icon::Circle.into(),
        AnsiColorIdentifier::Yellow.to_ansi_color(&appearance.theme().terminal_colors().normal),
    )
}

/// Used for buttons that stop the current task
pub fn red_stop_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(Icon::StopFilled.into(), appearance.theme().ansi_fg_red())
}
