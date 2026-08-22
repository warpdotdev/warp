//! Reusable read-only code block for TUI Markdown surfaces.
//!
//! The view owns a char-cell [`CodeEditorModel`], translates its syntax
//! decorations into [`TuiEditorElement`] character overlays, and falls back to
//! lightweight text for pathological inputs.

use rangemap::RangeSet;
use string_offset::CharOffset;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::content::version::BufferVersion;
use warp_editor::model::CoreEditorModel;
use warpui_core::elements::tui::{Color, TuiElement, TuiFlex, TuiParentElement, TuiStyle, TuiText};
use warpui_core::{AppContext, Entity, ModelHandle, TuiView, ViewContext};

use crate::editor_element::{TuiEditorElement, TuiEditorStyles};
use crate::tui_builder::TuiUiBuilder;

const MAX_HIGHLIGHT_BYTES: usize = 256 * 1024;
const MAX_CODE_LINES: usize = 5_000;
const TRUNCATION_NOTICE: &str = "… code block truncated …";

/// Events emitted to the Markdown-owning parent.
pub(crate) enum TuiCodeBlockViewEvent {
    LayoutChanged,
    SyntaxUpdated,
}

/// Persistent payload identity for one code child.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TuiCodeBlockPayload {
    pub code: String,
    pub language: Option<String>,
}

impl TuiCodeBlockPayload {
    pub(crate) fn new(code: impl Into<String>, language: Option<String>) -> Self {
        Self {
            code: code.into(),
            language,
        }
    }
}

/// One editor-backed code block retained across parent redraws.
pub(crate) struct TuiCodeBlockView {
    editor: ModelHandle<CodeEditorModel>,
    payload: TuiCodeBlockPayload,
    expected_syntax_version: Option<BufferVersion>,
    text_overrides: Vec<(std::ops::Range<CharOffset>, TuiStyle)>,
    fallback_text: Option<String>,
}

impl TuiCodeBlockView {
    pub(crate) fn new(payload: TuiCodeBlockPayload, ctx: &mut ViewContext<Self>) -> Self {
        let editor = Self::create_editor(ctx);
        let mut view = Self {
            editor,
            payload: TuiCodeBlockPayload::new(String::new(), None),
            expected_syntax_version: None,
            text_overrides: Vec::new(),
            fallback_text: None,
        };
        view.sync(payload, ctx);
        view
    }

    fn create_editor(ctx: &mut ViewContext<Self>) -> ModelHandle<CodeEditorModel> {
        let editor = ctx.add_model(|ctx| CodeEditorModel::new_tui(0, ctx));
        ctx.subscribe_to_model(&editor, |me, source, event, ctx| {
            // A language change replaces the editor model. Events already
            // queued by the old parser must not style the replacement.
            if source.id() != me.editor.id() {
                return;
            }
            match event {
                CodeEditorModelEvent::SyntaxHighlightingUpdated => {
                    me.refresh_highlights(ctx);
                    ctx.emit(TuiCodeBlockViewEvent::SyntaxUpdated);
                    ctx.notify();
                }
                CodeEditorModelEvent::LayoutInvalidated => {
                    ctx.emit(TuiCodeBlockViewEvent::LayoutChanged);
                    ctx.notify();
                }
                CodeEditorModelEvent::ContentChanged { .. }
                | CodeEditorModelEvent::SelectionChanged
                | CodeEditorModelEvent::DiffUpdated
                | CodeEditorModelEvent::UnifiedDiffComputed(_)
                | CodeEditorModelEvent::ViewportUpdated(_)
                | CodeEditorModelEvent::InteractionStateChanged
                | CodeEditorModelEvent::DelayedRenderingFlushed => {}
                #[cfg(windows)]
                CodeEditorModelEvent::WindowsCtrlC { .. } => {}
            }
        });
        editor
    }

