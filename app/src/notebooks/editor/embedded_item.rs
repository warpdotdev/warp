use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use itertools::Itertools;
use markdown_parser::html_parser::WARP_EMBED_ATTRIBUTE_NAME;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::{Vector2F, vec2f};
use serde_yaml::Mapping;
use string_offset::ByteOffset;
use warp_core::ui::appearance::Appearance;
use warp_editor::content::markdown::MarkdownStyle;
use warp_editor::content::text::TextStylesWithMetadata;
use warp_editor::editor::EmbeddedItemModel;
use warp_editor::extract_block;
use warp_editor::render::BLOCK_FOOTER_HEIGHT;
use warp_editor::render::element::{CursorData, CursorDisplayType, RenderContext, RenderableBlock};
use warp_editor::render::layout::TextLayout;
use warp_editor::render::model::viewport::ViewportItem;
use warp_editor::render::model::{
    BlockItem, BlockSpacing, BrokenBlockEmbedding, EMBEDDED_ITEM_FIRST_LINE_HEIGHT, EmbeddedItem,
    EmbeddedItemHTMLRepresentation, EmbeddedItemRichFormat, LaidOutEmbeddedItem, ParagraphStyles,
    RenderState,
};
use warpui::elements::{Border, ConstrainedBox, CornerRadius, Empty, Margin, Padding, Radius};
use warpui::text_layout::TextFrame;
use warpui::units::{IntoPixels, Pixels};
use warpui::{AppContext, Element, LayoutContext, SingletonEntity, SizeConstraint};

use crate::cloud_object::CloudObject;
use crate::cloud_object::model::persistence::CloudModel;
use crate::drive::DriveObjectType;
use crate::drive::cloud_object_styling::warp_drive_icon_color;
use crate::server::ids::{HashableId, ToServerId};
use crate::ui_components::icons::Icon;
use crate::workflows::workflow::Workflow;
use crate::workflows::{CloudWorkflow, WorkflowId};

// Spacing for the embedded workflow card.
const EMBED_WORKFLOW_SPACING: BlockSpacing = BlockSpacing {
    margin: Margin::uniform(0.)
        .with_top(8.)
        .with_left(4.)
        .with_bottom(8.)
        .with_right(16.),
    padding: Padding::uniform(8.)
        .with_left(16.)
        .with_top(16.)
        // Reserve space for the buttons.
        .with_bottom(BLOCK_FOOTER_HEIGHT),
};

// Spacing for the text sections (e.g. title, command) within the workflow card.
const EMBED_WORKFLOW_TEXT_SPACING: BlockSpacing = BlockSpacing {
    margin: Margin::uniform(0.)
        .with_top(8.)
        .with_left(4.)
        .with_bottom(8.)
        .with_right(16.),
    padding: Padding::uniform(8.)
        .with_left(40.)
        .with_top(16.)
        // Reserve space for the buttons.
        .with_bottom(BLOCK_FOOTER_HEIGHT),
};
const TITLE_TO_DESCRIPTION_PADDING: f32 = 4.;
const DESCRIPTION_TO_COMMAND_PADDING: f32 = 8.;

const WORKFLOW_ICON_SIZE: f32 = 16.;
const WORKFLOW_TEXT_PADDING: f32 = 24.;

#[derive(Debug)]
pub struct EmbeddedWorkflow {
    hashed_id: String,
    syntax_highlights: Vec<(Range<ByteOffset>, ColorU)>,
}

impl EmbeddedWorkflow {
    pub fn new(hashed_id: String) -> Self {
        Self {
            hashed_id,
            syntax_highlights: vec![],
        }
    }

    pub fn with_syntax_highlighting(
        mut self,
        syntax_highlights: Vec<(Range<ByteOffset>, ColorU)>,
    ) -> Self {
        self.syntax_highlights = syntax_highlights;
        self
    }

