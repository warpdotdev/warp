use std::borrow::Cow;
use std::ops::Sub;

use warpui::elements::{
    Align, ConstrainedBox, Container, CrossAxisAlignment, Flex, Icon, ParentElement, Shrinkable,
};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, SingletonEntity};

use super::HistoryEntry;
use crate::appearance::Appearance;
use crate::input_suggestions::AIQueryHistoryEntryDetails;
use crate::ui_components::icons::Icon as UiIcon;
use crate::util::time_format::{format_approx_duration_from_now, human_readable_precise_duration};

/// Vertical spacing between line items in rich history details.
pub(crate) const DETAILS_PARAGRAPH_SPACING: f32 = 8.;

/// Renders the details panel for rich history items. One of the items, the linked workflow name,
/// can be enabled/disabled since we only want it for some views and not others.
pub fn render_rich_history(entry: &HistoryEntry, ctx: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(ctx);
    let ui_builder = appearance.ui_builder();

    let mut flex_column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    if let Some(workflow) = entry.linked_workflow(ctx) {
        flex_column.add_child(render_row_with_icon_and_paragraph(
            "bundled/svg/workflow.svg",
            workflow.name().to_owned(),
            appearance,
        ))
    }

    if let Some(exit_code) = entry.exit_code {
        let icon = if exit_code.was_successful() {
            UiIcon::CheckSkinny
        } else {
            UiIcon::AlertTriangle
        };
        flex_column.add_child(
            Container::new(render_row_with_icon_and_paragraph(
                icon.into(),
                format!("Exit code {}", exit_code.value()),
                appearance,
            ))
            .with_margin_top(DETAILS_PARAGRAPH_SPACING)
            .finish(),
        );
    }

    if let Some(pwd) = &entry.pwd {
        flex_column.add_child(
            Container::new(render_row_with_icon_and_paragraph(
                UiIcon::Folder.into(),
                pwd.clone(),
                appearance,
            ))
            .with_margin_top(DETAILS_PARAGRAPH_SPACING)
            .finish(),
        );
    }

    if let Some(git_head) = &entry.git_head {
        flex_column.add_child(
            Container::new(render_row_with_icon_and_paragraph(
                UiIcon::GitBranch.into(),
                git_head.clone(),
                appearance,
            ))
            .with_margin_top(DETAILS_PARAGRAPH_SPACING)
            .finish(),
        );
    }

    if let (Some(start_ts), Some(completed_ts)) = (entry.start_ts, entry.completed_ts) {
        flex_column.add_child(
            Container::new(
                ui_builder
                    .paragraph(format!(
                        "Finished in {}",
                        human_readable_precise_duration((completed_ts).sub(start_ts))
                    ))
                    .build()
                    .finish(),
            )
            .with_margin_top(DETAILS_PARAGRAPH_SPACING)
            .finish(),
        );
    }

    if let Some(start_ts) = entry.start_ts {
        flex_column.add_child(
            Container::new(
                ui_builder
                    .paragraph(format!(
                        "Last ran {}",
                        format_approx_duration_from_now(start_ts)
                    ))
                    .build()
                    .finish(),
            )
            .with_margin_top(DETAILS_PARAGRAPH_SPACING)
            .finish(),
        );
    }

    flex_column.finish()
}

pub(crate) fn render_ai_query_rich_history(
    entry: &AIQueryHistoryEntryDetails,
    ctx: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(ctx);

    let mut details_column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(render_row_with_icon_and_paragraph(
            entry.output_status.icon().into(),
            entry.output_status.display_text(),
            appearance,
        ));

    if let Some(working_directory) = &entry.working_directory {
        details_column.add_child(
            Container::new(render_row_with_icon_and_paragraph(
                UiIcon::Folder.into(),
                working_directory.clone(),
                appearance,
            ))
            .with_margin_top(DETAILS_PARAGRAPH_SPACING)
            .finish(),
        );
    }

    details_column.add_child(
        Container::new(
            appearance
                .ui_builder()
                .paragraph(format!(
                    "Ran {}",
                    format_approx_duration_from_now(entry.start_time)
                ))
                .build()
                .finish(),
        )
        .with_margin_top(DETAILS_PARAGRAPH_SPACING)
        .finish(),
    );

    details_column.finish()
}

pub(crate) fn render_row_with_icon_and_paragraph(
    icon_path: &'static str,
    text: impl Into<Cow<'static, str>>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Flex::row()
        .with_children([
            ConstrainedBox::new(Icon::new(icon_path, appearance.theme().foreground()).finish())
                .with_max_height(appearance.ui_font_size())
                .with_max_width(appearance.ui_font_size())
                .finish(),
            Shrinkable::new(
                1.,
                Align::new(
                    appearance
                        .ui_builder()
                        .paragraph(text)
                        .with_style(UiComponentStyles {
                            margin: Some(Coords::uniform(0.).left(4.)),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .left()
                .finish(),
            )
            .finish(),
        ])
        .finish()
}
