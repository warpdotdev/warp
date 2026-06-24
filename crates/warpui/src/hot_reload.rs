//! Hot-reload client for WarpUI.
//!
//! Connects to `dx serve --hot-patch`'s devserver, applies binary patches via
//! [`subsecond::apply_patch`], and notifies the event loop to re-render.
//!
//! This is a standalone implementation of the devserver protocol — it does not
//! depend on `dioxus-core` or `dioxus-devtools`, mirroring the approach used by
//! the Blinc UI framework. The wire format is a small subset of dx's
//! `DevserverMsg` enum; unrecognised variants are silently ignored so minor
//! schema additions from newer dx versions won't break us.
//!
//! ## Usage
//!
//! Call [`connect`] once at application startup, before the event loop starts.
//! The event loop should pass a wake closure that forces a repaint. After each
//! successful patch, [`take_rebuild_pending`] returns `true` on the next call
//! and the caller should invalidate all views.

use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use serde::Deserialize;
use subsecond::JumpTable;

/// Set to `true` by the WebSocket thread after a successful `apply_patch`.
/// Drained by [`take_rebuild_pending`]; the event loop reads this once per
/// frame to decide whether to invalidate all views.
static REBUILD_PENDING: AtomicBool = AtomicBool::new(false);

type WakeFn = Box<dyn Fn() + Send + Sync + 'static>;

/// Callback registered by the event loop. Called after each patch to wake the
/// event loop out of `ControlFlow::Wait` so it can check `REBUILD_PENDING`.
static WAKE_FN: OnceLock<WakeFn> = OnceLock::new();

/// Drains the rebuild-pending flag. Returns `true` if a hot-patch landed since
/// the last call. Cheap (single atomic swap); safe to call every frame.
pub fn take_rebuild_pending() -> bool {
    REBUILD_PENDING.swap(false, Ordering::AcqRel)
}

/// Connect to the dx devserver and begin receiving hot-patch messages.
///
/// `wake` is called after every successful patch to nudge the event loop out
/// of `ControlFlow::Wait`. Pass the winit `EventLoopProxy::send_event` closure
/// or similar.
///
/// If `DIOXUS_DEVSERVER_PORT` is not set (normal `cargo run` path), this
/// function returns immediately without spawning a thread.
pub fn connect(wake: impl Fn() + Send + Sync + 'static) {
    let _ = WAKE_FN.set(Box::new(wake));

    // Register with subsecond so the stale-call recovery path also wakes the
    // event loop (fires after every successful apply_patch, providing a
    // belt-and-suspenders guarantee alongside our WebSocket handler below).
    subsecond::register_handler(Arc::new(|| {
        REBUILD_PENDING.store(true, Ordering::Release);
        if let Some(f) = WAKE_FN.get() {
            f();
        }
    }));

    let Some(endpoint) = devserver_ws_endpoint() else {
        log::debug!(
            "hot-reload: DIOXUS_DEVSERVER_PORT not set — not connecting \
             (run under `dx serve --hot-patch` to enable)"
        );
        return;
    };
    eprintln!("[hot-reload] connecting to {endpoint}");

    let _ = std::thread::Builder::new()
        .name("warpui-hot-reload".into())
        .spawn(move || run(endpoint));
}

/// Construct the devserver WebSocket URL from env vars set by `dx serve`.
fn devserver_ws_endpoint() -> Option<String> {
    let ip = std::env::var("DIOXUS_DEVSERVER_IP").unwrap_or_else(|_| "localhost".to_string());
    let port = std::env::var("DIOXUS_DEVSERVER_PORT").ok()?;
    Some(format!("ws://{ip}:{port}/_dioxus"))
}

/// The build ID stamped into the binary by `dx`, read back to authenticate
/// patches from the matching devserver session.
fn build_id() -> u64 {
    std::env::var("DIOXUS_BUILD_ID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn run(endpoint: String) {
    let uri = format!(
        "{endpoint}?aslr_reference={}&build_id={}&pid={}",
        subsecond::aslr_reference(),
        build_id(),
        process::id(),
    );

    let (mut ws, _resp) = match tungstenite::connect(&uri) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("[hot-reload] connection FAILED: {e}");
            return;
        }
    };

    eprintln!("[hot-reload] connected OK to {uri}");

    while let Ok(msg) = ws.read() {
        if let tungstenite::Message::Text(text) = msg {
            handle_message(text.as_str());
        }
    }

    log::debug!("hot-reload: devserver connection closed");
}

fn handle_message(text: &str) {
    let msg: DevserverMsg = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            log::trace!("hot-reload: ignoring unparseable message: {e}");
            return;
        }
    };

    let DevserverMsg::HotReload(hot) = msg else {
        return;
    };

    // PID guard: ignore patches meant for a different process instance.
    if let Some(target_pid) = hot.for_pid {
        if target_pid != process::id() {
            log::trace!(
                "hot-reload: patch for pid {target_pid}, ours is {}, skipping",
                process::id()
            );
            return;
        }
    }

    let Some(jump_table) = hot.jump_table else {
        return;
    };

    // SAFETY: `subsecond::apply_patch` is unsafe because the patcher and the
    // running process must agree on symbol layout and ASLR offset. The dx CLI
    // guarantees this: it links the patch against this binary's symbol table
    // and the ASLR reference we sent as a query parameter at connect time.
    // The `for_pid` guard above ensures we never apply another process's patch.
    unsafe {
        match subsecond::apply_patch(jump_table) {
            Ok(()) => {
                eprintln!("[hot-reload] patch applied — triggering redraw");
                REBUILD_PENDING.store(true, Ordering::Release);
                if let Some(wake) = WAKE_FN.get() {
                    wake();
                } else {
                    eprintln!("[hot-reload] WARNING: no wake fn registered");
                }
            }
            Err(e) => eprintln!("[hot-reload] patch FAILED: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Wire protocol types — minimal mirror of dx's DevserverMsg.
// ---------------------------------------------------------------------------

/// Subset of the dx devserver message enum. `#[serde(other)]` on `Unknown`
/// means new variants added by future dx versions won't break deserialization.
#[derive(Debug, Deserialize)]
enum DevserverMsg {
    HotReload(HotReloadMsg),
    #[serde(other)]
    Unknown,
}

/// Subset of dx's `HotReloadMsg`. Fields we don't act on are accepted as their
/// default types so schema additions don't break deserialization.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct HotReloadMsg {
    /// The binary patch (function-body swap). `None` for asset-only reloads.
    jump_table: Option<JumpTable>,
    /// PID of the process this patch targets. `None` means broadcast.
    for_pid: Option<u32>,
    // Accept but ignore these fields:
    #[allow(dead_code)]
    templates: serde_json::Value,
    #[allow(dead_code)]
    assets: Vec<PathBuf>,
    #[allow(dead_code)]
    for_build_id: Option<u64>,
    #[allow(dead_code)]
    ms_elapsed: Option<u64>,
}
