//! Durable CLI-agent session handles ("task handles").
//!
//! A handle records `(agent, session_id, cwd)` for a CLI agent session so the
//! project rail can list past tasks by name and resume them after the agent
//! exits or the app restarts. The table is a rebuildable index — the agent's
//! own transcript store is the source of truth — and every write here is
//! driven by plugin session events, never by Warp's own agent-mode drivers.
//!
//! Identity model (see the partial unique indexes in the migration):
//! - `(agent, session_id)` is the *task*: one row per upstream session, no
//!   matter which pane ran or resumed it.
//! - `(pane_uuid, agent)` scopes the single *in-flight* row (`session_id IS
//!   NULL`) a pane may have while an agent is starting up and has not yet
//!   revealed its id.

use chrono::Utc;
use diesel::SqliteConnection;
use diesel::prelude::*;

use crate::persistence::model::AgentSessionHandleRecord;
use crate::persistence::schema::agent_session_handles::dsl;

/// Upper bound on stored handles per distinct `cwd`; the cheapest stand-in for
/// a per-project cap until read-time project resolution exists. Aligned with
/// the rail's "N more…" overflow so GC and display agree about scale.
const MAX_HANDLES_PER_CWD: i64 = 20;

/// Handles older than this are aged out on the next write, mirroring Claude
/// Code's default `cleanupPeriodDays` after which the transcript itself is
/// deleted and the handle could no longer resume anything.
const MAX_HANDLE_AGE_DAYS: i64 = 30;

/// Records that an agent session started in `pane_uuid` at `cwd` with no known
/// session id yet. Replaces any previous in-flight row for the same
/// `(pane_uuid, agent)` — a pane relaunching an agent supersedes its earlier
/// unidentified launch, which can no longer be resolved to anything.
pub(super) fn insert_inflight(
    conn: &mut SqliteConnection,
    agent_name: &str,
    pane: &[u8],
    cwd_path: &str,
) -> Result<(), diesel::result::Error> {
    let now = Utc::now().naive_utc();
    diesel::delete(
        dsl::agent_session_handles
            .filter(dsl::pane_uuid.eq(pane))
            .filter(dsl::agent.eq(agent_name))
            .filter(dsl::session_id.is_null()),
    )
    .execute(conn)?;
    diesel::insert_into(dsl::agent_session_handles)
        .values((
            dsl::agent.eq(agent_name),
            dsl::cwd.eq(cwd_path),
            dsl::pane_uuid.eq(pane),
            dsl::created_at.eq(now),
            dsl::last_seen_at.eq(now),
        ))
        .execute(conn)?;
    Ok(())
}

/// Attaches a now-known session id to the pane's in-flight row, or — when the
/// id already exists as a task row (the session was resumed) — merges into it:
/// the existing task row keeps its history, gains the new pane as provenance,
/// and the in-flight row is dropped. `cwd` is refreshed in both cases because
/// resuming can legitimately happen from a different checkout of the repo.
pub(super) fn identify(
    conn: &mut SqliteConnection,
    agent_name: &str,
    pane: &[u8],
    cwd_path: &str,
    session: &str,
) -> Result<(), diesel::result::Error> {
    let now = Utc::now().naive_utc();

    let existing_task: Option<i32> = dsl::agent_session_handles
        .filter(dsl::agent.eq(agent_name))
        .filter(dsl::session_id.eq(session))
        .select(dsl::id)
        .first(conn)
        .optional()?;

    if let Some(task_id) = existing_task {
        // Resume of a known session: refresh the task row, drop the in-flight
        // row that this launch created.
        diesel::update(dsl::agent_session_handles.filter(dsl::id.eq(task_id)))
            .set((
                dsl::pane_uuid.eq(pane),
                dsl::cwd.eq(cwd_path),
                dsl::last_seen_at.eq(now),
            ))
            .execute(conn)?;
        diesel::delete(
            dsl::agent_session_handles
                .filter(dsl::pane_uuid.eq(pane))
                .filter(dsl::agent.eq(agent_name))
                .filter(dsl::session_id.is_null()),
        )
        .execute(conn)?;
    } else {
        let promoted = diesel::update(
            dsl::agent_session_handles
                .filter(dsl::pane_uuid.eq(pane))
                .filter(dsl::agent.eq(agent_name))
                .filter(dsl::session_id.is_null()),
        )
        .set((
            dsl::session_id.eq(session),
            dsl::cwd.eq(cwd_path),
            dsl::last_seen_at.eq(now),
        ))
        .execute(conn)?;
        // The id can arrive without a preceding session_start (missed event,
        // restored pane): create the task row directly rather than losing it.
        if promoted == 0 {
            diesel::insert_into(dsl::agent_session_handles)
                .values((
                    dsl::agent.eq(agent_name),
                    dsl::session_id.eq(session),
                    dsl::cwd.eq(cwd_path),
                    dsl::pane_uuid.eq(pane),
                    dsl::created_at.eq(now),
                    dsl::last_seen_at.eq(now),
                ))
                .execute(conn)?;
        }
    }

    gc(conn, cwd_path)?;
    Ok(())
}

