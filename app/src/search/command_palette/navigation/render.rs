use crate::appearance::Appearance;
use crate::context_chips::display_chip::{
    chip_container, render_git_diff_stats_content, render_udi_chip, udi_font_size, GitLineChanges,
    UdiChipConfig,
};
use crate::context_chips::prompt_snapshot::PromptSnapshot;
use crate::context_chips::{ChipValue, ContextChipKind};
use crate::search::command_palette::navigation::search::SessionHighlightIndices;
use crate::search::result_renderer::ItemHighlightState;
use crate::session_management::{CommandContext, SessionNavigationData};
use crate::settings::FontSettings;
use crate::terminal::blockgrid_element::BlockGridElement;
use crate::terminal::grid_size_util::grid_cell_dimensions;
use crate::terminal::ligature_settings::should_use_ligature_rendering;
use crate::terminal::model::blockgrid::BlockGrid;
use crate::terminal::model::grid::Dimensions;
use crate::terminal::safe_mode_settings::get_secret_obfuscation_mode;
use crate::terminal::SizeInfo;
use pathfinder_geometry::vector::vec2f;
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, Highlight,
    ParentElement, Radius, Shrinkable, Wrap,
};
use warpui::fonts::{Properties, Weight};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::units::IntoPixels;
use warpui::{AppContext, Element, SingletonEntity};

/// Renders a navigation session.
pub fn render_navigation_session(
    session: &SessionNavigationData,
    appearance: &Appearance,
    item_highlight_state: ItemHighlightState,
    is_active_session: bool,
    highlight_indices: &SessionHighlightIndices,
    app: &AppContext,
) -> Box<dyn Element> {
    render_navigation_session_internal(
        render_session_label(
            session,
            appearance,
            item_highlight_state,
            is_active_session,
            highlight_indices,
            app,
        )
        .finish(),
    )
}

fn render_navigation_session_internal(label: Box<dyn Element>) -> Box<dyn Element> {
    ConstrainedBox::new(label)
        .with_height(styles::NAVIGATION_PALETTE_ITEM_HEIGHT)
        .finish()
}

fn render_session_label(
    session: &SessionNavigationData,
    appearance: &Appearance,
    item_highlight_state: ItemHighlightState,
    is_active_session: bool,
    highlight_indices: &SessionHighlightIndices,
    app: &AppContext,
) -> Flex {
    let mut navigation_palette_item = Flex::column();

    let prompt = if let Some(ps1_grid) = &session.prompt_elements().ps1_prompt_grid {
        render_prompt_ps1(ps1_grid, appearance, app)
    } else if let Some(snapshot) = &session.prompt_elements().prompt_chip_snapshot {
        render_prompt_udi(snapshot, appearance)
    } else {
        // Fallback: empty container if neither is available (e.g. very early startup).
        Container::new(Flex::row().finish()).finish()
    };

    let command_info = render_command_context(
        session,
        item_highlight_state,
        is_active_session,
        highlight_indices.command_indices.clone(),
        highlight_indices.hint_text_indices.clone(),
        appearance,
    );

    navigation_palette_item.add_child(
        Container::new(prompt)
            .with_margin_right(styles::NAVIGATION_PALETTE_ROW_HORIZONTAL_SPACING)
            .finish(),
    );

    navigation_palette_item.add_child(
        Container::new(command_info)
            .with_margin_top(styles::NAVIGATION_PALETTE_ROW_VERTICAL_SPACING)
            .with_margin_right(styles::NAVIGATION_PALETTE_ROW_HORIZONTAL_SPACING)
            .finish(),
    );

    navigation_palette_item
}

fn render_current_session_pill(
    command_context: CommandContext,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let current_session_pill = appearance
        .ui_builder()
        .span("Current".to_string())
        .with_style(UiComponentStyles {
            font_family_id: Some(appearance.monospace_font_family()),
            // The font size is scaled down to make sure the pill fits in the row with its padding.
            font_size: Some(appearance.monospace_font_size() * 0.85),
            font_color: Some(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().background())
                    .into_solid(),
            ),
            ..Default::default()
        })
        .build()
        .with_padding_left(5.)
        .with_padding_right(5.)
        .with_margin_left(10.)
        .with_margin_right(8.)
        .with_background_color(appearance.theme().background().into_solid())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .finish();

    Shrinkable::new(
        // We need different flex values when different hint texts are present, otherwise the actual command won't take up enough room.
        match command_context {
            CommandContext::LastRunCommand { .. } | CommandContext::LastRunAIBlock { .. } => 0.5,
            CommandContext::RunningCommand { .. } | CommandContext::RunningAIBlock { .. } => 0.35,
            CommandContext::None => 1.,
        },
        Align::new(
            ConstrainedBox::new(current_session_pill)
                .with_max_width(135.)
                .finish(),
        )
        .right()
        .finish(),
    )
    .finish()
}

