mod queries;
use std::cell::{Ref, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arborium::tree_sitter::{InputEdit, Parser, Tree};
use futures::stream::AbortHandle;
use languages::Language;
use parking_lot::Mutex;
use queries::highlight_query::HighlightQuery;
pub use queries::highlight_query::{ColorMap, TextSlice};
use queries::indent_query::{indentation_delta, IndentDelta};
use rangemap::{RangeMap, RangeSet};
use string_offset::{ByteOffset, CharOffset};
use warp_editor::content::buffer::{Buffer, BufferSnapshot};
use warp_editor::content::edit::PreciseDelta;
use warp_editor::content::text::IndentUnit;
use warp_editor::content::version::BufferVersion;
use warp_editor::decoration::DecorationLayer;
use warpui_core::color::ColorU;
use warpui_core::text::point::Point;
use warpui_core::{AppContext, Entity, ModelContext, WeakModelHandle};

const MAX_SYNTAX_TREES: usize = 3;

/// Maximum buffer size in bytes for which we attempt to parse a syntax tree.
/// Files larger than this are skipped to avoid tree-sitter's super-linear
/// memory growth on large inputs. See this tree-sitter issue:
/// https://github.com/tree-sitter/tree-sitter/issues/222#issuecomment-435987441
const MAX_PARSE_BYTES: usize = 2 * 1024 * 1024; // 2 MB

thread_local! {
    static PARSER: RefCell<Parser> = RefCell::new(Parser::new());
}
pub enum DecorationStateEvent {
    DecorationUpdated { version: BufferVersion },
}

struct LanguageQueries {
    language: Arc<Language>,
    syntax_query: HighlightQuery,
}

/// Single-entry cache for highlight queries.
/// Stores the most recent highlight computation result.
struct HighlightCache {
    key: HighlightCacheKey,
    highlights: RangeMap<CharOffset, ColorU>,
}

struct HighlightCacheKey {
    version: BufferVersion,
    ranges: RangeSet<CharOffset>,
    language_id: Option<arborium::tree_sitter::Language>,
}

impl HighlightCacheKey {
    /// Check if this cache entry matches the given content version, ranges, and language.
    fn matches(
        &self,
        version: BufferVersion,
        ranges: &RangeSet<CharOffset>,
        language_id: &Option<arborium::tree_sitter::Language>,
    ) -> bool {
        if self.version != version {
            return false;
        }
        if &self.language_id != language_id {
            return false;
        }
        // RangeSet derives PartialEq, so we can compare directly
        &self.ranges == ranges
    }
}

/// Manages the decoration styles derived from the underlying text source (e.g. syntax highlighting).
/// The updates are computed asynchronously and we notify the editor model upon completion via
/// DecorationUpdated event.
pub struct SyntaxTreeState {
    syntax_tree: Mutex<HashMap<BufferVersion, Tree>>,
    language_queries: Option<LanguageQueries>,
    buffer_version: BufferVersion,
    color_map: ColorMap,
    buffer_handle: WeakModelHandle<Buffer>,
    parsing_handle: Option<AbortHandle>,
    /// Cache for highlight results to avoid recomputing for the same viewport ranges.
    highlight_cache: RefCell<Option<HighlightCache>>,
}

impl SyntaxTreeState {
    pub fn new(
        buffer_handle: WeakModelHandle<Buffer>,
        buffer_version: BufferVersion,
        color_map: ColorMap,
    ) -> Self {
        Self {
            color_map,
            syntax_tree: Mutex::new(HashMap::new()),
            buffer_version,
            buffer_handle,
            parsing_handle: None,
            language_queries: None,
            highlight_cache: RefCell::new(None),
        }
    }

    pub fn set_language(&mut self, language: Arc<Language>) {
        self.language_queries = Some(LanguageQueries {
            syntax_query: HighlightQuery::new(&language.highlight_query, self.color_map),
            language,
        });
    }

    pub fn has_supported_highlighting(&self) -> bool {
        self.language_queries.is_some()
    }

    pub fn indent_unit(&self) -> Option<IndentUnit> {
        self.language_queries
            .as_ref()
            .map(|queries| queries.language.indent_unit)
    }

    pub fn bracket_pairs(&self) -> Option<&[(char, char)]> {
        self.language_queries
            .as_ref()
            .map(|queries| queries.language.bracket_pairs.as_slice())
    }

    pub fn comment_prefix(&self) -> Option<&str> {
        self.language_queries
            .as_ref()
            .and_then(|queries| queries.language.comment_prefix.as_ref())
            .map(|s| s.as_str())
    }

    /// Given multiple character ranges, return their corresponding highlight colors.
    /// If the tree is not ready or the buffer model has been deallocated, this returns None.
    pub fn highlights_in_ranges(
        &self,
        ranges: RangeSet<CharOffset>,
        render_content_version: Option<BufferVersion>,
        ctx: &AppContext,
    ) -> Option<Ref<'_, RangeMap<CharOffset, ColorU>>> {
        // If no render content version is provided, default the most recent content version.
        let buffer_version = render_content_version.unwrap_or(self.buffer_version);

        let language_id = self
            .language_queries
            .as_ref()
            .map(|q| q.language.grammar.clone());

        // Check cache first
        if let Ok(cache) = Ref::filter_map(self.highlight_cache.borrow(), |c| c.as_ref()) {
            if cache.key.matches(buffer_version, &ranges, &language_id) {
                // Return a borrowed reference to the cached highlights
                return Some(Ref::map(cache, |c| &c.highlights));
            }
        }

        // Cache miss - compute highlights
        let mut syntax_tree_lock = self.syntax_tree.lock();
        let tree = syntax_tree_lock.get(&buffer_version)?;
        let buffer = self.buffer_handle.upgrade(ctx)?;
        let language_queries = self.language_queries.as_ref()?;

        let mut combined_highlights = RangeMap::new();

        // Iterate over all ranges and collect highlights for each
        for range in ranges.iter() {
            let highlights = language_queries.syntax_query.get_highlighted_chunks(
                range.clone(),
                &language_queries.language.highlight_query,
                buffer.as_ref(ctx),
                tree,
            );

            // Merge the highlights into the combined map
            for (highlight_range, color) in highlights.iter() {
                combined_highlights.insert(highlight_range.clone(), *color);
            }
        }

        // Once we have rendered content version X, we could discard syntax trees belonging to versions before X.
        if let Some(render_content_version) = render_content_version {
            // First, drop any versions older than the rendered one in a single pass.
            syntax_tree_lock.retain(|version, _| *version >= render_content_version);
            Self::truncate_tree_state(&mut syntax_tree_lock, self.buffer_version);
        }

        // Store in cache before returning
        *self.highlight_cache.borrow_mut() = Some(HighlightCache {
            key: HighlightCacheKey {
                version: buffer_version,
                ranges,
                language_id,
            },
            highlights: combined_highlights,
        });

        // Return a borrowed reference to the cached highlights
        Ref::filter_map(self.highlight_cache.borrow(), |c| {
            c.as_ref().map(|cache| &cache.highlights)
        })
        .ok()
    }

    /// Given a point in buffer, return the absolute indentation level the point should have.
    pub fn indentation_at_point(&self, point: Point, ctx: &AppContext) -> Option<IndentDelta> {
        let syntax_tree_lock = self.syntax_tree.lock();
        let tree = syntax_tree_lock.get(&self.buffer_version)?;
        let buffer = self.buffer_handle.upgrade(ctx)?;
        let language_queries = self.language_queries.as_ref()?;

        indentation_delta(
            buffer.as_ref(ctx),
            tree,
            point,
            language_queries.language.indents_query.as_ref()?,
        )
    }

    /// Re-parse the tree based on the updated tree and source content.
    ///
    /// Returns `None` if the buffer exceeds [`MAX_PARSE_BYTES`].
    async fn parse_text(
        content: BufferSnapshot,
        old_tree: Option<Tree>,
        language: &Language,
    ) -> Option<Tree> {
        if content.byte_len() > MAX_PARSE_BYTES {
            return None;
        }
        Some(PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            parser
                .set_language(&language.grammar)
                .expect("incompatible grammar");
            let mut bytes = content.bytes();
            let mut callback = |byte_offset: usize, _point: arborium::tree_sitter::Point| {
                // Add 1 since the buffer is 1 indexed.
                bytes.seek(ByteOffset::from(byte_offset + 1));
                bytes.next().unwrap_or_default()
            };
            parser
                .parse_with_options(&mut callback, old_tree.as_ref(), None)
                .expect("Should succeed")
        }))
    }

    /// Translate an incoming edit delta into an InputEdit for incrementally updating the syntax
    /// tree. Uses the precomputed byte edit info (which was captured from the correct intermediate
    /// buffer state) and `replaced_points` instead of re-deriving from the final buffer.
    fn delta_to_input_edit(delta: &PreciseDelta) -> InputEdit {
        // Convert 1-indexed ByteOffset values to 0-indexed for tree-sitter.
        let start_byte = delta.replaced_byte_range.start.as_usize().saturating_sub(1);
        let old_end_byte = delta.replaced_byte_range.end.as_usize().saturating_sub(1);

        InputEdit {
            start_byte,
            old_end_byte,
            new_end_byte: start_byte + delta.new_byte_length,
            start_position: point_to_syntax_point(delta.replaced_points.start),
            old_end_position: point_to_syntax_point(delta.replaced_points.end),
            new_end_position: point_to_syntax_point(delta.new_end_point),
        }
    }

    pub fn invalidate_highlight_cache_for_version(&self, version: BufferVersion) {
        // Check if the cache exists and if it matches the version being invalidated
        let mut cache = self.highlight_cache.borrow_mut();
        if let Some(ref cached) = *cache {
            if cached.key.version == version {
                *cache = None;
            }
        }
    }

    pub fn set_color_map(&mut self, color_map: ColorMap) {
        self.color_map = color_map;
        if let Some(language_query) = self.language_queries.take() {
            self.set_language(language_query.language);
        }
        // Clear highlight cache since colors have changed
        *self.highlight_cache.borrow_mut() = None;
    }

    /// Truncates the syntax tree cache to maintain the MAX_SYNTAX_TREES policy.
    /// Keeps the oldest MAX_SYNTAX_TREES - 1 versions and the provided content_version.
    fn truncate_tree_state(
        syntax_tree_lock: &mut HashMap<BufferVersion, Tree>,
        buffer_version: BufferVersion,
    ) {
        if syntax_tree_lock.len() <= MAX_SYNTAX_TREES {
            return;
        }

        let mut versions: Vec<BufferVersion> = syntax_tree_lock.keys().copied().collect();
        versions.sort();

        let mut keep: HashSet<BufferVersion> = versions
            .iter()
            .take(MAX_SYNTAX_TREES - 1)
            .copied()
            .collect();
        keep.insert(buffer_version);

        syntax_tree_lock.retain(|v, _| keep.contains(v));
    }
}

