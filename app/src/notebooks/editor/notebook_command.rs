use std::borrow::Cow;
use std::mem;
use std::ops::Range;
use std::sync::Arc;

use async_channel::Sender;
use itertools::Itertools;
use lazy_static::lazy_static;
use markdown_parser::markdown_parser::CODE_BLOCK_DEFAULT_MARKDOWN_LANG;
use pathfinder_color::ColorU;
use string_offset::{ByteOffset, CharOffset};
use syntect::easy::HighlightLines;
use syntect::highlighting::{self, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use ui_components::lightbox::{LightboxImage, LightboxImageSource};
use warp_completer::signatures::CommandRegistry;
use warp_core::r#async::debounce;
use warp_editor::content::anchor::Anchor;
use warp_editor::content::buffer::{Buffer, BufferEvent, EditOrigin};
use warp_editor::content::mermaid_diagram::mermaid_asset_source;
use warp_editor::content::selection_model::BufferSelectionModel;
use warp_editor::content::text::{
    BlockType, BufferBlockStyle, CodeBlockType, CODE_BLOCK_DEFAULT_DISPLAY_LANG,
    CODE_BLOCK_SHELL_DISPLAY_LANG,
};
use warp_editor::editor::RunnableCommandModel;
use warp_util::user_input::UserInput;
use warpui::elements::{
    Align, Border, Container, CornerRadius, CrossAxisAlignment, Empty, Flex, MainAxisAlignment,
    MouseStateHandle, ParentElement, Radius, Shrinkable, Text,
};
use warpui::fonts::Properties;
use warpui::platform::Cursor;
use warpui::presenter::ChildView;
use warpui::r#async::SpawnedFutureHandle;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    AppContext, AssetProvider as _, Element, Entity, ModelAsRef, ModelContext, ModelHandle,
    SingletonEntity, ViewHandle, WeakModelHandle, WindowId,
};

use super::interaction_state_model::InteractionStateModel;
use super::keys::{custom_action_to_display, NotebookKeybindings};
use super::model::ChildModelHandle;
use super::view::EditorViewAction;
use super::{rich_text_styles, NotebookWorkflow};
use crate::appearance::Appearance;
use crate::completer::SessionAgnosticContext;
use crate::drive::workflows::arguments::ArgumentsState;
use crate::editor::InteractionState;
use crate::features::FeatureFlag;
use crate::menu::MenuItemFields;
use crate::notebooks::file::MarkdownDisplayMode;
use crate::notebooks::styles::block_footer_action_button;
use crate::notebooks::telemetry::{ActionEntrypoint, BlockInfo};
use crate::settings::FontSettings;
use crate::terminal::input::decorations::{
    parse_current_commands_and_tokens, ParsedTokenData, ParsedTokensSnapshot,
};
use crate::terminal::input::DEBOUNCE_INPUT_DECORATION_PERIOD;
use crate::themes::theme::{AnsiColorIdentifier, AnsiColors};
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons::Icon;
use crate::util::bindings::CustomAction;
use crate::util::color::{ContrastingColor, MinimumAllowedContrast};
use crate::view_components::dropdown::DropdownAction;
use crate::view_components::Dropdown;
use crate::workflows::workflow::Workflow;
use crate::workflows::WorkflowType;
use crate::workspace::WorkspaceAction;
use crate::ASSETS;

lazy_static! {
    static ref SUPPORTED_LANGUAGES: &'static [&'static str] = &[
        "Go",
        "Java",
        "C++",
        "C#",
        "JavaScript",
        "Python",
        "Ruby on Rails",
        "Rust",
        "SQL",
        "YAML",
        "JSON",
        "PHP",
    ];
}

#[derive(Default)]
struct MouseStateHandles {
    insert_button_state: MouseStateHandle,
    copy_button_state: MouseStateHandle,
    mermaid_raw_button_state: MouseStateHandle,
    mermaid_rendered_button_state: MouseStateHandle,
    mermaid_fullscreen_button_state: MouseStateHandle,
}

struct CachedHighlightKey {
    buffer_content: String,
    style: CodeBlockType,
}

