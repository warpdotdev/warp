use std::collections::HashMap;
use std::ops::Range;
use std::time::Duration;

use async_channel::Sender;
use string_offset::ByteOffset;
use warp_completer::completer::SuggestionTypeName;
use warp_completer::signatures::CommandRegistry;
use warp_core::ui::theme::AnsiColorIdentifier;
use warpui::r#async::SpawnedFutureHandle;
use warpui::{Entity, ModelContext, SingletonEntity, ViewHandle};

use crate::appearance::Appearance;
use crate::completer::SessionAgnosticContext;
use crate::debounce::debounce;
use crate::editor::{EditorView, TextStyleOperation};
use crate::terminal::input::decorations::{
    parse_current_commands_and_tokens, ParsedTokenData, ParsedTokensSnapshot,
};

/// Debounce for syntax highlighting workflow
pub const DEBOUNCE_INPUT_DECORATION_PERIOD: Duration = Duration::from_millis(500);

pub struct SyntaxHighlightable {
    editor_handle: ViewHandle<EditorView>,
    syntax_highlighting_handle: Option<SpawnedFutureHandle>,
    debounce_input_background_tx: Sender<()>,
}

impl SyntaxHighlightable {
    pub fn new(editor_handle: ViewHandle<EditorView>, ctx: &mut ModelContext<Self>) -> Self {
        let (debounce_input_background_tx, debounce_input_background_rx) =
            async_channel::unbounded();

        let _ = ctx.spawn_stream_local(
            debounce(
                DEBOUNCE_INPUT_DECORATION_PERIOD,
                debounce_input_background_rx,
            ),
            |me, _, ctx| me.highlight_syntax(ctx),
            |_me, _ctx| {},
        );

        Self {
            editor_handle,
            syntax_highlighting_handle: None,
            debounce_input_background_tx,
        }
    }

    pub fn debounce_highlight(&mut self) {
        let _ = self.debounce_input_background_tx.try_send(());
    }

    pub fn highlight_syntax(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(handle) = self.syntax_highlighting_handle.take() {
            handle.abort();
        }

        let buffer_text = self.editor_handle.as_ref(ctx).buffer_text(ctx);

        let completion_context = SessionAgnosticContext::new(CommandRegistry::global_instance());
        self.syntax_highlighting_handle =
            Some(
                ctx.spawn(
                    async move {
                        parse_current_commands_and_tokens(buffer_text, &completion_context).await
                    },
                    move |highlightable, parsed_tokens, ctx| {
                        highlightable.update_with_parsed_tokens(parsed_tokens, ctx);
                    },
                ),
            );
    }

    fn update_with_parsed_tokens(
        &mut self,
        parsed_tokens: ParsedTokensSnapshot,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.editor_handle.as_ref(ctx).buffer_text(ctx) != parsed_tokens.buffer_text {
            log::warn!("Stale syntax highlighting for workflow, will not apply it");
            return;
        }

        let ranges = self.parsed_token_to_color_style_ranges(parsed_tokens.parsed_tokens, ctx);
        let appearance = Appearance::as_ref(ctx);
        let terminal_colors_normal = appearance.theme().terminal_colors().normal;
        for (suggestion_type, ranges) in ranges {
            let color: AnsiColorIdentifier = suggestion_type.into();
            self.editor_handle.update(ctx, |editor, ctx| {
                editor.update_buffer_styles(
                    ranges,
                    TextStyleOperation::default()
                        .set_syntax_color(color.to_ansi_color(&terminal_colors_normal).into()),
                    ctx,
                )
            });
        }
    }

    fn parsed_token_to_color_style_ranges(
        &mut self,
        parsed_tokens: Vec<ParsedTokenData>,
        ctx: &mut ModelContext<Self>,
    ) -> HashMap<SuggestionTypeName, Vec<Range<ByteOffset>>> {
        let mut ranges: HashMap<SuggestionTypeName, Vec<Range<ByteOffset>>> = HashMap::new();
        for token_data in parsed_tokens {
            if let Some(description) = &token_data.token_description {
                let suggestion_type = description.suggestion_type;
                let range = ByteOffset::from(token_data.token.span.start())
                    ..ByteOffset::from(token_data.token.span.end());
                ranges
                    .entry(suggestion_type.to_name())
                    .or_default()
                    .push(range);
            }
        }
        ctx.notify();
        ranges
    }
}

impl Entity for SyntaxHighlightable {
    type Event = ();
}