impl DecorationLayer for SyntaxTreeState {
    fn update_internal_state_with_delta(
        &mut self,
        deltas: &[PreciseDelta],
        version: BufferVersion,
        content: BufferSnapshot,
        ctx: &mut ModelContext<Self>,
    ) {
        // If there is an active parsing in progress. Abort that first before starting another one.
        if let Some(handle) = self.parsing_handle.take() {
            handle.abort();
        }

        let Some(language) = self
            .language_queries
            .as_ref()
            .map(|language_queries| language_queries.language.clone())
        else {
            return;
        };

        let mut syntax_tree_lock = self.syntax_tree.lock();
        let mut tree = syntax_tree_lock.get(&self.buffer_version).cloned();
        if let Some(tree) = &mut tree {
            for delta in deltas {
                let edit = Self::delta_to_input_edit(delta);
                tree.edit(&edit);
            }

            // We write to the tree immediately after editing first to prevent flickering in the render
            // state before reparsing gets completed.
            if let Some(existing) = syntax_tree_lock.get_mut(&version) {
                existing.clone_from(tree);
            } else {
                syntax_tree_lock.insert(version, tree.clone());
                Self::truncate_tree_state(&mut syntax_tree_lock, version);
            }
        }

        let handle = ctx
            .spawn(
                async move {
                    let new_tree = Self::parse_text(content, tree, &language).await;
                    futures_lite::future::yield_now().await;
                    new_tree
                },
                move |model, new_tree, ctx| {
                    let Some(new_tree) = new_tree else {
                        // Buffer exceeded MAX_PARSE_BYTES; skip updating the syntax tree, but
                        // still emit DecorationUpdated so any delayed rendering is flushed
                        let mut syntax_tree_lock = model.syntax_tree.lock();
                        syntax_tree_lock.remove(&version);
                        drop(syntax_tree_lock);
                        model.invalidate_highlight_cache_for_version(version);
                        // (the editor delays showing content until this event fires).
                        ctx.emit(DecorationStateEvent::DecorationUpdated { version });
                        return;
                    };
                    let mut syntax_tree_lock = model.syntax_tree.lock();
                    model.invalidate_highlight_cache_for_version(version);
                    if let Some(old_tree) = syntax_tree_lock.get_mut(&version) {
                        *old_tree = new_tree;
                    } else {
                        // This is for the case where we are updating the syntax tree for the first time.
                        syntax_tree_lock.insert(version, new_tree);
                        Self::truncate_tree_state(&mut syntax_tree_lock, model.buffer_version);
                    }
                    ctx.emit(DecorationStateEvent::DecorationUpdated { version });
                },
            )
            .abort_handle();

        self.buffer_version = version;
        self.parsing_handle = Some(handle);
    }
}

impl Entity for SyntaxTreeState {
    type Event = DecorationStateEvent;
}

/// Convert a 1-indexed buffer Point into a 0-indexed tree-sitter Point.
fn point_to_syntax_point(point: Point) -> arborium::tree_sitter::Point {
    // Subtracting 1 from row to convert from 1-indexed buffer rows to 0-indexed tree-sitter rows.
    arborium::tree_sitter::Point {
        row: point.row.saturating_sub(1) as usize,
        column: point.column as usize,
    }
}