struct CachedHighlightColors {
    key: CachedHighlightKey,
    colors: Vec<(Range<ByteOffset>, AnsiColorIdentifier)>,
}

impl CachedHighlightColors {
    fn matches_key(&self, buffer_content: &str, style: CodeBlockType) -> bool {
        self.key.buffer_content == buffer_content && self.key.style == style
    }
}

struct CodeHighlightResult {
    origin_text: String,
    colors: Vec<(Range<ByteOffset>, AnsiColorIdentifier)>,
}

enum CodeBlockDropdownIcon {
    Image(Icon),
    Tinted(Icon),
}

fn code_block_dropdown_icon(code_block_type: &CodeBlockType) -> CodeBlockDropdownIcon {
    match code_block_type {
        CodeBlockType::Mermaid => CodeBlockDropdownIcon::Image(Icon::MermaidLang),
        CodeBlockType::Code { lang } => match lang.as_str() {
            "Go" => CodeBlockDropdownIcon::Image(Icon::GoLang),
            "C++" => CodeBlockDropdownIcon::Image(Icon::CppLang),
            "JavaScript" => CodeBlockDropdownIcon::Image(Icon::JavaScriptLang),
            "Python" => CodeBlockDropdownIcon::Image(Icon::PythonLang),
            "Rust" => CodeBlockDropdownIcon::Image(Icon::RustLang),
            "SQL" => CodeBlockDropdownIcon::Image(Icon::SqlLang),
            "JSON" => CodeBlockDropdownIcon::Image(Icon::JsonLang),
            "PHP" => CodeBlockDropdownIcon::Image(Icon::PhpLang),
            "Kotlin" => CodeBlockDropdownIcon::Image(Icon::KotlinLang),
            "PowerShell" => CodeBlockDropdownIcon::Tinted(Icon::Powershell),
            _ => CodeBlockDropdownIcon::Tinted(Icon::Code1),
        },
        CodeBlockType::Shell => CodeBlockDropdownIcon::Tinted(Icon::Terminal),
    }
}

/// Runnable command behavior for notebooks.
pub struct NotebookCommand {
    start: Anchor,
    end: Anchor,
    interaction_state: ModelHandle<InteractionStateModel>,
    content: ModelHandle<Buffer>,
    selection_model: ModelHandle<BufferSelectionModel>,
    mouse_state_handles: MouseStateHandles,
    is_selected: bool,
    block_type_dropdown: ViewHandle<Dropdown<EditorViewAction>>,
    /// Display mode for this Mermaid block (Raw or Rendered).
    pub mermaid_display_mode: MarkdownDisplayMode,

    #[cfg_attr(test, allow(dead_code))]
    debounce_highlighting_tx: Sender<()>,
    syntax_highlighting_handle: Option<SpawnedFutureHandle>,
    cached_highlight_delta: Option<CachedHighlightColors>,

    syntax_config: Option<(SyntaxSet, Theme)>,

    handle: WeakModelHandle<Self>,
}

