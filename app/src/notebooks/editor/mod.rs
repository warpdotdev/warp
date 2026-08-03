//! Rich-text notebooks editor.

use std::sync::Arc;

use markdown_parser::markdown_parser::CODE_BLOCK_DEFAULT_MARKDOWN_LANG;
use pathfinder_color::ColorU;
use warp_core::ui::builder::CHECK_SVG_PATH;
use warp_core::ui::theme::color::internal_colors;
use warp_editor::content::text::{
    BlockHeaderSize, BlockType as ContentBlockType, BufferBlockStyle, CodeBlockType,
};
use warp_editor::render::model::{
    BrokenLinkStyle, CheckBoxStyle, EmbeddedItem, HorizontalRuleStyle, InlineCodeStyle,
    ParagraphStyles, RichTextStyles, TableStyle,
};
use warp_util::user_input::UserInput;
use warpui::elements::{Border, ListIndentLevel};
use warpui::fonts::FamilyId;
use warpui::ui_components::checkbox::HOVER_BACKGROUND_COLOR;

use crate::appearance::Appearance;
use crate::notebooks::editor::embedded_item::EmbeddedWorkflow;
use crate::settings::{FontSettings, derived_notebook_font_size};
use crate::themes::theme::Fill;
use crate::ui_components::icons::Icon;
use crate::util::color::{ContrastingColor, MinimumAllowedContrast};
use crate::workflows::{CloudWorkflow, WorkflowSource, WorkflowType};

mod block_insertion_menu;
mod embedded_item;
mod embedding_model;
mod find_bar;
mod interaction_state_model;
pub mod keys;
mod link_editor;
pub mod model;
pub mod notebook_command;
mod omnibar;
pub mod view;

pub use block_insertion_menu::BlockInsertionSource;
const NOTEBOOK_LINE_HEIGHT_RATIO: f32 = 1.5;
const NOTEBOOK_BASELINE_RATIO: f32 = 0.7;

#[derive(Clone, Copy)]
pub(crate) struct MarkdownTableAppearance {
    pub border_color: ColorU,
    pub header_background: ColorU,
    pub cell_background: ColorU,
    pub alternate_row_background: Option<ColorU>,
    pub text_color: ColorU,
    pub header_text_color: ColorU,
    pub scrollbar_nonactive_thumb_color: ColorU,
    pub scrollbar_active_thumb_color: ColorU,
    pub cell_padding: f32,
    pub outer_border: bool,
    pub column_dividers: bool,
    pub row_dividers: bool,
}

/// A kind of block that can be added to a notebook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    RunnableCommand,
    Code,
    Header(BlockHeaderSize),
    Text,
    UnorderedList,
    OrderedList,
    TaskList,
}

impl BlockType {
    const ALL: [BlockType; 12] = [
        BlockType::RunnableCommand,
        BlockType::Code,
        BlockType::Header(BlockHeaderSize::Header1),
        BlockType::Header(BlockHeaderSize::Header2),
        BlockType::Header(BlockHeaderSize::Header3),
        BlockType::Header(BlockHeaderSize::Header4),
        BlockType::Header(BlockHeaderSize::Header5),
        BlockType::Header(BlockHeaderSize::Header6),
        BlockType::Text,
        BlockType::UnorderedList,
        BlockType::OrderedList,
        BlockType::TaskList,
    ];

    fn all() -> impl Iterator<Item = Self> {
        Self::ALL.into_iter()
    }

    /// Block types that behave as code:
    /// * [`BlockType::RunnableCommand`]
    /// * [`BlockType::Code`]
    ///
    /// These types support multiple paragraphs and syntax highlighting, but not user-defined
    /// formatting. In the block insertion menu, these types are grouped together.
    fn code_block_types() -> impl Iterator<Item = Self> {
        [BlockType::RunnableCommand, BlockType::Code].into_iter()
    }

    /// Block types that behave as text (plain text, headings, and lists). These types support
    /// user-defined formatting. In the block insertion menu, these types are grouped together.
    fn text_block_types() -> impl Iterator<Item = Self> {
        Self::all().filter(|block_type| {
            *block_type != BlockType::Code && *block_type != BlockType::RunnableCommand
        })
    }

    fn icon(self) -> Icon {
        match self {
            BlockType::Text => Icon::TextBlock,
            BlockType::Header(_) => Icon::HeaderBlock,
            BlockType::RunnableCommand => Icon::RunnableCommandBlock,
            BlockType::Code => Icon::Code1,
            BlockType::UnorderedList => Icon::BulletedListBlock,
            BlockType::OrderedList => Icon::OrderedListBlock,
            BlockType::TaskList => Icon::TaskListBlock,
        }
    }

