//! Traits and abstractions supporting the find operation across rich content blocks.
use std::sync::atomic::{AtomicUsize, Ordering};

use warpui::{AppContext, View, ViewContext, ViewHandle};

use super::FindOptions;

/// Unique ID for a find match in a rich content view.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct RichContentMatchId(usize);

impl Default for RichContentMatchId {
    fn default() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        Self(COUNTER.fetch_add(1, Ordering::SeqCst))
    }
}

/// Trait to be implemented by blocklist rich content view to support find operations.
pub trait FindableRichContentView: View {
    /// Runs a find operation configured with the given `options` on the view and returns
    /// a list of match IDs corresponding to found matches.
    ///
    /// The view is responsible for actually storing matches (in whatever representation makes
    /// sense for the given view) and maintaining a mapping between the returned match IDs and the
    /// internal matches.
    ///
    /// As such, the view is also responsible for rendering the appropriate UI for highlighted
    /// matches.
    fn run_find(
        &mut self,
        options: &FindOptions,
        ctx: &mut ViewContext<Self>,
    ) -> Vec<RichContentMatchId>;

    /// Clears cached matches stored in a previous `run_find` call, if necessary.
    fn clear_matches(&mut self, ctx: &mut ViewContext<Self>);
}

/// Wrapper trait around `RichContentView` that enables storing a homogenous collection of
/// `RichContentView` implementations (via their corresponding `ViewHandle`s).
///
/// Simply directs each method call to the corresponding `FindableRichContentView` call.
///
/// New rich content views do _not_ require a new `FindableRichContentHandle` implementation;
/// this is an implementation detail of the `FindModel`-internal usage of the
/// `FindableRichContentView` trait.
pub(crate) trait FindableRichContentHandle {
    fn run_find(&self, options: &FindOptions, ctx: &mut AppContext) -> Vec<RichContentMatchId>;

    fn clear_matches(&self, ctx: &mut AppContext);
}

/// Blanket implementation of `FindableRichContentHandle` for any handles of view type that
/// implements `FindableRichContentView`.
impl<F> FindableRichContentHandle for ViewHandle<F>
where
    F: FindableRichContentView,
{
    fn run_find(&self, options: &FindOptions, ctx: &mut AppContext) -> Vec<RichContentMatchId> {
        self.update(ctx, |me, ctx| me.run_find(options, ctx))
    }

    fn clear_matches(&self, ctx: &mut AppContext) {
        self.update(ctx, |me, ctx| me.clear_matches(ctx));
    }
}