    pub fn command_text_frames(
        &self,
        command: String,
        command_text_style: &ParagraphStyles,
        text_layout: &TextLayout,
    ) -> Vec<Arc<TextFrame>> {
        // Index of the active syntax styling.
        let mut syntax_style_index = 0;

        // ByteOffset before the current line.
        let mut byteoffset_before_line = ByteOffset::zero();
        let default_command_style =
            text_layout.style_and_font(command_text_style, &TextStylesWithMetadata::default());
        let mut text_frames = vec![];

        for line in command.lines() {
            let mut style_runs = Vec::new();

            let total_line_byteoffset = ByteOffset::from(line.len());
            let mut byteoffset_from_line_start = ByteOffset::zero();

            // Mapping from byte to character offset.
            let byte_to_charoffset_mapping =
                line.char_indices().map(|(index, _)| index).collect_vec();

            while let Some((styling_range, color)) = self.syntax_highlights.get(syntax_style_index)
            {
                // Break out of the loop if either
                // 1) the current byte offset is already past the max of the line.
                // 2) the start of the active styling range is past the max of the line.
                if byteoffset_from_line_start >= total_line_byteoffset
                    || styling_range.start >= byteoffset_before_line + total_line_byteoffset
                {
                    break;
                }

                // Total byte offset from the start of text frame.
                let byteoffset_from_frame_start =
                    byteoffset_from_line_start + byteoffset_before_line;

                // Three scenarios:
                // 1) If byte offset is before the start of the styling range, push a style run with default styling until the start of styling range.
                // 2) If byte offset is after the start and before the end of the styling range, push the style run with the active styling.
                // 3) If byte offset is after the end of the styling range, increment the active styling range index.
                byteoffset_from_line_start = if styling_range.start > byteoffset_from_frame_start {
                    let new_byteoffset = styling_range.start - byteoffset_before_line;
                    style_runs.push((
                        byteoffset_from_line_start..new_byteoffset,
                        default_command_style,
                    ));
                    new_byteoffset
                } else if styling_range.start <= byteoffset_from_frame_start
                    && byteoffset_from_frame_start < styling_range.end
                {
                    let new_byteoffset =
                        (styling_range.end - byteoffset_before_line).min(total_line_byteoffset);
                    let command_style = text_layout.style_and_font(
                        command_text_style,
                        &TextStylesWithMetadata::default().with_color(*color),
                    );
                    style_runs.push((byteoffset_from_line_start..new_byteoffset, command_style));

                    // Only increment the active style range index if we have consumed the entire styling range.
                    if styling_range.end <= total_line_byteoffset + byteoffset_before_line {
                        syntax_style_index += 1;
                    }

                    new_byteoffset
                } else {
                    syntax_style_index += 1;
                    continue;
                };
            }

            // If the byte offset is not past the line max, push a default style run for the remaining part
            // of the line.
            if byteoffset_from_line_start < total_line_byteoffset {
                style_runs.push((
                    byteoffset_from_line_start..total_line_byteoffset,
                    default_command_style,
                ));
            }

            // Translate from byte offsets to character offsets.
            let mut char_style_runs = vec![];
            for (style_range, style) in style_runs {
                let starting_char =
                    match byte_to_charoffset_mapping.binary_search(&style_range.start.as_usize()) {
                        Ok(num) => num,
                        Err(num) => num,
                    };

                let ending_char =
                    match byte_to_charoffset_mapping.binary_search(&style_range.end.as_usize()) {
                        Ok(num) => num,
                        Err(num) => num,
                    };

                char_style_runs.push((starting_char..ending_char, style));
            }

            text_frames.push(text_layout.layout_text(
                line,
                command_text_style,
                &EMBED_WORKFLOW_TEXT_SPACING,
                &char_style_runs,
            ));

            // Include linebreaks into the byte offset.
            byteoffset_before_line += total_line_byteoffset + 1;
        }
        text_frames
    }

    /// Get the backing [`CloudWorkflow`] for this embed.
    fn get_workflow<'a>(&self, app: &'a AppContext) -> Option<&'a CloudWorkflow> {
        // TODO: @ianhodge - replace the `from_hash` when we create a new API for going from
        // sqlite hash id -> uid
        let uid = WorkflowId::from_hash(&self.hashed_id).map(|id| id.to_server_id().uid())?;
        CloudModel::as_ref(app)
            .get_by_uid(&uid)
            .and_then(|object| object.as_any().downcast_ref())
    }
}