/// Renders the prompt as UDI-style context chips from a [`PromptSnapshot`].
fn render_prompt_udi(snapshot: &PromptSnapshot, appearance: &Appearance) -> Box<dyn Element> {
    let mut chip_row = Wrap::row().with_spacing(4.);

    for chip_result in snapshot.chips() {
        let Some(value) = chip_result.value() else {
            continue;
        };
        // GitDiffStats are rendered differently than other chips, so we handle them separately.
        // This ensures that the rendered chip matches the live input chip.
        if matches!(chip_result.kind(), ContextChipKind::GitDiffStats) {
            let line_changes = match value {
                ChipValue::GitDiffStats(g) => g.clone(),
                ChipValue::Text(raw) => {
                    let Some(parsed) = GitLineChanges::parse_from_git_output(raw) else {
                        continue;
                    };
                    parsed
                }
                ChipValue::GithubPullRequest(_) => continue,
            };
            let font_size = udi_font_size(appearance);
            let content = render_git_diff_stats_content(
                &line_changes,
                font_size,
                appearance.monospace_font_family(),
                font_size,
                appearance,
            );
            chip_row.add_child(chip_container(content, Some(Border::all(0.)), appearance).finish());
            continue;
        }

        let color = chip_result
            .kind()
            .default_styles(appearance, false)
            .value_color;
        let value_text = value.to_string();
        let config = if let Some(icon) = chip_result.kind().udi_icon() {
            UdiChipConfig::new_with_icon(icon, color, value_text)
        } else {
            UdiChipConfig::new(color, value_text)
        }
        .with_border_override(Border::all(0.));
        chip_row.add_child(render_udi_chip(config, appearance));
    }

    let prompt_section = Container::new(chip_row.finish())
        .with_margin_right(styles::NAVIGATION_PALETTE_COMMAND_HINT_MARGIN * 2.);

    prompt_section.finish()
}

/// Renders the prompt from the raw PS1 terminal grid, preserving full
/// fidelity of the user's custom prompt (colors, glyphs, etc.).
fn render_prompt_ps1(
    prompt_grid: &BlockGrid,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let cell_dimensions = grid_cell_dimensions(
        app.font_cache(),
        appearance.monospace_font_family(),
        appearance.monospace_font_size(),
        appearance.line_height_ratio(),
    );
    // Derive the SizeInfo width from the grid's own column count so the
    // element renders at its natural size. The parent flex layout will
    // constrain it to the available palette width.
    let grid_width_px = prompt_grid.grid_handler().columns() as f32 * cell_dimensions.x();
    let size_info = SizeInfo::new(
        vec2f(grid_width_px, cell_dimensions.y()),
        cell_dimensions.x().into_pixels(),
        cell_dimensions.y().into_pixels(),
        0.0.into_pixels(),
        0.0.into_pixels(),
    );
    let enforce_minimum_contrast = *FontSettings::as_ref(app).enforce_minimum_contrast;
    let obfuscate_secrets = get_secret_obfuscation_mode(app);
    let mut block_grid_element = BlockGridElement::new(
        prompt_grid,
        appearance,
        enforce_minimum_contrast,
        obfuscate_secrets,
        size_info,
    );
    if should_use_ligature_rendering(app) {
        block_grid_element = block_grid_element.with_ligature_rendering();
    }

    let prompt_section = Container::new(block_grid_element.finish())
        .with_margin_right(styles::NAVIGATION_PALETTE_COMMAND_HINT_MARGIN * 2.);

    prompt_section.finish()
}

