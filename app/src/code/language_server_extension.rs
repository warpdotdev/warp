use lsp::{HoverContents, LspServerLogLevel, MarkupKind};
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use num_traits::SaturatingSub;
use string_offset::CharOffset;
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::WarpTheme;
use warp_core::ui::theme::color::internal_colors;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_editor::render::model::Decoration;
use warpui::elements::{
    Border, ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, Flex, FormattedTextElement, HighlightedHyperlink, Hoverable,
    MouseStateHandle, ParentElement, Radius, Rect, ScrollbarWidth,
};
use warpui::{AppContext, Element, SingletonEntity, ViewContext};

use super::editor::view::{CodeEditorRenderOptions, CodeEditorView};
use super::lsp_telemetry::LspTelemetryEvent;
use crate::code::local_code_editor::{
    HOVER_TOOLTIP_MAX_HEIGHT, HOVER_TOOLTIP_MAX_WIDTH, HoverContentSegment, LocalCodeEditorView,
    LspHoverState,
};
use crate::editor::InteractionState;

/// A processed diagnostic with its converted offset range.
/// Stored on LocalCodeEditorView and used for both decoration and hover display.
#[derive(Clone)]
pub struct ProcessedDiagnostic {
    /// The diagnostic message.
    pub message: String,
    /// The severity of the diagnostic.
    pub severity: lsp_types::DiagnosticSeverity,
    /// The start offset (0-based, for rendering).
    pub start: CharOffset,
    /// The end offset (0-based, for rendering).
    pub end: CharOffset,
}

enum PendingSection {
    Markdown(Vec<FormattedTextLine>),
    Code { language: String, code: String },
}

#[derive(Default)]
struct PendingSections {
    sections: Vec<PendingSection>,
    pending: Option<PendingSection>,
    active_line_break: bool,
}

impl PendingSections {
    fn push_formatted_line(&mut self, line: FormattedTextLine) {
        match line {
            FormattedTextLine::LineBreak
                if self.active_line_break
                    || matches!(self.pending, Some(PendingSection::Code { .. }) | None) => {}
            FormattedTextLine::HorizontalRule => {
                self.active_line_break = false;
                if let Some(section) = self.pending.take() {
                    self.sections.push(section);
                }
            }
            FormattedTextLine::CodeBlock(code_block) => {
                self.active_line_break = false;
                match self.pending.take() {
                    Some(pending @ PendingSection::Markdown(_)) => {
                        self.sections.push(pending);
                    }
                    Some(PendingSection::Code { mut code, language }) => {
                        if language == code_block.lang {
                            code.push('\n');
                            code.push_str(code_block.code.trim());

                            self.pending = Some(PendingSection::Code { code, language });
                            return;
                        }
                        self.sections.push(PendingSection::Code { code, language });
                    }
                    None => (),
                };
                self.pending = Some(PendingSection::Code {
                    code: code_block.code,
                    language: code_block.lang,
                })
            }
            other => {
                self.active_line_break = matches!(other, FormattedTextLine::LineBreak);
                match self.pending.take() {
                    Some(code @ PendingSection::Code { .. }) => {
                        self.sections.push(code);
                        self.pending = Some(PendingSection::Markdown(vec![other]));
                    }
                    Some(PendingSection::Markdown(mut markdown)) => {
                        markdown.push(other);
                        self.pending = Some(PendingSection::Markdown(markdown));
                    }
                    None => self.pending = Some(PendingSection::Markdown(vec![other])),
                }
            }
        }
    }

    fn flush(self, ctx: &mut ViewContext<LocalCodeEditorView>) -> Vec<HoverContentSegment> {
        let mut segments = Vec::new();
        for section in self.sections {
            match section {
                PendingSection::Markdown(text_lines) => {
                    segments.push(HoverContentSegment::Text(FormattedText::new(text_lines)))
                }
                PendingSection::Code { language, code } => segments.push(
                    LocalCodeEditorView::create_highlighted_code_fragment(language, code, ctx),
                ),
            }
        }

        if let Some(pending) = self.pending {
            match pending {
                PendingSection::Markdown(text_lines) => {
                    segments.push(HoverContentSegment::Text(FormattedText::new(text_lines)))
                }
                PendingSection::Code { language, code } => segments.push(
                    LocalCodeEditorView::create_highlighted_code_fragment(language, code, ctx),
                ),
            }
        }
        segments
    }
}