impl EmbeddedItem for EmbeddedWorkflow {
    fn layout(&self, text_layout: &TextLayout, app: &AppContext) -> Box<dyn LaidOutEmbeddedItem> {
        let cloud_model = CloudModel::as_ref(app);
        let cloud_workflow = self.get_workflow(app);

        let base_text_style = &text_layout.rich_text_styles().base_text;
        let width = text_layout.max_width() - EMBED_WORKFLOW_TEXT_SPACING.x_axis_offset();

        let Some(workflow) = cloud_workflow.and_then(|workflow| {
            if !workflow.is_trashed(cloud_model) {
                Some(Into::<Workflow>::into(workflow))
            } else {
                None
            }
        }) else {
            return Box::new(BrokenBlockEmbedding::new(width, base_text_style.font_size));
        };

        let command_text_style = &text_layout.rich_text_styles().embedding_text;

        let title_style =
            text_layout.style_and_font(base_text_style, &TextStylesWithMetadata::default());

        let title_frame = text_layout.layout_text(
            workflow.name(),
            base_text_style,
            &EMBED_WORKFLOW_TEXT_SPACING,
            &[(0..workflow.name().chars().count(), title_style)],
        );

        // Use placeholder style for description text.
        let description_style = text_layout.style_and_font(
            base_text_style,
            &TextStylesWithMetadata::default().for_placeholder(),
        );
        let description_frame = workflow.description().map(|description| {
            text_layout.layout_text(
                description,
                base_text_style,
                &EMBED_WORKFLOW_TEXT_SPACING,
                &[(0..description.chars().count(), description_style)],
            )
        });

        let content_frames = self.command_text_frames(
            workflow.content().to_owned(),
            command_text_style,
            text_layout,
        );

        let is_agent_mode_prompt =
            cloud_workflow.is_some_and(|w| w.model().data.is_agent_mode_workflow());

        Box::new(LaidOutEmbeddedWorkflow::new(
            title_frame,
            description_frame,
            content_frames,
            width,
            is_agent_mode_prompt,
        ))
    }

    fn hashed_id(&self) -> &str {
        self.hashed_id.as_str()
    }

    fn to_mapping(&self, style: MarkdownStyle) -> Mapping {
        let mut base = match style {
            MarkdownStyle::Internal => Default::default(),
            MarkdownStyle::Export { app_context, .. } => app_context
                .and_then(|ctx| self.get_workflow(ctx))
                .and_then(|workflow| serde_yaml::to_value(&workflow.model().data).ok())
                .and_then(|value| match value {
                    serde_yaml::Value::Mapping(mapping) => Some(mapping),
                    _ => None,
                })
                .unwrap_or_default(),
        };

        base.insert("id".into(), self.hashed_id().into());
        base
    }

    fn to_rich_format(&self, app: &AppContext) -> EmbeddedItemRichFormat<'_> {
        let cloud_model = CloudModel::as_ref(app);
        let workflow = self.get_workflow(app);

        // If the workflow is no longer accessible or is trashed, set the content to
        // an empty string. But we should still keep the HTML element formatting and
        // attributes so we could re-parse the ID and metadata when pasted into Warp.
        let workflow_content = workflow
            .and_then(|workflow| {
                if !workflow.is_trashed(cloud_model) {
                    Some(workflow.model().data.content().to_owned())
                } else {
                    None
                }
            })
            .unwrap_or("".to_owned());

        EmbeddedItemRichFormat {
            plain_text: workflow_content.clone(),
            html: EmbeddedItemHTMLRepresentation {
                element_name: "pre",
                content: workflow_content,
                attributes: HashMap::from([(WARP_EMBED_ATTRIBUTE_NAME, self.hashed_id())]),
            },
        }
    }
}

#[derive(Debug)]
pub struct LaidOutEmbeddedWorkflow {
    pub title: Arc<TextFrame>,
    pub description: Option<Arc<TextFrame>>,
    pub command: Vec<Arc<TextFrame>>,

    pub title_height: Pixels,
    pub description_height: Option<Pixels>,
    pub command_height: Pixels,

    width: Pixels,
    is_agent_mode_prompt: bool,
}

