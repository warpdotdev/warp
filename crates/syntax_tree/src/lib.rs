mod queries;
use std::cell::{Ref, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use arborium::tree_sitter::{InputEdit, ParseOptions, ParseState, Parser, Tree};
use instant::Instant;
use languages::Language;
use parking_lot::Mutex;
use queries::highlight_query::HighlightQuery;
pub use queries::highlight_query::{ColorMap, TextSlice};
use queries::indent_query::{IndentDelta, indentation_delta};
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
/// Files larger than this are skipped as a cheap first line of defense: this
/// check is a size comparison, not a parse attempt, so it can never itself run
/// away. It does NOT bound the cost of parsing a buffer under the limit -- see
/// [`PARSE_BUDGET`] for that. See also this tree-sitter issue:
/// https://github.com/tree-sitter/tree-sitter/issues/222#issuecomment-435987441
const MAX_PARSE_BYTES: usize = 2 * 1024 * 1024; // 2 MB

/// Wall-clock budget for a single tree-sitter parse, including error recovery.
/// Tree-sitter's error-recovery pass (`ts_parser__recover`) can allocate memory
/// super-linearly on dense-error inputs *regardless of file size*: a buffer
/// well under [`MAX_PARSE_BYTES`] can still drive multi-GB memory spikes (see
/// APP-4667). This budget is enforced via a progress callback, which
/// tree-sitter polls roughly every 100 parse actions
/// (`OP_COUNT_PER_PARSER_TIMEOUT_CHECK` in tree-sitter's own `parser.c`), so a
/// runaway parse can only overshoot the deadline by the time it takes to run
/// one more such batch, not by an amount that scales with input size.
///
/// In local benchmarks against dense-error SQL, that overshoot stayed within
/// roughly 2x this budget even for inputs at the [`MAX_PARSE_BYTES`] cap, while
/// an uncapped parse of the same input kept growing (a ~2 MB pathological
/// input took ~700ms/~220MB unbounded; an ~8.5MB one took ~3s/~870MB). 750ms
/// comfortably covers legitimate full parses of files up to the size cap while
/// keeping the worst-case bailout in the low seconds instead of unbounded.
const PARSE_BUDGET: Duration = Duration::from_millis(750);

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

/// Outcome of a single [`SyntaxTreeState::parse_text`] attempt.
enum ParseOutcome {
    Parsed(Tree),
    /// The buffer exceeds [`MAX_PARSE_BYTES`]; parsing was skipped entirely.
    TooLarge,
    /// Parsing exceeded [`PARSE_BUDGET`] before finishing.
    BudgetExceeded,
    /// Cancelled because a newer edit superseded this parse before it finished.
    /// Not indicative of a pathological buffer: the buffer's "quick" tree
    /// already reflects the edit, and a coalesced parse for it is dispatched
    /// right after, so callers should silently discard this outcome instead of
    /// falling back or latching.
    Superseded,
    /// `parse_with_options` returned `None` for a reason other than our own
    /// cancellation (e.g. a scanner error). Falls back like `BudgetExceeded`,
    /// but must not be mislabeled as a timeout or trip the latch.
    Failed,
}

impl ParseOutcome {
    #[cfg(test)]
    fn expect_tree(self, msg: &str) -> Tree {
        match self {
            ParseOutcome::Parsed(tree) => tree,
            ParseOutcome::TooLarge
            | ParseOutcome::BudgetExceeded
            | ParseOutcome::Superseded
            | ParseOutcome::Failed => panic!("{msg}"),
        }
    }
}

/// Why our own progress callback returned `true` to cancel a parse. `None` means
/// the callback never had a chance to fire before `parse_with_options` returned,
/// or fired but chose not to cancel -- either way, a `None` parse result paired
/// with no `CancelReason` indicates a failure that isn't ours.
#[derive(Clone, Copy)]
enum CancelReason {
    DeadlineExceeded,
    Superseded,
}

/// Classifies why `parse_with_options` returned `None`, given whether *our*
/// progress callback actually triggered the cancellation (and why). Kept as a
/// pure function so the deadline-vs-superseded-vs-failed classification is unit
/// testable without needing to provoke a real tree-sitter parser/scanner
/// failure.
fn classify_parse_result(
    result: Option<Tree>,
    cancel_reason: Option<CancelReason>,
) -> ParseOutcome {
    match (result, cancel_reason) {
        (Some(tree), _) => ParseOutcome::Parsed(tree),
        (None, Some(CancelReason::DeadlineExceeded)) => ParseOutcome::BudgetExceeded,
        (None, Some(CancelReason::Superseded)) => ParseOutcome::Superseded,
        (None, None) => ParseOutcome::Failed,
    }
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
    /// Cache for highlight results to avoid recomputing for the same viewport ranges.
    highlight_cache: RefCell<Option<HighlightCache>>,
    /// Set once a parse for this buffer has exceeded [`PARSE_BUDGET`]. While set,
    /// tree-sitter parsing is skipped entirely instead of re-spending the full
    /// budget on every keystroke, so a pathological buffer degrades to "no
    /// highlighting" rather than repeatedly stalling. Cleared by [`Self::set_language`].
    parse_budget_exceeded: bool,
    /// Wall-clock budget for a single parse. Defaults to [`PARSE_BUDGET`];
    /// overridable in tests (see `set_parse_budget_for_test`) so budget-exceeded
    /// behavior can be exercised deterministically without waiting on real time.
    parse_budget: Duration,
    /// Cancellation flag for the currently in-flight parse, if any. `abort()`ing
    /// the async task that runs a blocking tree-sitter parse does NOT interrupt
    /// it (the task only observes cancellation between `.await` points, and the
    /// blocking call has none until it returns) -- so this flag is threaded into
    /// the parse's own progress callback and is the only thing that actually
    /// stops an in-flight parse early.
    active_parse_cancel: Option<Arc<AtomicBool>>,
    /// Id of the most recently dispatched parse. A completion is only applied if
    /// it still matches this value, so a stale result can never be mistaken for
    /// the current one.
    active_generation: u64,
    /// The latest edit that arrived while a parse was already in flight. Only
    /// the newest edit is kept (older ones are coalesced away); it is dispatched
    /// once the in-flight parse's completion is observed, so at most one
    /// tree-sitter parse ever runs at a time for this buffer.
    pending_edit: Option<(BufferVersion, BufferSnapshot)>,
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
            language_queries: None,
            highlight_cache: RefCell::new(None),
            parse_budget_exceeded: false,
            parse_budget: PARSE_BUDGET,
            active_parse_cancel: None,
            active_generation: 0,
            pending_edit: None,
        }
    }

    /// Overrides the wall-clock parse budget for this instance. Test-only: lets
    /// tests drive a real `ParseOutcome::BudgetExceeded` completion (e.g. with
    /// `Duration::ZERO`) deterministically, without depending on the real
    /// [`PARSE_BUDGET`] constant or machine speed.
    #[cfg(test)]
    pub(crate) fn set_parse_budget_for_test(&mut self, budget: Duration) {
        self.parse_budget = budget;
    }

    pub fn set_language(&mut self, language: Arc<Language>) {
        self.language_queries = Some(LanguageQueries {
            syntax_query: HighlightQuery::new(&language.highlight_query, self.color_map),
            language,
        });
        self.parse_budget_exceeded = false;
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
        if let Ok(cache) = Ref::filter_map(self.highlight_cache.borrow(), |c| c.as_ref())
            && cache.key.matches(buffer_version, &ranges, &language_id)
        {
            // Return a borrowed reference to the cached highlights
            return Some(Ref::map(cache, |c| &c.highlights));
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
    /// `deadline` and `cancel` are supplied by the caller (rather than computed
    /// from [`PARSE_BUDGET`] here) so both can be controlled from tests: an
    /// already-past `deadline` deterministically drives a real
    /// [`ParseOutcome::BudgetExceeded`], and a pre-set `cancel` flag
    /// deterministically drives a real [`ParseOutcome::Superseded`], without
    /// depending on wall-clock timing or machine speed.
    async fn parse_text(
        content: BufferSnapshot,
        old_tree: Option<Tree>,
        language: &Language,
        deadline: Instant,
        cancel: Arc<AtomicBool>,
    ) -> ParseOutcome {
        if content.byte_len() > MAX_PARSE_BYTES {
            return ParseOutcome::TooLarge;
        }
        PARSER.with(|parser| {
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

            let mut cancel_reason = None;
            // The progress callback must return `true` to cancel parsing -- tree-sitter's
            // polarity here is easy to get backwards (see
            // https://github.com/tree-sitter/tree-sitter/discussions/4312). Checking the
            // shared `cancel` flag first means a newer edit always wins over a merely-slow
            // parse when both conditions are true.
            let mut progress_callback = |_state: &ParseState| -> bool {
                if cancel.load(Ordering::Relaxed) {
                    cancel_reason = Some(CancelReason::Superseded);
                    true
                } else if Instant::now() >= deadline {
                    cancel_reason = Some(CancelReason::DeadlineExceeded);
                    true
                } else {
                    false
                }
            };
            let options = ParseOptions::new().progress_callback(&mut progress_callback);

            let result = parser.parse_with_options(&mut callback, old_tree.as_ref(), Some(options));
            classify_parse_result(result, cancel_reason)
        })
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
        if let Some(ref cached) = *cache
            && cached.key.version == version
        {
            *cache = None;
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

    /// Drops the syntax tree for `version` and notifies the editor without
    /// updating it, for outcomes that mean "no usable tree for this parse"
    /// (too large, budget exceeded, or an unrelated parse failure).
    fn discard_tree_for_version(&mut self, version: BufferVersion, ctx: &mut ModelContext<Self>) {
        let mut syntax_tree_lock = self.syntax_tree.lock();
        syntax_tree_lock.remove(&version);
        drop(syntax_tree_lock);
        self.invalidate_highlight_cache_for_version(version);
        // The editor delays showing content until this event fires, so we still emit it
        // even though there's no new tree to show.
        ctx.emit(DecorationStateEvent::DecorationUpdated { version });
    }

    /// Starts a tree-sitter parse for `version`/`content` on the background
    /// executor. Only one parse is ever in flight per [`SyntaxTreeState`] --
    /// callers must check `self.active_parse_cancel` before calling this (see
    /// [`DecorationLayer::update_internal_state_with_delta`]).
    fn dispatch_parse(
        &mut self,
        version: BufferVersion,
        content: BufferSnapshot,
        language: Arc<Language>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.active_generation += 1;
        let generation = self.active_generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.active_parse_cancel = Some(cancel.clone());

        let old_tree = self.syntax_tree.lock().get(&version).cloned();
        let deadline = Instant::now() + self.parse_budget;

        ctx.spawn(
            async move {
                let outcome =
                    Self::parse_text(content, old_tree, &language, deadline, cancel).await;
                futures_lite::future::yield_now().await;
                outcome
            },
            move |model, outcome, ctx| {
                model.handle_parse_completion(generation, version, outcome, ctx);
            },
        );
    }

    /// Applies a completed parse's outcome, then dispatches any edit that was
    /// coalesced while this parse was running.
    fn handle_parse_completion(
        &mut self,
        generation: u64,
        version: BufferVersion,
        outcome: ParseOutcome,
        ctx: &mut ModelContext<Self>,
    ) {
        // Defense in depth: only apply/latch a completion that is still the
        // current parse. In the normal (single-in-flight-parse) flow this is
        // always true, since a new parse is only ever dispatched from here
        // (once this generation's in-flight bookkeeping is cleared) or when no
        // parse is in flight at all.
        if self.active_generation == generation {
            self.active_parse_cancel = None;

            match outcome {
                ParseOutcome::Parsed(new_tree) => {
                    let mut syntax_tree_lock = self.syntax_tree.lock();
                    self.invalidate_highlight_cache_for_version(version);
                    if let Some(old_tree) = syntax_tree_lock.get_mut(&version) {
                        *old_tree = new_tree;
                    } else {
                        // This is for the case where we are updating the syntax tree for the first time.
                        syntax_tree_lock.insert(version, new_tree);
                        Self::truncate_tree_state(&mut syntax_tree_lock, self.buffer_version);
                    }
                    drop(syntax_tree_lock);
                    ctx.emit(DecorationStateEvent::DecorationUpdated { version });
                }
                ParseOutcome::TooLarge => self.discard_tree_for_version(version, ctx),
                ParseOutcome::Failed => {
                    // Not the parse budget's fault (e.g. a scanner error) -- fall back like
                    // any other unparseable buffer, but don't mislabel it as a timeout or
                    // trip the latch over it.
                    log::warn!(
                        "[SyntaxTreeState] tree-sitter returned no tree for a reason other than the parse budget (e.g. a scanner error); disabling syntax highlighting for this parse only"
                    );
                    self.discard_tree_for_version(version, ctx);
                }
                ParseOutcome::BudgetExceeded => {
                    // Expected-but-notable: a pathological buffer tripped the parse budget.
                    // Latch it off so we don't repeat this on every keystroke; logged once
                    // per trip since the latch prevents further attempts.
                    log::warn!(
                        "[SyntaxTreeState] tree-sitter parse exceeded {PARSE_BUDGET:?} budget; disabling syntax highlighting for this buffer until its language is reset"
                    );
                    self.parse_budget_exceeded = true;
                    self.discard_tree_for_version(version, ctx);
                }
                ParseOutcome::Superseded => {
                    // A newer edit already coalesced over this one; its parse is dispatched
                    // below. Leave the existing "quick" tree/cache/event alone instead of
                    // tearing down highlighting for no reason.
                }
            }
        }

        if let Some((next_version, next_content)) = self.pending_edit.take()
            && let Some(language) = self
                .language_queries
                .as_ref()
                .map(|language_queries| language_queries.language.clone())
        {
            self.dispatch_parse(next_version, next_content, language, ctx);
        }
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
        let Some(language) = self
            .language_queries
            .as_ref()
            .map(|language_queries| language_queries.language.clone())
        else {
            return;
        };

        if self.parse_budget_exceeded {
            // This buffer already proved pathological; skip tree-sitter entirely instead of
            // re-spending PARSE_BUDGET on every keystroke.
            self.buffer_version = version;
            self.discard_tree_for_version(version, ctx);
            return;
        }

        // Eagerly apply the delta to a cloned tree so highlighting stays roughly correct
        // (and doesn't flicker) while the real reparse happens in the background, regardless
        // of whether that reparse runs now or is coalesced below.
        {
            let mut syntax_tree_lock = self.syntax_tree.lock();
            let mut tree = syntax_tree_lock.get(&self.buffer_version).cloned();
            if let Some(tree) = &mut tree {
                for delta in deltas {
                    let edit = Self::delta_to_input_edit(delta);
                    tree.edit(&edit);
                }

                // We write to the tree immediately after editing first to prevent flickering in
                // the render state before reparsing gets completed.
                if let Some(existing) = syntax_tree_lock.get_mut(&version) {
                    existing.clone_from(tree);
                } else {
                    syntax_tree_lock.insert(version, tree.clone());
                    Self::truncate_tree_state(&mut syntax_tree_lock, version);
                }
            }
        }

        self.buffer_version = version;

        if let Some(cancel) = &self.active_parse_cancel {
            // A parse is already in flight for an older edit. `abort()`ing its async task
            // wouldn't actually stop the blocking tree-sitter call underneath it (see the
            // field doc on `active_parse_cancel`), so instead signal it to bail out at its
            // next progress-callback check, and coalesce this edit: only the latest one is
            // dispatched once that parse's completion is observed, so at most one
            // tree-sitter parse ever runs at a time for this buffer.
            cancel.store(true, Ordering::Relaxed);
            self.pending_edit = Some((version, content));
            return;
        }

        self.dispatch_parse(version, content, language, ctx);
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

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