    fn icon_color(self, appearance: &Appearance) -> Option<Fill> {
        match self {
            BlockType::Text
            | BlockType::Header(_)
            | BlockType::UnorderedList
            | BlockType::OrderedList
            | BlockType::TaskList => Some(Fill::Solid(appearance.theme().ui_warning_color())),
            BlockType::RunnableCommand | BlockType::Code => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            BlockType::Text => "Text",
            BlockType::Header(size) => size.label(),
            BlockType::RunnableCommand => "Command",
            BlockType::UnorderedList => "Bulleted list",
            BlockType::OrderedList => "Numbered list",
            BlockType::Code => "Code",
            BlockType::TaskList => "To-do list",
        }
    }
}

/// The embedded item transformation for notebooks.
pub(super) fn notebook_embedded_item_conversion(
    mut mapping: serde_yaml::Mapping,
) -> Option<Arc<dyn EmbeddedItem>> {
    use serde_yaml::Value;
    match mapping.remove(&Value::String("id".to_string())) {
        Some(Value::String(hashed_id)) => Some(Arc::new(EmbeddedWorkflow::new(hashed_id))),
        _ => None,
    }
}

pub(crate) fn markdown_table_appearance(appearance: &Appearance) -> MarkdownTableAppearance {
    let theme = appearance.theme();
    MarkdownTableAppearance {
        border_color: internal_colors::neutral_4(theme),
        header_background: ColorU::transparent_black(),
        cell_background: ColorU::transparent_black(),
        alternate_row_background: None,
        text_color: internal_colors::text_sub(theme, theme.background()),
        header_text_color: internal_colors::text_main(theme, theme.background()),
        scrollbar_nonactive_thumb_color: theme.nonactive_ui_detail().into_solid(),
        scrollbar_active_thumb_color: theme.active_ui_detail().into_solid(),
        cell_padding: 12.,
        outer_border: false,
        column_dividers: false,
        row_dividers: true,
    }
}

pub(crate) fn markdown_table_style(
    appearance: &Appearance,
    font_family: FamilyId,
    font_size: f32,
) -> TableStyle {
    let table_appearance = markdown_table_appearance(appearance);
    TableStyle {
        border_color: table_appearance.border_color,
        header_background: table_appearance.header_background,
        cell_background: table_appearance.cell_background,
        alternate_row_background: table_appearance.alternate_row_background,
        text_color: table_appearance.text_color,
        header_text_color: table_appearance.header_text_color,
        scrollbar_nonactive_thumb_color: table_appearance.scrollbar_nonactive_thumb_color,
        scrollbar_active_thumb_color: table_appearance.scrollbar_active_thumb_color,
        font_family,
        font_size,
        cell_padding: table_appearance.cell_padding,
        outer_border: table_appearance.outer_border,
        column_dividers: table_appearance.column_dividers,
        row_dividers: table_appearance.row_dividers,
    }
}

/// Build [`RichTextStyles`] based on the current [`Appearance`].
pub fn rich_text_styles(appearance: &Appearance, font_settings: &FontSettings) -> RichTextStyles {
    let line_height_ratio = NOTEBOOK_LINE_HEIGHT_RATIO;
    let baseline_ratio = NOTEBOOK_BASELINE_RATIO;
    let theme = appearance.theme();
    let inline_font_color: ColorU = theme.terminal_colors().normal.red.into();
    let font_size = derived_notebook_font_size(font_settings);

    let base_text = ParagraphStyles {
        font_size,
        font_weight: Default::default(),
        line_height_ratio,
        font_family: appearance.ui_font_family(),
        text_color: theme.main_text_color(theme.background()).into_solid(),
        baseline_ratio,
        fixed_width_tab_size: None,
    };

    RichTextStyles {
        base_text,
        code_text: ParagraphStyles {
            font_family: appearance.monospace_font_family(),
            font_size,
            font_weight: Default::default(),
            line_height_ratio,
            text_color: theme.main_text_color(theme.background()).into_solid(),
            baseline_ratio,
            fixed_width_tab_size: Some(4),
        },
        code_background: theme.background().into(),
        embedding_background: theme.surface_2().into(),
        embedding_text: ParagraphStyles {
            font_size,
            font_weight: Default::default(),
            line_height_ratio,
            font_family: appearance.monospace_font_family(),
            text_color: theme.main_text_color(theme.surface_2()).into_solid(),
            baseline_ratio,
            fixed_width_tab_size: Some(4),
        },
        code_border: Border::all(1.).with_border_fill(theme.surface_3()),
        placeholder_color: appearance
            .theme()
            .hint_text_color(theme.background())
            .into_solid(),
        selection_fill: appearance.theme().text_selection_color().into(),
        cursor_fill: theme
            .cursor()
            .on_background(theme.background(), MinimumAllowedContrast::Text)
            .into(),
        inline_code_style: InlineCodeStyle {
            font_family: appearance.monospace_font_family(),
            background: theme.surface_3().into(),
            font_color: inline_font_color
                .on_background(theme.surface_3().into(), MinimumAllowedContrast::Text),
        },
        check_box_style: CheckBoxStyle {
            border_color: theme.foreground().into(),
            border_width: 2.,
            icon_path: CHECK_SVG_PATH,
            background: theme.accent().into(),
            hover_background: *HOVER_BACKGROUND_COLOR,
        },
        horizontal_rule_style: HorizontalRuleStyle {
            color: theme.surface_3().into(),
            rule_height: 3.,
        },
        broken_link_style: BrokenLinkStyle {
            icon_path: "bundled/svg/link-broken-02.svg",
            icon_color: theme.terminal_colors().normal.red.into(),
        },
        block_spacings: Default::default(),
        show_placeholder_text_on_empty_block: true,
        minimum_paragraph_height: Some(base_text.line_height()),
        cursor_width: 3.,
        highlight_urls: true,
        table_style: markdown_table_style(appearance, appearance.ui_font_family(), font_size),
    }
}

