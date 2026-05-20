use pathfinder_color::ColorU;
use warp_core::ui::icons::Icon as WarpIcon;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::{Fill, WarpTheme};
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Element, Flex, ParentElement,
};
use warpui::fonts::FamilyId;
use warpui::elements::Text;

use super::github_pr_info::{
    GithubPrInfo, GithubPrMergeStateStatus, GithubPrMergeable, GithubPrState,
};
use super::github_pr_display_text_from_url;

/// Compact visual treatment for PR merge status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GithubPrStatusIndicatorKind {
    Merged,
    Closed,
    Draft,
    Conflicts,
    Blocked,
    Ready,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GithubPrStatusIndicator {
    pub kind: GithubPrStatusIndicatorKind,
}

const STATUS_DOT_SIZE: f32 = 6.;
const STATUS_ICON_SIZE: f32 = 8.;

pub fn github_pr_chip_label(info: &GithubPrInfo) -> String {
    github_pr_display_text_from_url(&info.url)
        .map(|label| label.strip_prefix("PR ").unwrap_or(&label).to_string())
        .unwrap_or_else(|| info.url.clone())
}

pub fn github_pr_chip_label_from_url(url: &str) -> String {
    github_pr_display_text_from_url(url)
        .map(|label| label.strip_prefix("PR ").unwrap_or(&label).to_string())
        .unwrap_or_else(|| url.to_string())
}

pub fn github_pr_status_indicator(info: &GithubPrInfo) -> GithubPrStatusIndicator {
    let kind = if info.state == GithubPrState::Merged {
        GithubPrStatusIndicatorKind::Merged
    } else if info.state == GithubPrState::Closed {
        GithubPrStatusIndicatorKind::Closed
    } else if info.is_draft || info.merge_state_status == GithubPrMergeStateStatus::Draft {
        GithubPrStatusIndicatorKind::Draft
    } else if info.mergeable == GithubPrMergeable::Conflicting
        || info.merge_state_status == GithubPrMergeStateStatus::Dirty
    {
        GithubPrStatusIndicatorKind::Conflicts
    } else if matches!(
        info.merge_state_status,
        GithubPrMergeStateStatus::Blocked | GithubPrMergeStateStatus::Behind
    ) {
        GithubPrStatusIndicatorKind::Blocked
    } else if info.merge_state_status == GithubPrMergeStateStatus::Clean
        && info.mergeable == GithubPrMergeable::Mergeable
    {
        GithubPrStatusIndicatorKind::Ready
    } else {
        GithubPrStatusIndicatorKind::Unknown
    };
    GithubPrStatusIndicator { kind }
}

pub fn github_pr_status_tooltip(info: &GithubPrInfo) -> String {
    let label = github_pr_display_text_from_url(&info.url).unwrap_or_else(|| info.url.clone());
    let status = match github_pr_status_indicator(info).kind {
        GithubPrStatusIndicatorKind::Merged => "Merged",
        GithubPrStatusIndicatorKind::Closed => "Closed",
        GithubPrStatusIndicatorKind::Draft => "Draft",
        GithubPrStatusIndicatorKind::Conflicts => "Merge conflicts",
        GithubPrStatusIndicatorKind::Blocked => "Blocked from merging",
        GithubPrStatusIndicatorKind::Ready => "Ready to merge",
        GithubPrStatusIndicatorKind::Unknown => "Pull request",
    };
    format!("{label} · {status}")
}

pub fn github_pr_status_search_suffix(info: &GithubPrInfo) -> String {
    match github_pr_status_indicator(info).kind {
        GithubPrStatusIndicatorKind::Merged => "merged".to_string(),
        GithubPrStatusIndicatorKind::Closed => "closed".to_string(),
        GithubPrStatusIndicatorKind::Draft => "draft".to_string(),
        GithubPrStatusIndicatorKind::Conflicts => "conflicts".to_string(),
        GithubPrStatusIndicatorKind::Blocked => "blocked".to_string(),
        GithubPrStatusIndicatorKind::Ready => "ready".to_string(),
        GithubPrStatusIndicatorKind::Unknown => String::new(),
    }
}

