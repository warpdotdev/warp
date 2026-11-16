//! Notifier for GitHub authentication state changes.
//!
//! This singleton emits events when GitHub OAuth completes, allowing
//! any component in the app (e.g., `UpdateEnvironmentForm`) to react to
//! auth state changes without relying on window activation timing.

use warpui::{Entity, ModelContext, SingletonEntity};

/// Events emitted by the GitHub auth notifier.
#[derive(Debug, Clone)]
pub enum GitHubAuthEvent {
    /// GitHub authentication completed successfully.
    /// Components should refetch GitHub data when this fires.
    AuthCompleted,
}

/// Singleton notifier for GitHub authentication state.
///
/// This serves as a coordination point for GitHub OAuth flows.
/// When auth completes (detected via URI callback), this notifier emits
/// an `AuthCompleted` event that any subscribed component can react to.
pub struct GitHubAuthNotifier;

impl GitHubAuthNotifier {
    pub fn new() -> Self {
        Self
    }

    /// Notify subscribers that GitHub auth has completed.
    /// Call this from within an update closure on the notifier.
    pub fn notify_auth_completed(&self, ctx: &mut ModelContext<Self>) {
        ctx.emit(GitHubAuthEvent::AuthCompleted);
    }
}

impl Default for GitHubAuthNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Entity for GitHubAuthNotifier {
    type Event = GitHubAuthEvent;
}

impl SingletonEntity for GitHubAuthNotifier {}