impl NotebookCommand {
    /// Create a new `NotebookCommand` model to back the runnable command between `start` and `end`.
    pub fn new(
        start: CharOffset,
        end: CharOffset,
        interaction_state: ModelHandle<InteractionStateModel>,
        content: ModelHandle<Buffer>,
        selection_model: ModelHandle<BufferSelectionModel>,
        rte_window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let current_block_style =
            NotebookCommand::block_type_to_code_type(content.as_ref(ctx).block_type_at_point(end));
        let (start, end) = selection_model.update(ctx, |selection_model, ctx| {
            (
                selection_model.anchor(start, ctx),
                selection_model.anchor(end, ctx),
            )
        });

        let block_type_dropdown = ctx.add_typed_action_view(rte_window_id, |ctx| {
            let mut dropdown = Dropdown::new(ctx);

            dropdown.set_top_bar_max_width(96.);
            dropdown.set_menu_width(160., ctx);

            dropdown.set_rich_items(
                CodeBlockType::all().map(|code_block_type| {
                    let mut item = MenuItemFields::new(code_block_type.to_string())
                        .with_on_select_action(DropdownAction::select_action_and_close(
                            EditorViewAction::CodeBlockTypeSelectedAtOffset {
                                code_block_type: code_block_type.clone(),
                                start_anchor: start.clone(),
                            },
                        ));
                    item = match code_block_dropdown_icon(&code_block_type) {
                        CodeBlockDropdownIcon::Image(icon) => item.with_image_icon(icon.into()),
                        CodeBlockDropdownIcon::Tinted(icon) => item.with_icon(icon),
                    };
                    item.into_item()
                }),
                ctx,
            );

            let current_dropdown_selection = match &current_block_style {
                CodeBlockType::Shell => CODE_BLOCK_SHELL_DISPLAY_LANG,
                CodeBlockType::Mermaid => "Mermaid",
                CodeBlockType::Code { lang } if lang == "text" => CODE_BLOCK_DEFAULT_DISPLAY_LANG,
                CodeBlockType::Code { lang } => lang,
            };
            dropdown.set_selected_by_name(current_dropdown_selection, ctx);

            dropdown
        });

        let syntax_config = {
            let ps = SyntaxSet::load_defaults_newlines();
            if let Ok(asset) = ASSETS.get("bundled/syntax_theme/base16.tmTheme") {
                let mut cursor = std::io::Cursor::new(asset);
                match ThemeSet::load_from_reader(&mut cursor) {
                    Ok(theme) => Some((ps, theme)),
                    Err(e) => {
                        log::debug!("Failed to load theme set from asset: {e}");
                        None
                    }
                }
            } else {
                None
            }
        };

        ctx.subscribe_to_model(&content, Self::on_buffer_content_updated);

        let (debounce_highlighting_tx, debounce_highlighting_rx) = async_channel::unbounded();
        let _ = ctx.spawn_stream_local(
            debounce(DEBOUNCE_INPUT_DECORATION_PERIOD, debounce_highlighting_rx),
            |me, _, ctx| me.highlight_syntax(ctx),
            |_me, _ctx| {},
        );

        let mut command = Self {
            start,
            end,
            interaction_state,
            content,
            selection_model,
            mouse_state_handles: Default::default(),
            is_selected: false,
            block_type_dropdown,
            mermaid_display_mode: MarkdownDisplayMode::Raw,
            syntax_highlighting_handle: None,
            cached_highlight_delta: None,
            debounce_highlighting_tx,
            syntax_config,
            handle: ctx.handle(),
        };

        command.highlight_syntax(ctx);
        command
    }

    fn block_type_to_code_type(block_type: BlockType) -> CodeBlockType {
        match block_type {
            BlockType::Text(BufferBlockStyle::CodeBlock {
                code_block_type: CodeBlockType::Shell,
            }) => CodeBlockType::Shell,
            BlockType::Text(BufferBlockStyle::CodeBlock {
                code_block_type: CodeBlockType::Mermaid,
            }) => CodeBlockType::Mermaid,
            BlockType::Text(BufferBlockStyle::CodeBlock {
                code_block_type: CodeBlockType::Code { lang },
            }) if SUPPORTED_LANGUAGES.contains(&lang.as_str()) => CodeBlockType::Code { lang },
            BlockType::Text(BufferBlockStyle::CodeBlock { .. }) => CodeBlockType::Code {
                lang: CODE_BLOCK_DEFAULT_MARKDOWN_LANG.to_string(),
            },
            _ => Default::default(),
        }
    }

    #[cfg(test)]
    pub fn start_anchor(&self) -> Anchor {
        self.start.clone()
    }

    // Returns the CodeBlockType of this command
    fn code_block_type(&self, ctx: &AppContext) -> CodeBlockType {
        if let Some(offset) = self.end_offset(ctx) {
            NotebookCommand::block_type_to_code_type(
                self.content.as_ref(ctx).block_type_at_point(offset),
            )
        } else {
            Default::default()
        }
    }

    #[cfg(test)]
    pub fn syntax_highlighting_handle(&self) -> Option<SpawnedFutureHandle> {
        self.syntax_highlighting_handle.clone()
    }

