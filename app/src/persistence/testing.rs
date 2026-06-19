//! Module with integration test-only util methods setting up sqlite.

use std::path::Path;

use diesel::{ExpressionMethods, QueryDsl, RunQueryDsl};

use super::sqlite::{get_all_workspace_language_servers_by_workspace, init_db};
use super::{schema, PersistenceScope};
use crate::ai::persisted_workspace::EnablementState;

/// Updates the 'user' and 'host' columns for stored blocks to the given values.
///
/// This is used at runtime to update the user and host values to real values based on the running
/// machine in integration tests that rely on accuracy of these values.
pub fn set_user_and_hostname_for_blocks(user: String, hostname: String) {
    let mut conn =
        init_db(&PersistenceScope::App).expect("Should be able to establish sqlite connection.");

    // Update the 'user' and 'host' columns to their real values (based on the machine on which this test is running)
    // for blocks that were stored with the placeholder 'local:user' and 'local:host' values.
    //
    // This allows us to use real (rather than mocked out) logic for matching restored
    // blocks to the appropriate session based on session hostnamebased on system hostname.
    diesel::update(schema::blocks::dsl::blocks.filter(schema::blocks::user.eq("local:user")))
        .set((
            schema::blocks::user.eq(user),
            schema::blocks::host.eq(hostname),
        ))
        .execute(&mut conn)
        .expect("Failed to update user and hostname for restored blocks.");
}

pub fn set_user_and_hostname_for_commands(user: String, hostname: String) {
    let mut conn =
        init_db(&PersistenceScope::App).expect("Should be able to establish sqlite connection.");

    // Update the 'user' and 'host' columns to their real values (based on the machine on which
    // this test is running) for commands that were stored with the placeholder 'local:user' and
    // 'local:host' values.
    //
    // This allows us to use real (rather than mocked out) logic for matching history commands to
    // the appropriate session based on session hostnamebased on system hostname.
    diesel::update(
        schema::commands::dsl::commands.filter(schema::commands::username.eq("local:user")),
    )
    .set((
        schema::commands::username.eq(user),
        schema::commands::hostname.eq(hostname),
    ))
    .execute(&mut conn)
    .expect("Failed to update user and hostname for persisted commands.");
}

/// Reads the persisted enablement of a custom LSP server (`kind = 'Custom'`)
/// the way a fresh app launch does: through the same loader, which inner-joins
/// `workspace_language_server` against `workspace_metadata`. An orphaned custom
/// row — one whose `workspace_metadata` parent was never written, or was
/// deleted by metadata cleanup — is invisible to that join, exactly as it would
/// be on the next launch.
///
/// Returns `Some(state)` when a surviving row exists for `(repo_path, name)`,
/// or `None` when nothing survives the reload.
pub fn persisted_custom_lsp_enablement(repo_path: &Path, name: &str) -> Option<EnablementState> {
    let mut conn =
        init_db(&PersistenceScope::App).expect("Should be able to establish sqlite connection.");
    let servers = get_all_workspace_language_servers_by_workspace(&mut conn)
        .expect("reading persisted workspace language servers should succeed");
    servers
        .custom
        .get(repo_path)
        .and_then(|by_name| by_name.get(name))
        .cloned()
}
