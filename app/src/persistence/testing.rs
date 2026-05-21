//! Module with integration test-only util methods setting up sqlite.

use diesel::{ExpressionMethods, QueryDsl, RunQueryDsl};

use super::{schema, sqlite::init_db, PersistenceScope};

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