    pub fn highlight_syntax(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(handle) = self.syntax_highlighting_handle.take() {
            handle.abort_handle().abort();
        }

        let success = self.try_apply_cached_highlighting(ctx);
        if success {
            return;
        }

        let code_block_type = self.code_block_type(ctx);
        let Some(buffer_text) = self.command(ctx) else {
            return;
        };

        match code_block_type {
            CodeBlockType::Shell => {
                let completion_context =
                    SessionAgnosticContext::new(CommandRegistry::global_instance());
                self.syntax_highlighting_handle = Some(ctx.spawn(
                    async move {
                        parse_current_commands_and_tokens(buffer_text, &completion_context).await
                    },
                    |notebook_command, parsed_tokens, ctx| {
                        notebook_command.update_buffer_with_parsed_tokens(parsed_tokens, ctx);
                    },
                ));
            }
            CodeBlockType::Mermaid => (),
            // Skip highlighting for default code.
            CodeBlockType::Code { lang } if lang == "text" => (),
            CodeBlockType::Code { lang } => {
                let Some((syntax_set, syntax_theme)) = self.syntax_config.clone() else {
                    return;
                };

                self.syntax_highlighting_handle = Some(ctx.spawn(
                    parse_code_into_style_ranges(buffer_text, lang, syntax_set, syntax_theme),
                    |notebook_command, result, ctx| {
                        notebook_command.update_buffer_with_parsed_code_syntax(result, ctx);
                    },
                ));
            }
        }
    }

    fn update_buffer_with_parsed_code_syntax(
        &mut self,
        highlight_result: Option<CodeHighlightResult>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(highlight_result) = highlight_result else {
            return;
        };

        self.maybe_apply_highlighting(
            CachedHighlightKey {
                buffer_content: highlight_result.origin_text,
                style: self.code_block_type(ctx),
            },
            highlight_result.colors,
            ctx,
        );
    }

    fn update_buffer_with_parsed_tokens(
        &mut self,
        parsed_tokens: ParsedTokensSnapshot,
        ctx: &mut ModelContext<Self>,
    ) {
        let colors = parsed_token_to_color_style_ranges(parsed_tokens.parsed_tokens);

        self.maybe_apply_highlighting(
            CachedHighlightKey {
                buffer_content: parsed_tokens.buffer_text,
                style: CodeBlockType::Shell,
            },
            colors,
            ctx,
        );
    }

    pub fn try_apply_cached_highlighting(&self, ctx: &mut ModelContext<Self>) -> bool {
        let code_block_type = self.code_block_type(ctx);
        let Some(buffer_text) = self.command(ctx) else {
            return false;
        };
        match &self.cached_highlight_delta {
            // If the command block content matches our cache, simply update with the cache.
            Some(cache) if cache.matches_key(&buffer_text, code_block_type) => {
                if let Some(block_start) = self.start_offset(ctx) {
                    self.apply_highlighting_to_buffer(&cache.colors, block_start, ctx)
                }
                true
            }
            _ => false,
        }
    }

    /// Write syntax highlighting colors into the buffer and cache them with the given key. If the
    /// key does not match the buffer state, or the backing content range has been unstyled, the
    /// highlighting is discarded.
    fn maybe_apply_highlighting(
        &mut self,
        key: CachedHighlightKey,
        colors: Vec<(Range<ByteOffset>, AnsiColorIdentifier)>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(buffer_text) = self.command(ctx) else {
            return;
        };

        // If the command text has changed from when we parsed it, discard the parsing result.
        if buffer_text != key.buffer_content {
            return;
        }

        let Some(block_start) = self.start_offset(ctx) else {
            return;
        };

        // If the text range is no longer a code block, do not try to highlight it.
        if !matches!(
            self.content
                .as_ref(ctx)
                .block_type_at_point(block_start + 1),
            BlockType::Text(BufferBlockStyle::CodeBlock { .. })
        ) {
            return;
        }

        self.apply_highlighting_to_buffer(&colors, block_start, ctx);
        self.cached_highlight_delta = Some(CachedHighlightColors { key, colors });
    }