impl LocalCodeEditorView {
    /// Refresh diagnostics from the LSP server.
    /// This updates the cached processed diagnostics and creates decorations for the editor.
    pub(super) fn refresh_diagnostics(&mut self, ctx: &mut ViewContext<Self>) {
        // Update cached processed diagnostics.
        self.processed_diagnostics = self.compute_processed_diagnostics(ctx);

        // Convert processed diagnostics to decorations.
        let appearance = Appearance::as_ref(ctx);
        let error_color = appearance.theme().ui_error_color();
        let warning_color = appearance.theme().ui_warning_color();

        let decorations: Vec<Decoration> = self
            .processed_diagnostics
            .iter()
            .map(|diag| {
                let color = match diag.severity {
                    lsp_types::DiagnosticSeverity::ERROR => error_color,
                    lsp_types::DiagnosticSeverity::WARNING => warning_color,
                    _ => error_color, // Fallback, though we filter to only errors/warnings
                };
                Decoration::new(diag.start, diag.end).with_dashed_underline(color)
            })
            .collect();

        self.diagnostic_decorations = decorations;
        self.update_editor_decorations(ctx);
    }

    pub(super) fn clear_diagnostics(&mut self, ctx: &mut ViewContext<Self>) {
        self.processed_diagnostics.clear();
        self.diagnostic_decorations.clear();
        self.update_editor_decorations(ctx);
    }

    /// Update the editor's text decorations with diagnostic decorations.
    fn update_editor_decorations(&self, ctx: &mut ViewContext<Self>) {
        // Pass diagnostic decorations to the render state.
        let decorations = self.diagnostic_decorations.clone();
        self.editor.update(ctx, |editor, ctx| {
            editor.set_diagnostic_decorations(decorations, ctx);
        });
    }

    /// Compute processed diagnostics (errors and warnings) with their converted offset ranges.
    /// Returns an empty vec if LSP server is not available or there are no diagnostics.
    fn compute_processed_diagnostics(&self, ctx: &ViewContext<Self>) -> Vec<ProcessedDiagnostic> {
        let Some(lsp_server) = self.lsp_server.as_ref() else {
            return Vec::new();
        };
        let Some(file_path) = self.file_path() else {
            return Vec::new();
        };
        let Some(doc_diagnostics) = lsp_server
            .as_ref(ctx)
            .diagnostics_for_path(file_path)
            .ok()
            .flatten()
        else {
            return Vec::new();
        };

        // Only show diagnostics that match the current buffer version.
        let current_buffer_version = self.editor.as_ref(ctx).buffer_version(ctx).as_usize() as i32;
        let diag_count = doc_diagnostics.diagnostics.len();
        let diag_age_ms = doc_diagnostics.published_at.elapsed().as_millis();

        match doc_diagnostics.version {
            Some(version) if version != current_buffer_version => {
                lsp_server.as_ref(ctx).log_to_server_log(
                    LspServerLogLevel::Info,
                    format!(
                        "render: DROPPED (version mismatch) file={} render_version={current_buffer_version} diag_version={version} diag_count={diag_count} age_ms={diag_age_ms}",
                        file_path.display(),
                    ),
                );
                return Vec::new();
            }
            Some(version) => {
                lsp_server.as_ref(ctx).log_to_server_log(
                    LspServerLogLevel::Debug,
                    format!(
                        "render: OK file={} render_version={current_buffer_version} diag_version={version} diag_count={diag_count} age_ms={diag_age_ms}",
                        file_path.display(),
                    ),
                );
            }
            None => {
                lsp_server.as_ref(ctx).log_to_server_log(
                    LspServerLogLevel::Debug,
                    format!(
                        "render: UNVERSIONED file={} render_version={current_buffer_version} diag_count={diag_count} age_ms={diag_age_ms}",
                        file_path.display(),
                    ),
                );
            }
        }

        doc_diagnostics
            .diagnostics
            .iter()
            .filter_map(|diagnostic| {
                // Only include errors and warnings.
                let severity = diagnostic.severity?;
                if !matches!(
                    severity,
                    lsp_types::DiagnosticSeverity::ERROR | lsp_types::DiagnosticSeverity::WARNING
                ) {
                    return None;
                }

                // Convert LSP range to CharOffset range.
                let range: lsp::types::Range = diagnostic.range.into();
                let mut start_offset = self
                    .editor
                    .as_ref(ctx)
                    .lsp_location_to_offset(&range.start, ctx);
                let mut end_offset = self
                    .editor
                    .as_ref(ctx)
                    .lsp_location_to_offset(&range.end, ctx);

                // Handle zero-width ranges by expanding to at least 1 character.
                if start_offset >= end_offset {
                    end_offset = start_offset + CharOffset::from(1);
                }

                // Check if the diagnostic range only covers a newline character.
                // This happens for diagnostics like "missing semicolon" that point to
                // the end of a line. In this case, shift the range back to cover the
                // last character on the line instead, so it renders visibly.
                let is_single_char_range =
                    end_offset.saturating_sub(&start_offset) == CharOffset::from(1);
                if is_single_char_range {
                    let char_at_start = self.editor.as_ref(ctx).char_at(start_offset, ctx);
                    if let Some('\n') = char_at_start {
                        // Shift range back by 1 to cover the character before the newline.
                        if start_offset > CharOffset::from(1) {
                            start_offset = start_offset.saturating_sub(&CharOffset::from(1));
                            end_offset = end_offset.saturating_sub(&CharOffset::from(1));
                        }
                    }
                }

                // Convert to 0-based offsets (render offsets).
                let start = start_offset.saturating_sub(&CharOffset::from(1));
                let end = end_offset.saturating_sub(&CharOffset::from(1));

                Some(ProcessedDiagnostic {
                    message: diagnostic.message.clone(),
                    severity,
                    start,
                    end,
                })
            })
            .collect()
    }

