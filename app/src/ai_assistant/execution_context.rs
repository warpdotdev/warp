use std::sync::Arc;

pub use ai_types::{WarpAiExecutionContext, WarpAiOsContext};

use crate::terminal::model::session::Session;

/// Build the execution context that describes the given session.
pub fn execution_context_for_session(session: &Arc<Session>) -> WarpAiExecutionContext {
    WarpAiExecutionContext {
        os: WarpAiOsContext {
            category: session.host_info().os_category.clone(),
            distribution: session.host_info().linux_distribution.clone(),
        },
        shell_name: session.shell().shell_type().name().to_owned(),
        shell_version: session.shell().version().clone(),
    }
}