    fn apply_highlighting_to_buffer(
        &self,
        colors: &[(Range<ByteOffset>, AnsiColorIdentifier)],
        block_start: CharOffset,
        ctx: &mut ModelContext<Self>,
    ) {
        let appearance = Appearance::as_ref(ctx);
        let font_settings = FontSettings::as_ref(ctx);
        let terminal_colors_normal = appearance.theme().terminal_colors().normal.to_owned();
        let background_color = rich_text_styles(appearance, font_settings)
            .code_background
            .start_color();

        let transformed_colors =
            transform_ansi_color_to_solid_color(colors, &terminal_colors_normal, background_color);

        self.content.update(ctx, |buffer, ctx| {
            buffer.color_code_block_ranges(
                block_start + 1,
                &transformed_colors,
                self.selection_model.clone(),
                ctx,
            );
        });
    }

    fn on_buffer_content_updated(
        &mut self,
        _: ModelHandle<Buffer>,
        event: &BufferEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        // If the buffer changes, check to see if we should update the dropdown
        match event {
            BufferEvent::ContentChanged { origin, delta, .. }
                if *origin != EditOrigin::SystemEdit =>
            {
                let code_block_type = self.code_block_type(ctx);
                self.block_type_dropdown.update(ctx, |dropdown, ctx| {
                    dropdown.set_selected_by_name(code_block_type.to_string(), ctx)
                });
                let replacement_offset = &delta.old_offset;

                let Some(start_offset) = self.start_offset(ctx) else {
                    return;
                };

                if !matches!(
                    self.content
                        .as_ref(ctx)
                        .block_type_at_point(start_offset + 1),
                    BlockType::Text(BufferBlockStyle::CodeBlock { .. })
                ) {
                    return;
                }

                let Some(end_offset) = self.end_offset(ctx) else {
                    return;
                };

                // If the replacement range overlaps with command block range, regenerate the highlight.
                if start_offset <= replacement_offset.end && end_offset >= replacement_offset.start
                {
                    // In tests, run syntax highlighting immediately.
                    // TODO(ben): This is another case where mock timers in tests would be
                    // helpful.
                    #[cfg(test)]
                    self.highlight_syntax(ctx);
                    #[cfg(not(test))]
                    let _ = self.debounce_highlighting_tx.try_send(());
                }
            }
            _ => (),
        };
        ctx.notify();
    }

    /// The offset of this command's start marker.
    pub fn start_offset(&self, ctx: &impl ModelAsRef) -> Option<CharOffset> {
        self.selection_model.as_ref(ctx).resolve_anchor(&self.start)
    }

    /// The offset of this command's end marker.
    pub fn end_offset(&self, ctx: &impl ModelAsRef) -> Option<CharOffset> {
        self.selection_model.as_ref(ctx).resolve_anchor(&self.end)
    }

    /// The current text of this command.
    pub fn command(&self, ctx: &impl ModelAsRef) -> Option<String> {
        let start = self.start_offset(ctx)?;
        let end = self.end_offset(ctx)?;
        // Add 1 to start because it refers to the start marker offset.
        Some(
            self.content
                .as_ref(ctx)
                .text_in_range(start + 1..end)
                .into_string(),
        )
    }

    pub fn is_dropdown_focused(&self, ctx: &AppContext) -> bool {
        self.block_type_dropdown.as_ref(ctx).is_focused(ctx)
    }

    /// Whether or not this block contains the text cursor
    pub fn contains_cursor(&self, ctx: &impl ModelAsRef) -> bool {
        let cursor = self.selection_model.as_ref(ctx).first_selection_head();
        // Subtract one to get to the start marker of the block
        let block_start = self.content.as_ref(ctx).block_or_line_start(cursor) - 1;
        if let Some(start_offset) = self.start_offset(ctx) {
            start_offset == block_start
        } else {
            false
        }
    }