fn indicator_color(kind: GithubPrStatusIndicatorKind, theme: &WarpTheme) -> ColorU {
    match kind {
        GithubPrStatusIndicatorKind::Merged | GithubPrStatusIndicatorKind::Ready => {
            theme.ansi_fg_green()
        }
        GithubPrStatusIndicatorKind::Closed | GithubPrStatusIndicatorKind::Draft => {
            internal_colors::neutral_6(theme)
        }
        GithubPrStatusIndicatorKind::Conflicts | GithubPrStatusIndicatorKind::Blocked => {
            theme.ui_warning_color()
        }
        GithubPrStatusIndicatorKind::Unknown => internal_colors::neutral_5(theme),
    }
}

/// Renders a compact status dot or check icon for prompt chips and vertical-tab badges.
pub fn render_github_pr_status_indicator(
    indicator: GithubPrStatusIndicator,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    let color = indicator_color(indicator.kind, theme);
    let fill = Fill::Solid(color);
    match indicator.kind {
        GithubPrStatusIndicatorKind::Merged => Container::new(
            ConstrainedBox::new(WarpIcon::Check.to_warpui_icon(fill).finish())
                .with_width(STATUS_ICON_SIZE)
                .with_height(STATUS_ICON_SIZE)
                .finish(),
        )
        .with_margin_left(4.)
        .finish(),
        GithubPrStatusIndicatorKind::Unknown => warpui::elements::Empty::new().finish(),
        _ => Container::new(
            ConstrainedBox::new(WarpIcon::CircleFilled.to_warpui_icon(fill).finish())
                .with_width(STATUS_DOT_SIZE)
                .with_height(STATUS_DOT_SIZE)
                .finish(),
        )
        .with_margin_left(4.)
        .finish(),
    }
}

/// Badge content: GitHub icon, PR number, and optional status indicator.
pub fn render_github_pr_badge_content(
    label: &str,
    info: Option<&GithubPrInfo>,
    sub_text_color: ColorU,
    font_family: FamilyId,
    theme: &WarpTheme,
    github_icon: Box<dyn Element>,
) -> Box<dyn Element> {
    let mut row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(4.)
        .with_child(github_icon);

    row.add_child(
        Text::new_inline(label.to_string(), font_family, 10.)
            .with_color(sub_text_color.into())
            .finish(),
    );

    if let Some(info) = info {
        let indicator = github_pr_status_indicator(info);
        if indicator.kind != GithubPrStatusIndicatorKind::Unknown {
            row.add_child(render_github_pr_status_indicator(indicator, theme));
        }
    }

    row.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_chips::github_pr_info::parse_github_pr_command_output;

    fn sample_info(state: GithubPrState) -> GithubPrInfo {
        GithubPrInfo {
            url: "https://github.com/warp/warp/pull/42".to_string(),
            state,
            is_draft: false,
            mergeable: GithubPrMergeable::Mergeable,
            merge_state_status: GithubPrMergeStateStatus::Clean,
        }
    }

    #[test]
    fn test_parse_github_pr_command_output_json() {
        let raw = r#"{"url":"https://github.com/warp/warp/pull/1","state":"OPEN","isDraft":false,"mergeable":"MERGEABLE","mergeStateStatus":"CLEAN"}"#;
        let info = parse_github_pr_command_output(raw).expect("parse");
        assert_eq!(info.state, GithubPrState::Open);
        assert!(!info.is_draft);
    }

    #[test]
    fn test_parse_github_pr_command_output_legacy_url() {
        let info = parse_github_pr_command_output("https://github.com/warp/warp/pull/9")
            .expect("parse");
        assert_eq!(info.url, "https://github.com/warp/warp/pull/9");
        assert_eq!(info.state, GithubPrState::Open);
    }

    #[test]
    fn test_github_pr_status_indicator_merged() {
        let info = GithubPrInfo {
            state: GithubPrState::Merged,
            ..sample_info(GithubPrState::Merged)
        };
        assert_eq!(
            github_pr_status_indicator(&info).kind,
            GithubPrStatusIndicatorKind::Merged
        );
    }

    #[test]
    fn test_github_pr_status_indicator_conflicts() {
        let info = GithubPrInfo {
            mergeable: GithubPrMergeable::Conflicting,
            ..sample_info(GithubPrState::Open)
        };
        assert_eq!(
            github_pr_status_indicator(&info).kind,
            GithubPrStatusIndicatorKind::Conflicts
        );
    }

    #[test]
    fn test_github_pr_chip_label_strips_pr_prefix() {
        let info = sample_info(GithubPrState::Open);
        assert_eq!(github_pr_chip_label(&info), "#42");
    }
}