    /// Updates the retained child only when its code or language changes.
    pub(crate) fn sync(
        &mut self,
        payload: TuiCodeBlockPayload,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if self.payload == payload {
            return false;
        }

        let language_changed = self.payload.language != payload.language;
        // Capture the previous code before the payload is replaced so we can
        // decide whether the retained overrides are still valid.
        let prev_code = self.payload.code.clone();
        self.payload = payload;
        // A language change completely invalidates the prior highlights (the
        // token kinds no longer match), so clear them immediately.  For a
        // code-only streaming update that is a strict *append* (the new code
        // starts with the entire previous code), we keep overrides for
        // complete tokens in the interior so those tokens remain highlighted
        // while the new parse is in flight — avoiding a visible flash of
        // fully-unstyled text.  We drop overrides that end at or beyond the
        // previous code boundary because that token may still be growing:
        // e.g. `for` (override [0,3)) could be extending to `format`, and
        // retaining the override would color only the stale `for` prefix of
        // the now-longer identifier — precisely the intra-token miscoloring
        // this fix addresses.  If the code is *rewritten or shrunk* we clear
        // all overrides.  `refresh_highlights` replaces them once the fresh
        // parse for `expected_syntax_version` completes.
        if language_changed || !self.payload.code.starts_with(prev_code.as_str()) {
            self.text_overrides.clear();
        } else {
            // Even when the new code is a strict append of `prev_code`, the
            // *last token* of `prev_code` may still be mid-stream: a token
            // that started as `for` could be growing into `format`, and the
            // retained override `[0, 3)` would then color only the old `for`
            // prefix of the now-longer identifier — exactly the intra-token
            // miscoloring this fix is about.  Drop any override that ends at
            // or beyond the previous code boundary; the fresh parse will
            // restyle that token once it completes.
            let prev_end = CharOffset::from(prev_code.chars().count());
            self.text_overrides
                .retain(|(range, _)| range.end < prev_end);
        }
        self.expected_syntax_version = None;
        self.fallback_text = bounded_fallback_text(&self.payload.code);

        if self.fallback_text.is_some() {
            ctx.emit(TuiCodeBlockViewEvent::LayoutChanged);
            ctx.notify();
            return true;
        }

        if language_changed {
            self.editor = Self::create_editor(ctx);
        }
        self.editor.update(ctx, |editor, ctx| {
            if let Some(language) = &self.payload.language {
                editor.set_language_with_name(language, ctx);
            }
            editor.reset_content(InitialBufferState::plain_text(&self.payload.code), ctx);
            // Explicitly bootstrap parsing after a whole-buffer replacement.
            // This is also required in tests, where replacement event handling
            // intentionally returns before scheduling syntax work.
            editor.rebuild_layout_with_syntax_highlighting(ctx);
        });
        self.expected_syntax_version = Some(
            self.editor
                .as_ref(ctx)
                .content()
                .as_ref(ctx)
                .buffer_version(),
        );
        ctx.emit(TuiCodeBlockViewEvent::LayoutChanged);
        ctx.notify();
        true
    }

    /// Re-reads decorations only for the buffer version associated with the
    /// latest synchronized payload. A late parser event for an older revision
    /// therefore yields no map and cannot style newer streamed code.
    fn refresh_highlights(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(expected_version) = self.expected_syntax_version else {
            return;
        };
        let overrides = {
            let editor = self.editor.as_ref(ctx);
            let end = editor.content().as_ref(ctx).max_charoffset();
            if end <= CharOffset::from(1) {
                Vec::new()
            } else {
                let mut ranges = RangeSet::new();
                ranges.insert(CharOffset::from(1)..end);
                editor
                    .text_decoration_for_ranges(ranges, Some(expected_version), ctx)
                    .base_color_map
                    .as_ref()
                    .map(|colors| {
                        colors
                            .iter()
                            .map(|(range, color)| {
                                (
                                    range.clone(),
                                    TuiStyle::default().fg(Color::Rgb(color.r, color.g, color.b)),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            }
        };
        self.text_overrides = overrides;
    }

    fn render_body(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        if let Some(fallback_text) = &self.fallback_text {
            return TuiText::new(fallback_text.clone())
                .with_style(builder.primary_text_style())
                .finish();
        }
        TuiEditorElement::new(&self.editor, app)
            .with_styles(TuiEditorStyles {
                text: builder.primary_text_style(),
                ghost: builder.primary_text_style(),
                gap: builder.dim_text_style(),
                line_overrides: Vec::new(),
                text_overrides: self.text_overrides.clone(),
            })
            .hide_trailing_empty_line()
            .finish()
    }
}

/// Returns no fallback for code that is safe to highlight, or a UTF-8-safe
/// prefix bounded by both highlight limits with an explicit truncation notice.
fn bounded_fallback_text(code: &str) -> Option<String> {
    if code.len() <= MAX_HIGHLIGHT_BYTES && code.lines().count() <= MAX_CODE_LINES {
        return None;
    }

    let mut end = 0;
    let mut line_count = 1;
    for (offset, character) in code.char_indices() {
        let character_end = offset.saturating_add(character.len_utf8());
        if line_count > MAX_CODE_LINES || character_end > MAX_HIGHLIGHT_BYTES {
            break;
        }
        end = character_end;
        if character == '\n' {
            line_count += 1;
        }
    }

    let mut fallback_text = code[..end].to_owned();
    if end < code.len() {
        if !fallback_text.ends_with('\n') {
            fallback_text.push('\n');
        }
        fallback_text.push_str(TRUNCATION_NOTICE);
    }
    Some(fallback_text)
}

impl Entity for TuiCodeBlockView {
    type Event = TuiCodeBlockViewEvent;
}

impl TuiView for TuiCodeBlockView {
    fn ui_name() -> &'static str {
        "TuiCodeBlockView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        let mut column = TuiFlex::column();
        if let Some(language) = &self.payload.language {
            column.add_child(
                TuiText::new(language.clone())
                    .with_style(builder.muted_text_style())
                    .truncate()
                    .finish(),
            );
        }
        column.add_child(self.render_body(app));
        column.finish()
    }
}

#[cfg(test)]
#[path = "tui_code_block_view_tests.rs"]
mod tests;