impl LaidOutEmbeddedWorkflow {
    pub fn new(
        title: Arc<TextFrame>,
        description: Option<Arc<TextFrame>>,
        command: Vec<Arc<TextFrame>>,
        width: Pixels,
        is_agent_mode_prompt: bool,
    ) -> Self {
        let title_height = title
            .lines()
            .iter()
            .fold(0f32, |acc, line| {
                acc + line.font_size * line.line_height_ratio
            })
            .into_pixels();

        let description_height = description.as_ref().map(|description| {
            description
                .lines()
                .iter()
                .fold(0f32, |acc, line| {
                    acc + line.font_size * line.line_height_ratio
                })
                .into_pixels()
        });

        let command_height = command
            .iter()
            .fold(0f32, |acc, frame| {
                acc + frame.lines().iter().fold(0f32, |acc, line| {
                    acc + line.font_size * line.line_height_ratio
                })
            })
            .into_pixels();

        Self {
            title,
            description,
            command,
            title_height,
            description_height,
            command_height,
            width,
            is_agent_mode_prompt,
        }
    }
}

impl LaidOutEmbeddedItem for LaidOutEmbeddedWorkflow {
    fn height(&self) -> Pixels {
        let mut total_height = self.title_height;

        if let Some(height) = self.description_height {
            total_height += TITLE_TO_DESCRIPTION_PADDING.into_pixels() + height;
        }

        total_height += DESCRIPTION_TO_COMMAND_PADDING.into_pixels() + self.command_height;
        total_height
    }

    fn size(&self) -> Vector2F {
        vec2f(self.width.as_f32(), self.height().as_f32())
    }

    fn first_line_bound(&self) -> Vector2F {
        // Use a constant here so we are consistently aligning the block insertion menu.
        vec2f(self.width.as_f32(), EMBEDDED_ITEM_FIRST_LINE_HEIGHT)
    }

    fn element(
        &self,
        _state: &RenderState,
        viewport_item: ViewportItem,
        model: Option<&dyn EmbeddedItemModel>,
        ctx: &AppContext,
    ) -> Box<dyn RenderableBlock> {
        Box::new(RenderableEmbeddedWorkflow::new(
            viewport_item,
            model,
            ctx,
            self.is_agent_mode_prompt,
        ))
    }

