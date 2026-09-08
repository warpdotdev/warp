use warpui::Element;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, Flex, ParentElement, Radius,
};
use warpui::fonts::Weight;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};

use crate::appearance::Appearance;
use crate::terminal::model::block::Block;
use crate::terminal::view::WARP_PROMPT_HEIGHT_LINES;

pub(super) fn render_floating_block_snapshot(
    block: &Block,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let content = get_block_snapshot(block);
    let theme = appearance.theme();
    let ui_builder = appearance.ui_builder();
    let mut block_content = Flex::column();

    let font_color = theme.main_text_color(theme.surface_2()).into();

    let sub_font_color = theme.sub_text_color(theme.surface_2()).into();

    // Block Prompt
    if let Some(prompt) = content.prompt {
        block_content.add_child(
            ui_builder
                .span(prompt)
                .with_style(UiComponentStyles {
                    font_color: Some(font_color),
                    // Preview prompt font size should scale the same way as the block prompt.
                    font_size: Some(appearance.monospace_font_size() * WARP_PROMPT_HEIGHT_LINES),
                    font_family_id: Some(appearance.monospace_font_family()),
                    ..Default::default()
                })
                .build()
                .finish(),
        );
    }

    // Block Command
    block_content.add_child(
        Container::new(
            ui_builder
                .span(content.command)
                .with_style(UiComponentStyles {
                    font_weight: Some(Weight::Bold),
                    font_color: Some(font_color),
                    font_size: Some(appearance.monospace_font_size()),
                    font_family_id: Some(appearance.monospace_font_family()),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_padding_top(10.)
        .finish(),
    );

    // Emitted lines formatted
    let omitted_line_count = match content.omitted_line_count {
        Some(count) if count > 1 => Some(format!("({count} lines omitted)...")),
        Some(count) if count == 1 => Some(format!("({count} line omitted)...")),
        _ => None,
    };

    if let Some(omitted_line_count) = omitted_line_count {
        block_content.add_child(
            Container::new(
                ui_builder
                    .span(omitted_line_count)
                    .with_style(UiComponentStyles {
                        font_color: Some(sub_font_color),
                        font_size: Some(
                            appearance.monospace_font_size() * WARP_PROMPT_HEIGHT_LINES,
                        ),
                        font_family_id: Some(appearance.monospace_font_family()),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_padding_top(10.)
            .finish(),
        );
    }

    // Block Output
    block_content.add_child(
        Container::new(
            ui_builder
                .wrappable_text(content.output, true)
                .with_style(UiComponentStyles {
                    font_color: Some(font_color),
                    font_size: Some(appearance.monospace_font_size()),
                    font_family_id: Some(appearance.monospace_font_family()),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_padding_top(5.)
        .finish(),
    );

    Container::new(
        ConstrainedBox::new(
            Container::new(block_content.finish())
                .with_uniform_padding(16.)
                .finish(),
        )
        .with_width(476.)
        .with_max_height(150.)
        .finish(),
    )
    .with_background(theme.surface_1())
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
    .with_border(Border::all(1.).with_border_fill(theme.outline()))
    .finish()
}

struct BlockSnapshot {
    prompt: Option<String>,
    omitted_line_count: Option<usize>,
    command: String,
    output: String,
}

fn get_block_snapshot(block: &Block) -> BlockSnapshot {
    let command = block.command_to_string().trim().to_string();

    let lines = command.split('\n').collect::<Vec<&str>>();
    let trimmed_command = if lines.len() > 1 {
        format!("{}...", lines[0])
    } else {
        command
    };

    let output = block
        .output_grid()
        .contents_to_string(
            false, /*include_escape_sequences*/
            None,  /*max_rows*/
        )
        .trim()
        .to_string();

    let lines = output.split('\n').collect::<Vec<&str>>();

    let (trimmed_output, omitted_line_count) = match lines.len() {
        count if count > 3 => (lines[(count - 2)..].join("\n"), Some(count - 2)),
        _ => (output, None),
    };

    let prompt = block.pwd().map(String::from);

    BlockSnapshot {
        prompt,
        omitted_line_count,
        command: trimmed_command,
        output: trimmed_output,
    }
}