    /// Get diagnostics at the given offset from the cached processed diagnostics.
    /// Returns a list of ProcessedDiagnostic for any diagnostics whose range contains the offset.
    /// The input offset and ProcessedDiagnostic ranges are both 0-based render offsets.
    pub(super) fn diagnostics_at_offset(&self, offset: CharOffset) -> Vec<ProcessedDiagnostic> {
        self.processed_diagnostics
            .iter()
            .filter(|diag| offset >= diag.start && offset < diag.end)
            .cloned()
            .collect()
    }

    /// Request hover information (documentation, type info) for a given offset.
    pub fn hover_for_offset(&mut self, offset: CharOffset, ctx: &mut ViewContext<Self>) {
        if matches!(self.lsp_hover_state, LspHoverState::None) {
            return;
        }

        let lsp_position = self
            .editor()
            .as_ref(ctx)
            .offset_to_lsp_position(offset, ctx);

        let Some(file_path) = self.file_path() else {
            return;
        };

        if self.lsp_server.is_none() {
            log::warn!("No LSP server available for hover");
            return;
        }

        let future = match self
            .lsp_server
            .as_ref()
            .unwrap()
            .as_ref(ctx)
            .hover(file_path.to_path_buf(), lsp_position)
        {
            Ok(future) => future,
            Err(e) => {
                log::warn!("Failed to call lsp.hover: {e}");
                return;
            }
        };

        let abort_handle = ctx
            .spawn(future, move |me, result, ctx| {
                // Get diagnostics at the hovered offset from cached processed diagnostics.
                // We always check for diagnostics, regardless of the LSP hover result.
                let diagnostics = me.diagnostics_at_offset(offset);

                // Extract hover range and contents from the LSP result (if available).
                let (hover_range, hover_contents) = match result {
                    Ok(Some(hover_result)) => (hover_result.range, Some(hover_result.contents)),
                    _ => (None, None),
                };

                // Create hover segments if we have non-empty contents.
                let segments = match hover_contents {
                    Some(contents) if !contents.is_empty() => {
                        me.create_hover_content_segments(contents, ctx)
                    }
                    _ => Vec::new(),
                };

                // Only show the hover tooltip if there's something to display.
                if segments.is_empty() && diagnostics.is_empty() {
                    me.lsp_hover_state.clear();
                } else {
                    let had_content = !segments.is_empty();
                    let had_diagnostics = !diagnostics.is_empty();
                    if let Some(server) = me.lsp_server.as_ref() {
                        send_telemetry_from_ctx!(
                            LspTelemetryEvent::HoverShown {
                                server_type: server.as_ref(ctx).server_name(),
                                had_content,
                                had_diagnostics,
                            },
                            ctx
                        );
                    }

                    let editor = me.editor().as_ref(ctx);

                    // Determine the offset range for positioning the tooltip.
                    let offset_range = match hover_range {
                        Some(range) => {
                            let start = editor.lsp_location_to_offset(&range.start, ctx);
                            let end = editor.lsp_location_to_offset(&range.end, ctx);
                            // Rendering range is 0-based instead of 1-based.
                            start.saturating_sub(&CharOffset::from(1))
                                ..end.saturating_sub(&CharOffset::from(1))
                        }
                        None => match editor.word_range_at_offset(offset, ctx) {
                            Some(range) => {
                                range.start.saturating_sub(&CharOffset::from(1))
                                    ..range.end.saturating_sub(&CharOffset::from(1))
                            }
                            None => offset..offset + 1,
                        },
                    };

                    me.lsp_hover_state = LspHoverState::Loaded {
                        segments,
                        diagnostics,
                        hovered_offset_range: offset_range,
                        scroll_state: ClippedScrollStateHandle::default(),
                        mouse_state: MouseStateHandle::default(),
                    };
                }
                ctx.notify();
            })
            .abort_handle();

        self.lsp_hover_state = LspHoverState::Loading(Some(abort_handle));
    }