    /// Returns whether or not we should display the dropdown selector for this block. Essentially, we want to display
    /// it if the editor if the user has the command selected, or they are typing in it.
    fn should_display_block_type_dropdown(
        &self,
        editor_is_focused: bool,
        ctx: &AppContext,
    ) -> bool {
        // If we are in view mode or the editor is not focused, return false
        if !matches!(
            self.interaction_state.as_ref(ctx).interaction_state(),
            InteractionState::Editable
        ) || !editor_is_focused
        {
            return false;
        }

        // If this block is selected, return true
        if self.is_selected() {
            return true;
        }

        // If this block contains the cursor, and another block is not selected, return true
        if self.contains_cursor(ctx) && !self.interaction_state.as_ref(ctx).is_block_selected() {
            return true;
        }

        false
    }

    /// Whether this block is selected.
    pub fn is_selected(&self) -> bool {
        self.is_selected
    }

    /// Set whether or not this block is selected.
    pub fn set_selected(&mut self, selected: bool) -> bool {
        mem::replace(&mut self.is_selected, selected)
    }

    /// Promotes this notebook command into a [`Workflow`]. If the workflow is anonymous, the
    /// containing `NotebookView` fills in its title.
    pub fn to_workflow(&self, ctx: &AppContext) -> Option<NotebookWorkflow> {
        let command = self.command(ctx)?;
        let args_state = ArgumentsState::for_command_workflow(&Default::default(), command.clone());
        // TODO: Once notebook workflows have their own metadata, we can populate the title here.
        let workflow = Workflow::new(String::new(), command).with_arguments(args_state.arguments);
        Some(NotebookWorkflow {
            workflow: UserInput::new(Arc::new(WorkflowType::Notebook(workflow))),
            source: None,
        })
    }
}

impl Entity for NotebookCommand {
    type Event = ();
}

impl RunnableCommandModel for NotebookCommand {
    fn render_block_footer(&self, editor_is_focused: bool, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);
        let mut model = self.handle.clone();

        // Get the CodeBlockType at the end offset for the NotebookCommand
        // We would expect the BlockStyle at the offset to be a CodeBlock
        let block_style = self.code_block_type(ctx);