/// Stamps `last_seen_at` for the session. Dormant ordering in the rail is
/// `last_seen_at DESC`, so this is what keeps a just-exited task at the top.
pub(super) fn touch(
    conn: &mut SqliteConnection,
    agent_name: &str,
    session: &str,
) -> Result<(), diesel::result::Error> {
    diesel::update(
        dsl::agent_session_handles
            .filter(dsl::agent.eq(agent_name))
            .filter(dsl::session_id.eq(session)),
    )
    .set(dsl::last_seen_at.eq(Utc::now().naive_utc()))
    .execute(conn)?;
    Ok(())
}

/// Writes the resolved display label back onto the handle. The cache lets a
/// dormant row paint its name with no disk read; the resolver refreshes it
/// whenever a higher-tier source appears.
pub(super) fn set_title(
    conn: &mut SqliteConnection,
    agent_name: &str,
    session: &str,
    label: &str,
) -> Result<(), diesel::result::Error> {
    diesel::update(
        dsl::agent_session_handles
            .filter(dsl::agent.eq(agent_name))
            .filter(dsl::session_id.eq(session)),
    )
    .set(dsl::title.eq(label))
    .execute(conn)?;
    Ok(())
}

/// Persists the read/unread bits so a dormant row keeps its acknowledgement
/// state and a manual "mark as unread" shows on it.
pub(super) fn set_read_state(
    conn: &mut SqliteConnection,
    agent_name: &str,
    session: &str,
    seen: bool,
    unread: bool,
) -> Result<(), diesel::result::Error> {
    diesel::update(
        dsl::agent_session_handles
            .filter(dsl::agent.eq(agent_name))
            .filter(dsl::session_id.eq(session)),
    )
    .set((dsl::success_seen.eq(seen), dsl::marked_unread.eq(unread)))
    .execute(conn)?;
    Ok(())
}

/// Loads every handle, most recently seen first. The in-memory model is the
/// read surface; this runs once at startup (and after external rebuilds).
pub(super) fn load_all(
    conn: &mut SqliteConnection,
) -> Result<Vec<AgentSessionHandleRecord>, diesel::result::Error> {
    dsl::agent_session_handles
        .order(dsl::last_seen_at.desc())
        .select(AgentSessionHandleRecord::as_select())
        .load(conn)
}

/// Deletes a handle by task identity (the user's explicit "Forget").
pub(super) fn forget(
    conn: &mut SqliteConnection,
    agent_name: &str,
    session: &str,
) -> Result<(), diesel::result::Error> {
    diesel::delete(
        dsl::agent_session_handles
            .filter(dsl::agent.eq(agent_name))
            .filter(dsl::session_id.eq(session)),
    )
    .execute(conn)?;
    Ok(())
}

/// Ages out handles past [`MAX_HANDLE_AGE_DAYS`] and trims the touched `cwd`
/// to [`MAX_HANDLES_PER_CWD`] rows (oldest-seen first). Only ever driven by a
/// successful write: a read failure must never prune (a transient error is not
/// evidence that sessions are gone).
fn gc(conn: &mut SqliteConnection, cwd_path: &str) -> Result<(), diesel::result::Error> {
    let cutoff = Utc::now().naive_utc() - chrono::Duration::days(MAX_HANDLE_AGE_DAYS);
    diesel::delete(dsl::agent_session_handles.filter(dsl::last_seen_at.lt(cutoff)))
        .execute(conn)?;

    let excess: Vec<i32> = dsl::agent_session_handles
        .filter(dsl::cwd.eq(cwd_path))
        .order(dsl::last_seen_at.desc())
        .select(dsl::id)
        .load(conn)?
        .into_iter()
        .skip(MAX_HANDLES_PER_CWD as usize)
        .collect();
    if !excess.is_empty() {
        diesel::delete(dsl::agent_session_handles.filter(dsl::id.eq_any(excess))).execute(conn)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "agent_session_handles_tests.rs"]
mod tests;