    fn spacing(&self) -> BlockSpacing {
        EMBED_WORKFLOW_SPACING
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub struct RenderableEmbeddedWorkflow {
    viewport_item: ViewportItem,
    workflow_icon: Box<dyn Element>,
    border: Option<Border>,
    footer: Box<dyn Element>,
}

impl RenderableEmbeddedWorkflow {
    pub fn new(
        viewport_item: ViewportItem,
        model: Option<&dyn EmbeddedItemModel>,
        ctx: &AppContext,
        is_agent_mode_prompt: bool,
    ) -> Self {
        let appearance = Appearance::as_ref(ctx);

        let (icon, icon_color) = if is_agent_mode_prompt {
            (
                Icon::Prompt,
                warp_drive_icon_color(appearance, DriveObjectType::AgentModeWorkflow),
            )
        } else {
            (
                Icon::Workflow,
                warp_drive_icon_color(appearance, DriveObjectType::Workflow),
            )
        };
        let workflow_icon = ConstrainedBox::new(
            icon.to_warpui_icon(icon_color.into())
                .with_opacity(1.0)
                .finish(),
        )
        .with_height(WORKFLOW_ICON_SIZE)
        .with_width(WORKFLOW_ICON_SIZE)
        .finish();

        let footer = match model.and_then(|model| model.render_item_footer(ctx)) {
            Some(element) => element,
            None => Empty::new().finish(),
        };

        Self {
            viewport_item,
            workflow_icon,
            border: model.and_then(|model| model.border(ctx)),
            footer,
        }
    }
}

impl RenderableBlock for RenderableEmbeddedWorkflow {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(&mut self, _model: &RenderState, ctx: &mut LayoutContext, app: &AppContext) {
        self.workflow_icon.layout(
            SizeConstraint::strict(vec2f(WORKFLOW_ICON_SIZE, WORKFLOW_ICON_SIZE)),
            ctx,
            app,
        );

        self.footer.layout(
            SizeConstraint::strict(vec2f(
                self.viewport_item.content_size.x(),
                BLOCK_FOOTER_HEIGHT,
            )),
            ctx,
            app,
        );
    }

    fn paint(&mut self, model: &RenderState, ctx: &mut RenderContext, app: &AppContext) {
        let content = model.content();
        let embedded_workflow = extract_block!(self.viewport_item, content, (block, BlockItem::Embedded(workflow)) => block.embedded(workflow));

        let workflow: &LaidOutEmbeddedWorkflow = embedded_workflow
            .item
            .as_any()
            .downcast_ref()
            .expect("Should be a workflow");

        // Check if any of the active selections overlap with the embedded workflow.
        let selected = model.offset_in_active_selection(embedded_workflow.start_char_offset);
        // Check if any of the cursors are at the start of the embedded workflow.
        let draw_cursor = model.is_selection_head(embedded_workflow.start_char_offset);

        let styles = model.styles();
        let base_style = &styles.base_text;
        let code_style = &styles.embedding_text;

        let border = self.border.unwrap_or(styles.code_border);

        let background_rect = self.viewport_item.visible_bounds(ctx);

        ctx.paint
            .scene
            .draw_rect_without_hit_recording(background_rect)
            .with_border(border)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_background(model.styles().embedding_background);

        let mut content_origin = embedded_workflow.content_origin();

        // Vertically center the icon relative to the first line of the title text.
        let title_line_height = workflow
            .title
            .lines()
            .first()
            .map_or(workflow.title_height.as_f32(), |line| line.height());
        let workflow_icon_origin =
            content_origin + vec2f(0., (title_line_height - WORKFLOW_ICON_SIZE) / 2.);
        self.workflow_icon
            .paint(ctx.content_to_screen(workflow_icon_origin), ctx.paint, app);

        content_origin += vec2f(WORKFLOW_TEXT_PADDING, 0.);
        ctx.draw_text(
            content_origin,
            Default::default(),
            &workflow.title,
            base_style,
        );
        content_origin += vec2f(0., workflow.title_height.as_f32());

        if let Some(description_frame) = &workflow.description {
            content_origin += vec2f(0., TITLE_TO_DESCRIPTION_PADDING);

            ctx.draw_text(
                content_origin,
                Default::default(),
                description_frame,
                base_style,
            );

            content_origin += vec2f(
                0.,
                workflow.description_height.expect("Should exist").as_f32(),
            )
        }

        content_origin += vec2f(0., DESCRIPTION_TO_COMMAND_PADDING);

        for frame in &workflow.command {
            ctx.draw_text(content_origin, Default::default(), frame, code_style);

            content_origin += vec2f(
                0.,
                frame.lines().iter().fold(0f32, |acc, line| {
                    acc + line.font_size * line.line_height_ratio
                }),
            );
        }

        if selected {
            ctx.paint
                .scene
                .draw_rect_with_hit_recording(background_rect)
                .with_background(styles.selection_fill);
        }

        if draw_cursor {
            let line_height = styles.base_text.line_height().as_f32();
            // The lower right corner of the background rect is at reserved_origin + background_rect.size()
            // Add some horizontal padding and minus line height vertically so it's visible and aligned to
            // the bottom of the background rect.
            let end_of_line_position = embedded_workflow.reserved_origin()
                + background_rect.size()
                + vec2f(5., -line_height);
            ctx.draw_and_save_cursor(
                CursorDisplayType::Bar,
                end_of_line_position,
                vec2f(styles.cursor_width, line_height),
                CursorData::default(),
                styles,
            );
        }

        ctx.paint.scene.start_layer(warpui::ClipBounds::ActiveLayer);

        // Position the block footer right below the content area, flush with its right-hand edge.
        // This gives the footer some padding relative to the visible area with a background.
        let content_rect = self.viewport_item.content_bounds(ctx);
        let button_origin = content_rect.lower_right()
            - vec2f(
                self.footer.size().expect("Footer should be laid out").x(),
                0.,
            );
        self.footer.paint(button_origin, ctx.paint, app);

        ctx.paint.scene.stop_layer();
    }

    fn after_layout(&mut self, ctx: &mut warpui::AfterLayoutContext, app: &warpui::AppContext) {
        self.footer.after_layout(ctx, app);
    }

    fn dispatch_event(
        &mut self,
        _model: &warp_editor::render::model::RenderState,
        event: &warpui::event::DispatchedEvent,
        ctx: &mut warpui::EventContext,
        app: &AppContext,
    ) -> bool {
        self.footer.dispatch_event(event, ctx, app)
    }
}