        let mut footer = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::End);

        if self.should_display_block_type_dropdown(editor_is_focused, ctx) {
            footer.add_child(ChildView::new(&self.block_type_dropdown).finish());
        } else {
            footer.add_child(
                Container::new(
                    Text::new_inline(
                        self.code_block_type(ctx).to_string(),
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_style(Properties {
                        weight: warpui::fonts::Weight::Light,
                        ..Default::default()
                    })
                    .with_color(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().background())
                            .into(),
                    )
                    .finish(),
                )
                .with_vertical_padding(11.)
                .finish(),
            )
        }
        if matches!(block_style, CodeBlockType::Mermaid)
            && FeatureFlag::MarkdownMermaid.is_enabled()
        {
            let is_raw = matches!(self.mermaid_display_mode, MarkdownDisplayMode::Raw);
            let start_anchor_raw = self.start.clone();
            let start_anchor_rendered = self.start.clone();
            let tooltip_builder_raw = appearance.ui_builder().clone();
            let tooltip_builder_rendered = appearance.ui_builder().clone();

            let active_highlight = UiComponentStyles {
                background: Some(appearance.theme().surface_3().into()),
                ..Default::default()
            };

            let raw_button = icon_button(
                appearance,
                Icon::Code1,
                is_raw,
                self.mouse_state_handles.mermaid_raw_button_state.clone(),
            )
            .with_active_styles(active_highlight)
            .with_tooltip(move || {
                tooltip_builder_raw
                    .tool_tip("Raw".to_string())
                    .build()
                    .finish()
            })
            .build()
            .with_cursor(Cursor::Arrow)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(EditorViewAction::MermaidDisplayModeSelected {
                    start_anchor: start_anchor_raw.clone(),
                    mode: MarkdownDisplayMode::Raw,
                });
            })
            .finish();

            let rendered_button = icon_button(
                appearance,
                Icon::Dataflow04,
                !is_raw,
                self.mouse_state_handles
                    .mermaid_rendered_button_state
                    .clone(),
            )
            .with_active_styles(active_highlight)
            .with_tooltip(move || {
                tooltip_builder_rendered
                    .tool_tip("Rendered".to_string())
                    .build()
                    .finish()
            })
            .build()
            .with_cursor(Cursor::Arrow)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(EditorViewAction::MermaidDisplayModeSelected {
                    start_anchor: start_anchor_rendered.clone(),
                    mode: MarkdownDisplayMode::Rendered,
                });
            })
            .finish();

            footer.add_child(
                Container::new(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_spacing(2.)
                        .with_child(raw_button)
                        .with_child(rendered_button)
                        .finish(),
                )
                .with_uniform_padding(2.)
                .with_margin_left(8.)
                .with_background(appearance.theme().background())
                .with_border(Border::all(1.).with_border_fill(appearance.theme().surface_3()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .finish(),
            );

            if !is_raw {
                let fullscreen_model = self.handle.clone();
                footer.add_child(
                    Align::new(
                        block_footer_action_button(
                            appearance,
                            Icon::Maximize,
                            self.mouse_state_handles
                                .mermaid_fullscreen_button_state
                                .clone(),
                            "Open full screen",
                            None,
                        )
                        .on_click(move |ctx, app, _| {
                            if let Some(command_model) = fullscreen_model.upgrade(app) {
                                if let Some(source) = command_model.as_ref(app).command(app) {
                                    if !source.trim().is_empty() {
                                        ctx.dispatch_typed_action(WorkspaceAction::OpenLightbox {
                                            images: vec![LightboxImage {
                                                source: LightboxImageSource::Resolved {
                                                    asset_source: mermaid_asset_source(&source),
                                                },
                                                description: None,
                                            }],
                                            initial_index: 0,
                                        });
                                    }
                                }
                            }
                        })
                        .finish(),
                    )
                    .right()
                    .finish(),
                );
            }
        }
        footer.add_child(Shrinkable::new(1.0, Empty::new().finish()).finish());
        footer.add_child(
            Align::new(
                block_footer_action_button(
                    appearance,
                    Icon::Copy,
                    self.mouse_state_handles.copy_button_state.clone(),
                    "Copy",
                    custom_action_to_display(CustomAction::Copy),
                )
                .on_click(move |ctx, app, _| {
                    if let Some(command_model) = model.upgrade(app) {
                        if let Some(block_content) = command_model.as_ref(app).command(app) {
                            ctx.dispatch_typed_action(EditorViewAction::CopyTextToClipboard {
                                text: UserInput::new(block_content.trim()),
                                block: BlockInfo::CodeBlock,
                                entrypoint: ActionEntrypoint::Button,
                            });
                        }
                    }
                })
                .finish(),
            )
            .right()
            .finish(),
        );
        if matches!(block_style, CodeBlockType::Shell) {
            model = self.handle.clone();
            footer.add_child(
                Align::new(
                    block_footer_action_button(
                        appearance,
                        Icon::TerminalInput,
                        self.mouse_state_handles.insert_button_state.clone(),
                        "Run in terminal",
                        NotebookKeybindings::as_ref(ctx).run_commands_keybinding(),
                    )
                    .on_click(move |ctx, app, _| {
                        if let Some(command_model) = model.upgrade(app) {
                            if let Some(workflow) = command_model.as_ref(app).to_workflow(app) {
                                ctx.dispatch_typed_action(EditorViewAction::RunWorkflow(workflow));
                            }
                        }
                    })
                    .finish(),
                )
                .right()
                .finish(),
            );
        }
        footer.finish()
    }

    fn border(&self, app: &AppContext) -> Option<Border> {
        if self.is_selected {
            let border_fill = Appearance::as_ref(app).theme().accent();
            Some(Border::all(3.).with_border_fill(border_fill))
        } else {
            None
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ChildModelHandle for ModelHandle<NotebookCommand> {
    fn start_offset(&self, app: &AppContext) -> Option<CharOffset> {
        self.as_ref(app).start_offset(app)
    }

    fn end_offset(&self, app: &AppContext) -> Option<CharOffset> {
        self.as_ref(app).end_offset(app)
    }

    fn selectable(&self, _: &AppContext) -> bool {
        true
    }

    fn executable_workflow(&self, app: &AppContext) -> Option<NotebookWorkflow> {
        self.as_ref(app).to_workflow(app)
    }

    fn executable_command<'a>(&'a self, app: &'a AppContext) -> Option<Cow<'a, str>> {
        self.as_ref(app).command(app).map(Into::into)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn selected(&self, app: &AppContext) -> bool {
        self.as_ref(app).is_selected
    }

    fn set_selected(&self, selected: bool, ctx: &mut AppContext) -> bool {
        self.update(ctx, |model, _ctx| model.set_selected(selected))
    }

    fn clone_boxed(&self) -> Box<dyn ChildModelHandle> {
        Box::new(self.clone())
    }
}

// Parse code into style ranges based on the current ANSI color and language.
async fn parse_code_into_style_ranges(
    buffer_text: String,
    language: String,
    syntax_set: SyntaxSet,
    theme: Theme,
) -> Option<CodeHighlightResult> {
    // Find the syntax corresponding to the input language.
    let syntax = syntax_set.find_syntax_by_name(&language)?;

    let mut h = HighlightLines::new(syntax, &theme);
    let mut runs = Vec::new();

    let mut byte_offset = 0;
    for line in LinesWithEndings::from(&buffer_text) {
        let ranges = h.highlight_line(line, &syntax_set).ok()?;

        for (text_style, content) in ranges {
            let text_color = text_style.foreground;
            let text_len = content.len();

            if let Some(ansi_color) = to_ansi_color(text_color) {
                runs.push((
                    ByteOffset::from(byte_offset)..ByteOffset::from(byte_offset + text_len),
                    ansi_color,
                ));
            }

            byte_offset += text_len;
        }
    }

    Some(CodeHighlightResult {
        origin_text: buffer_text,
        colors: runs,
    })
}

// We use base16 theme here so the colors could translate fully to terminal ANSI color.
pub fn to_ansi_color(color: highlighting::Color) -> Option<AnsiColorIdentifier> {
    match color.r {
        0x00 => Some(AnsiColorIdentifier::Black),
        0x01 => Some(AnsiColorIdentifier::Red),
        0x02 => Some(AnsiColorIdentifier::Green),
        0x03 => Some(AnsiColorIdentifier::Yellow),
        0x04 => Some(AnsiColorIdentifier::Blue),
        0x05 => Some(AnsiColorIdentifier::Magenta),
        0x06 => Some(AnsiColorIdentifier::Cyan),
        0x07 => Some(AnsiColorIdentifier::White),
        _ => None,
    }
}

pub fn parsed_token_to_color_style_ranges(
    parsed_tokens: Vec<ParsedTokenData>,
) -> Vec<(Range<ByteOffset>, AnsiColorIdentifier)> {
    let mut colors = Vec::new();

    for token_data in parsed_tokens {
        let token_description = token_data.token_description.clone();
        if let Some(description) = token_description {
            let token_syntax_color: AnsiColorIdentifier =
                description.suggestion_type.to_name().into();

            let style_byte_offset_start = ByteOffset::from(token_data.token.span.start());
            let style_byte_offset_end = ByteOffset::from(token_data.token.span.end());

            colors.push((
                style_byte_offset_start..style_byte_offset_end,
                token_syntax_color,
            ))
        }
    }
    colors
}

pub fn transform_ansi_color_to_solid_color(
    colors: &[(Range<ByteOffset>, AnsiColorIdentifier)],
    terminal_colors_normal: &AnsiColors,
    background_color: ColorU,
) -> Vec<(Range<ByteOffset>, ColorU)> {
    colors
        .iter()
        .map(|(range, identifier)| {
            let foreground_color: ColorU =
                (*identifier).to_ansi_color(terminal_colors_normal).into();
            (
                range.clone(),
                foreground_color.on_background(background_color, MinimumAllowedContrast::Text),
            )
        })
        .collect_vec()
}