    pub(super) fn create_highlighted_code_fragment(
        language: String,
        code: String,
        ctx: &mut ViewContext<Self>,
    ) -> HoverContentSegment {
        let view = ctx.add_typed_action_view(|ctx| {
            CodeEditorView::new(
                None,
                None,
                CodeEditorRenderOptions::new(VerticalExpansionBehavior::InfiniteHeight),
                ctx,
            )
            .with_can_show_diff_ui(false)
            .with_show_line_numbers(false)
        });

        view.update(ctx, |view, ctx| {
            view.set_show_current_line_highlights(false, ctx);
            view.set_interaction_state(InteractionState::Selectable, ctx);
            let state = InitialBufferState::plain_text(code.trim());
            view.reset(state, ctx);
            view.set_language_with_name(&language, ctx);
        });

        HoverContentSegment::CodeBlock { view }
    }

    /// Creates hover content segments from parsed FormattedText lines.
    /// Code blocks are converted to CodeEditorViews for syntax highlighting,
    /// while other content is grouped into FormattedText segments.
    /// Consecutive code blocks with the same language are merged into a single view.
    pub(super) fn create_hover_content_segments(
        &mut self,
        content: HoverContents,
        ctx: &mut ViewContext<Self>,
    ) -> Vec<HoverContentSegment> {
        let mut pending = PendingSections::default();

        for section in content.sections {
            let text = match section.kind {
                MarkupKind::Markdown => match markdown_parser::parse_markdown(&section.value) {
                    Ok(text) => text,
                    Err(_) => FormattedText::new([FormattedTextLine::Line(vec![
                        FormattedTextFragment::plain_text(section.value),
                    ])]),
                },
                MarkupKind::PlainText => FormattedText::new([FormattedTextLine::Line(vec![
                    FormattedTextFragment::plain_text(section.value),
                ])]),
            };

            for line in text.lines {
                pending.push_formatted_line(line);
            }
        }

        pending.flush(ctx)
    }