fn render_command_context(
    session: &SessionNavigationData,
    item_highlight_state: ItemHighlightState,
    is_active_session: bool,
    command_indices: Option<Vec<usize>>,
    hint_text_indices: Vec<usize>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let command_render_info = CommandRenderInfo::from_context(session.command_context());

    let mut command_row = Flex::row();
    let command_row_font_size = appearance.monospace_font_size() - 2.;

    if let Some(command_text) = command_render_info.command_text {
        if !command_text.is_empty() {
            let running_command_text_color =
                item_highlight_state.main_text_fill(appearance).into_solid();

            let mut running_command_text =
                appearance
                    .ui_builder()
                    .span(command_text)
                    .with_style(UiComponentStyles {
                        font_family_id: Some(appearance.monospace_font_family()),
                        font_size: Some(command_row_font_size),
                        font_color: Some(running_command_text_color),
                        ..Default::default()
                    });

            if let Some(command_indices) = command_indices {
                let highlight = Highlight::new()
                    .with_properties(Properties::default().weight(Weight::Bold))
                    .with_foreground_color(running_command_text_color);
                running_command_text =
                    running_command_text.with_highlights(command_indices, highlight);
            }

            command_row.add_child(
                Shrinkable::new(
                    1.,
                    Container::new(running_command_text.build().finish())
                        .with_margin_right(command_render_info.row_spacing)
                        .finish(),
                )
                .finish(),
            );
        }
    }

    let hint_font_color = item_highlight_state.sub_text_fill(appearance).into_solid();

    let mut hint_text = appearance
        .ui_builder()
        .span(command_render_info.hint_text)
        .with_style(UiComponentStyles {
            font_color: Some(hint_font_color),
            font_family_id: Some(appearance.monospace_font_family()),
            font_size: Some(command_row_font_size),
            ..Default::default()
        });

    let highlight = Highlight::new()
        .with_properties(Properties::default().weight(Weight::Bold))
        .with_foreground_color(hint_font_color);
    hint_text = hint_text.with_highlights(hint_text_indices, highlight);

    command_row.add_child(
        Container::new(hint_text.build().finish())
            .with_margin_left(command_render_info.hint_margin)
            .with_margin_right(command_render_info.hint_margin)
            .finish(),
    );

    if is_active_session {
        command_row.add_child(render_current_session_pill(
            session.command_context(),
            appearance,
        ));
    }

    command_row = command_row.with_cross_axis_alignment(CrossAxisAlignment::End);

    command_row.finish()
}

pub(super) struct CommandRenderInfo {
    pub command_text: Option<String>,
    pub hint_text: String,
    row_spacing: f32,
    hint_margin: f32,
}

impl CommandRenderInfo {
    pub fn from_context(command_context: CommandContext) -> CommandRenderInfo {
        match command_context {
            CommandContext::RunningCommand { running_command } => CommandRenderInfo {
                command_text: Some(running_command),
                hint_text: "Running...".to_string(),
                row_spacing: styles::NAVIGATION_PALETTE_COMMAND_ROW_SPACING,
                hint_margin: styles::NAVIGATION_PALETTE_COMMAND_HINT_MARGIN,
            },
            CommandContext::LastRunCommand {
                last_run_command,
                mins_since_completion,
            } => CommandRenderInfo {
                row_spacing: match last_run_command.is_empty() {
                    true => 0., // Don't include any spacing if the command is empty.
                    false => styles::NAVIGATION_PALETTE_COMMAND_ROW_SPACING,
                },
                hint_margin: match last_run_command.is_empty() {
                    true => 0., // Don't include any margin if the command is empty.
                    false => styles::NAVIGATION_PALETTE_COMMAND_HINT_MARGIN,
                },
                command_text: Some(last_run_command),
                hint_text: match mins_since_completion {
                    Some(mins) if mins >= 60 => "Completed over 1 hour ago".to_string(),
                    Some(mins) if mins == 1 => format!("Completed {mins} minute ago"),
                    Some(mins) => format!("Completed {mins} minutes ago"),
                    None => "No timestamp found".to_string(),
                },
            },
            CommandContext::RunningAIBlock { prompt } => CommandRenderInfo {
                command_text: Some(prompt),
                hint_text: "Running...".to_string(),
                row_spacing: styles::NAVIGATION_PALETTE_COMMAND_ROW_SPACING,
                hint_margin: styles::NAVIGATION_PALETTE_COMMAND_HINT_MARGIN,
            },
            CommandContext::LastRunAIBlock { prompt } => CommandRenderInfo {
                command_text: Some(prompt),
                hint_text: "Completed".to_string(),
                row_spacing: styles::NAVIGATION_PALETTE_COMMAND_ROW_SPACING,
                hint_margin: styles::NAVIGATION_PALETTE_COMMAND_HINT_MARGIN,
            },
            CommandContext::None => CommandRenderInfo {
                command_text: Some(String::new()),
                hint_text: "Empty Session".to_string(),
                row_spacing: 0.,
                hint_margin: 0.,
            },
        }
    }
}

mod styles {
    pub const NAVIGATION_PALETTE_ITEM_HEIGHT: f32 = 70.;

    pub const NAVIGATION_PALETTE_ROW_VERTICAL_SPACING: f32 = 4.;

    pub const NAVIGATION_PALETTE_ROW_HORIZONTAL_SPACING: f32 = 5.;

    pub const NAVIGATION_PALETTE_COMMAND_ROW_SPACING: f32 = 10.;
    pub const NAVIGATION_PALETTE_COMMAND_HINT_MARGIN: f32 = 5.;
}
