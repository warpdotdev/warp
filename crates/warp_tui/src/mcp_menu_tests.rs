//! Render-to-lines fixtures for the TUI MCP menu.
//!
//! These drive the real [`TuiMcpMenuModel`] (through `refresh_rows`) against a
//! seeded [`TuiMcpSnapshot`] and render the resulting inline-menu snapshot via
//! `render_inline_menu`, asserting the exact rendered surface for the
//! cross-process OAuth coordination states introduced by APP-4959:
//!
//! - `WaitingForAuthentication` (a follower): renders the stable waiting copy,
//!   has no authorization URL, and is not selectable / has no
//!   `ReopenAuthorization` action (spec invariants #8, criterion #14).
//! - `Authenticating` (the leader): renders "authentication required" and is
//!   selectable (it owns the reopenable URL).
//! - `Running` (post-auth): renders the tool count and a stop/logout action.

use std::path::PathBuf;

use uuid::Uuid;
use warp::appearance::Appearance;
use warp::tui_export::{
    TuiMcpConfigState, TuiMcpManager, TuiMcpServerId, TuiMcpServerSnapshot, TuiMcpServerStatus,
    TuiMcpSnapshot, TuiMcpTransport,
};
use warpui_core::App;
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;

use super::TuiMcpMenuModel;
use crate::inline_menu::{TuiInlineMenuSnapshot, render_inline_menu};
use crate::input_suggestions_mode::TuiInputSuggestionsModeModel;
use crate::tui_builder::TuiUiBuilder;

/// Stable copy shown in the TUI menu for a follower waiting on another Warp
/// instance's OAuth flow. Kept in sync with `refresh_rows` in `mcp_menu.rs`.
const WAITING_COPY: &str = "waiting for auth in another instance…";

fn server_snapshot(
    id: u64,
    name: &str,
    status: TuiMcpServerStatus,
    has_credentials: bool,
    authorization_url: Option<&str>,
    tool_count: usize,
) -> TuiMcpServerSnapshot {
    TuiMcpServerSnapshot {
        id: TuiMcpServerId(id),
        installation_uuid: Uuid::new_v4(),
        name: name.to_string(),
        transport: TuiMcpTransport::Stdio,
        status,
        tool_count,
        resource_count: 0,
        has_credentials,
        authorization_url: authorization_url.map(str::to_string),
    }
}

fn snapshot_with(servers: Vec<TuiMcpServerSnapshot>) -> TuiMcpSnapshot {
    TuiMcpSnapshot {
        config_path: PathBuf::from("/home/user/.warp/mcp.toml"),
        config_state: TuiMcpConfigState::Ready,
        servers,
    }
}

/// Opens the MCP menu model against a seeded `TuiMcpManager` snapshot and
/// returns the rendered inline-menu lines plus the produced snapshot rows.
fn render_menu(snapshot: TuiMcpSnapshot) -> (Vec<String>, TuiInlineMenuSnapshot) {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
            ctx.add_singleton_model(|ctx| TuiMcpManager::for_test(ctx, snapshot));
            let suggestions_mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
            let menu = ctx.add_model(|ctx| TuiMcpMenuModel::new(suggestions_mode, ctx));
            menu.update(ctx, |model, ctx| model.open(ctx));
            let snap = menu
                .as_ref(ctx)
                .snapshot(ctx)
                .expect("open MCP menu must produce an inline-menu snapshot");
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_inline_menu(&snap, &TuiUiBuilder::from_app(ctx)),
                TuiRect::new(0, 0, 72, 14),
                ctx,
            );
            (frame.buffer.to_lines(), snap)
        })
    })
}

#[test]
fn waiting_for_authentication_renders_stable_copy_with_no_url_or_reopen() {
    let snapshot = snapshot_with(vec![server_snapshot(
        100,
        "figma",
        TuiMcpServerStatus::WaitingForAuthentication,
        /* has_credentials */ false,
        /* authorization_url */ None,
        0,
    )]);

    let (lines, snap) = render_menu(snapshot);

    // The waiting row is informational only: no action, not selectable, and no
    // authorization URL on the seeded server (a follower never has one).
    let waiting_row = snap
        .rows
        .iter()
        .find(|row| row.title == "figma")
        .expect("waiting server row is present");
    assert!(
        waiting_row
            .description
            .as_deref()
            .is_some_and(|d| d.contains(WAITING_COPY)),
        "waiting row description must carry the stable copy, got {:?}",
        waiting_row.description
    );
    assert!(
        !waiting_row.is_selectable,
        "a follower must not be selectable (no ReopenAuthorization action)"
    );

    // The rendered surface must show the exact stable copy and must not surface
    // any authorization URL or "authentication required" reopen affordance.
    let rendered = lines.join("\n");
    assert!(
        rendered.contains(WAITING_COPY),
        "rendered menu must contain the waiting copy; rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("authentication required"),
        "a waiting follower must not show the leader's authentication-required row"
    );
    assert!(
        !rendered.contains("http"),
        "no authorization URL may be rendered for a waiting follower"
    );
}

#[test]
fn leader_authenticating_and_running_states_render_alongside_waiting() {
    let snapshot = snapshot_with(vec![
        server_snapshot(
            100,
            "figma",
            TuiMcpServerStatus::WaitingForAuthentication,
            false,
            None,
            0,
        ),
        server_snapshot(
            200,
            "github",
            TuiMcpServerStatus::Authenticating,
            false,
            Some("https://example.invalid/oauth/authorize"),
            0,
        ),
        server_snapshot(300, "notion", TuiMcpServerStatus::Running, true, None, 5),
    ]);

    let (lines, snap) = render_menu(snapshot);

    // The follower waiting row stays non-selectable with the stable copy...
    let waiting = snap
        .rows
        .iter()
        .find(|row| row.title == "figma")
        .expect("waiting row");
    assert!(!waiting.is_selectable, "follower row is not selectable");
    assert!(
        waiting
            .description
            .as_deref()
            .is_some_and(|d| d.contains(WAITING_COPY)),
        "waiting row keeps the stable copy"
    );

    // ...while the leader (Authenticating) row is selectable because it owns the
    // reopenable URL, and the Running row carries a stop action.
    let leader = snap
        .rows
        .iter()
        .find(|row| row.title == "github")
        .expect("leader row");
    assert!(
        leader.is_selectable,
        "leader Authenticating row must be selectable (ReopenAuthorization)"
    );
    let running = snap
        .rows
        .iter()
        .find(|row| row.title == "notion")
        .expect("running row");
    assert!(
        running.is_selectable,
        "Running row must be selectable (Stop)"
    );
    // A server with credentials also exposes a Log out row.
    assert!(
        snap.rows.iter().any(|row| row.title == "Log out notion"),
        "Running server with credentials must expose a Log out row"
    );

    let rendered = lines.join("\n");
    assert!(
        rendered.contains(WAITING_COPY),
        "waiting copy renders; rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("authentication required"),
        "leader authentication-required copy renders"
    );
    assert!(
        rendered.contains("running · 5 tools"),
        "running tool-count copy renders"
    );
}