    /// Render the LSP hover tooltip if hover state is available.
    pub(super) fn render_hover_tooltip(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        let (segments, diagnostics, scroll_state, mouse_state) = match &self.lsp_hover_state {
            LspHoverState::Loaded {
                segments,
                diagnostics,
                scroll_state,
                mouse_state,
                ..
            } => (
                segments,
                diagnostics,
                scroll_state.clone(),
                mouse_state.clone(),
            ),
            _ => return None,
        };

        // Don't show tooltip if there's no content.
        if segments.is_empty() && diagnostics.is_empty() {
            return None;
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // Build content column with diagnostics first, then hover info.
        let mut content_column =
            Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        let mut is_first = true;

        // Render diagnostics first (if any).
        for diagnostic in diagnostics {
            if !is_first {
                content_column.add_child(Self::render_separator(theme));
            } else {
                is_first = false;
            }

            content_column.add_child(Self::render_diagnostic(diagnostic, appearance));
        }

        // Render hover info segments after diagnostics.
        for segment in segments {
            if !is_first {
                content_column.add_child(Self::render_separator(theme));
            } else {
                is_first = false;
            }
            match segment {
                HoverContentSegment::Text(formatted_text) => {
                    // Render text content using FormattedTextElement.
                    let text_element = FormattedTextElement::new(
                        formatted_text.clone(),
                        appearance.monospace_font_size(),
                        appearance.ui_font_family(),
                        appearance.monospace_font_family(),
                        theme.active_ui_text_color().into(),
                        HighlightedHyperlink::default(),
                    )
                    .finish();
                    content_column.add_child(text_element);
                }
                HoverContentSegment::CodeBlock { view, .. } => {
                    // Render code block using the embedded CodeEditorView.
                    let code_element = Container::new(ChildView::new(view).finish())
                        .with_padding_top(4.)
                        .with_horizontal_padding(8.)
                        .finish();
                    content_column.add_child(code_element);
                }
            }
        }

        // Make content scrollable if it exceeds max height.
        let scrollable_content = ClippedScrollable::vertical(
            scroll_state,
            content_column.finish(),
            ScrollbarWidth::Auto,
            theme.disabled_ui_text_color().into(),
            theme.active_ui_text_color().into(),
            warpui::elements::Fill::None,
        )
        .finish();

        let constrained_content = ConstrainedBox::new(scrollable_content)
            .with_width(HOVER_TOOLTIP_MAX_WIDTH)
            .with_max_height(HOVER_TOOLTIP_MAX_HEIGHT)
            .finish();

        let tooltip = Container::new(constrained_content)
            .with_horizontal_padding(8.)
            .with_vertical_padding(6.)
            .with_background(theme.background())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_border(Border::all(1.).with_border_fill(internal_colors::neutral_4(theme)))
            .finish();

        // Wrap in Hoverable so we can track whether the mouse is over the tooltip.
        // This is used by LocalCodeEditorView to avoid clearing hover state when
        // the mouse moves over the tooltip itself.
        let hoverable_tooltip = Hoverable::new(mouse_state, |_| tooltip).finish();

        Some(hoverable_tooltip)
    }

    /// Render a separator line between hover card sections.
    fn render_separator(theme: &WarpTheme) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Rect::new()
                    .with_background(internal_colors::neutral_2(theme))
                    .finish(),
            )
            .with_height(1.)
            .finish(),
        )
        .with_vertical_padding(4.)
        .finish()
    }

    /// Render a diagnostic message with severity prefix.
    fn render_diagnostic(
        diagnostic: &ProcessedDiagnostic,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        // Create the diagnostic text with bold severity prefix.
        let severity_text = match diagnostic.severity {
            lsp_types::DiagnosticSeverity::ERROR => "Error",
            lsp_types::DiagnosticSeverity::WARNING => "Warning",
            lsp_types::DiagnosticSeverity::INFORMATION => "Info",
            lsp_types::DiagnosticSeverity::HINT => "Hint",
            _ => "Diagnostic",
        };

        let text = FormattedText::new([FormattedTextLine::Line(vec![
            FormattedTextFragment::bold(format!("{severity_text}: ")),
            FormattedTextFragment::plain_text(&diagnostic.message),
        ])]);

        // Use error or warning color for the entire diagnostic text.
        let text_color = match diagnostic.severity {
            lsp_types::DiagnosticSeverity::ERROR => theme.ui_error_color(),
            lsp_types::DiagnosticSeverity::WARNING => theme.ui_warning_color(),
            _ => theme.active_ui_text_color().into_solid(),
        };

        FormattedTextElement::new(
            text,
            appearance.monospace_font_size(),
            appearance.ui_font_family(),
            appearance.monospace_font_family(),
            text_color,
            HighlightedHyperlink::default(),
        )
        .finish()
    }
}