impl From<BlockType> for BufferBlockStyle {
    fn from(block_type: BlockType) -> Self {
        match block_type {
            BlockType::RunnableCommand => Self::CodeBlock {
                code_block_type: CodeBlockType::Shell,
            },
            BlockType::Text => Self::PlainText,
            BlockType::Header(header_size) => Self::Header { header_size },
            BlockType::UnorderedList => Self::UnorderedList {
                indent_level: ListIndentLevel::One,
            },
            BlockType::OrderedList => Self::ordered_list(ListIndentLevel::One),
            BlockType::Code => Self::CodeBlock {
                code_block_type: CodeBlockType::Code {
                    lang: CODE_BLOCK_DEFAULT_MARKDOWN_LANG.into(),
                },
            },
            BlockType::TaskList => Self::TaskList {
                indent_level: ListIndentLevel::One,
                complete: false,
            },
        }
    }
}

impl<'a> From<&'a ContentBlockType> for BlockType {
    fn from(block_type: &'a ContentBlockType) -> Self {
        match block_type {
            // TODO: Add support for block item here.
            ContentBlockType::Item(_) => BlockType::Text,
            ContentBlockType::Text(block_style) => Self::from(block_style),
        }
    }
}

impl<'a> From<&'a BufferBlockStyle> for BlockType {
    fn from(block_style: &'a BufferBlockStyle) -> Self {
        match block_style {
            BufferBlockStyle::CodeBlock { code_block_type } => match code_block_type {
                CodeBlockType::Shell => BlockType::RunnableCommand,
                CodeBlockType::Mermaid | CodeBlockType::Code { .. } => BlockType::Code,
            },
            BufferBlockStyle::PlainText => BlockType::Text,
            BufferBlockStyle::Header { header_size } => BlockType::Header(*header_size),
            BufferBlockStyle::UnorderedList { .. } => BlockType::UnorderedList,
            BufferBlockStyle::OrderedList { .. } => BlockType::OrderedList,
            BufferBlockStyle::TaskList { .. } => BlockType::TaskList,
            BufferBlockStyle::Table { .. } => BlockType::Text,
        }
    }
}

/// Wrapper around the shared [`Workflow`] type with additional context for workflows contained
/// within a notebook.
///
/// This may be a command block that's part of the notebook text, or an embedded Warp Drive workflow.
#[derive(Debug, Clone, PartialEq)]
pub struct NotebookWorkflow {
    /// Definition of the workflow itself.
    pub workflow: UserInput<Arc<WorkflowType>>,
    /// The source of the workflow, for attribution. If `None`, the workflow should be attributed
    /// to the parent notebook.
    pub source: Option<WorkflowSource>,
}

impl NotebookWorkflow {
    pub fn from_cloud_workflow(cloud_workflow: Box<CloudWorkflow>) -> Self {
        Self {
            source: Some(cloud_workflow.permissions.owner.into()),
            workflow: UserInput::new(Arc::new(WorkflowType::Cloud(cloud_workflow))),
        }
    }

    /// Extract the [`WorkflowType`], assigning a name using the given callback if needed.
    pub fn named_workflow<F: FnOnce() -> Option<String>>(&self, name: F) -> Arc<WorkflowType> {
        match &**self.workflow {
            WorkflowType::Notebook(workflow) if workflow.name().is_empty() => match name() {
                Some(name) => {
                    let mut workflow = workflow.clone();
                    workflow.set_name(name.as_str());
                    Arc::new(WorkflowType::Notebook(workflow))
                }
                None => (*self.workflow).clone(),
            },
            _ => (*self.workflow).clone(),
        }
    }
}
