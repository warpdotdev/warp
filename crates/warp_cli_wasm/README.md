# warp_cli_wasm — sandboxless orchestrator feasibility spike

> **Prototype / feasibility spike — not production architecture.**
> Optimized for speed of learning. The deliverable is as much the findings
> below as the code.

This crate is a thin `cdylib` that compiles `warp_cli` (the Rust CLI argument
layer that backs `oz-dev agent run`) to `wasm32-unknown-unknown` and exposes a
small `wasm-bindgen` entrypoint runnable inside a JavaScript runtime (Node).

The goal, from the originating request, was to evaluate whether a
**sandboxless / WASM-based orchestrator** is feasible for agent execution — an
orchestrator that runs only in the gaps between sub-agent execution instead of
reserving a full sandbox the whole time. A WASM runtime may preserve
session/tooling behavior while being cheaper to start and easier to scale.

## TL;DR (verdict)

**Positive, partial.** The `warp_cli` layer — argument parsing, the command
model, serde types, and the transitive dependency tree (`warp_core`,
`warp_util`, `warpui_core`, `local_control`, etc.) — **compiles cleanly to
`wasm32-unknown-unknown`** and **executes inside a Node-hosted WASM module**
with only two feature shims. An `agent run` command equivalent to
`oz-dev agent run --prompt hello --api-key=...` can be constructed and parsed,
and the module can make a real outbound HTTP request from inside the sandbox via
host `fetch`. Startup is ~18ms and the WASM artifact is ~5MB (processed).

The **gap** is the agent *execution* loop itself: `agent_sdk::run` lives in the
`warp` (GUI app) crate and needs an `AppContext` (auth, server API client, AI
client, settings, the whole app bootstrap). That whole app *already* builds for
`wasm32-unknown-unknown` today — `script/wasm/bundle` ships it to the browser.
So the question is not "can the agent run in WASM" (it already does, in the
browser) but "can the browser-wasm path be retargeted to a Node-hosted,
sandboxless, CLI-shaped runtime." This spike isolates the CLI layer to measure
how much of the orchestrator runs sandboxless in Node today and to map the gap.

## What this proves (the working slice)

1. **`warp_cli` builds for `wasm32-unknown-unknown`.** `cargo check` and
   `cargo build --release` both succeed for the spike crate, which pulls in
   `warp_cli` and its full transitive dep graph.
2. **The artifact loads and runs in Node.** `wasm-bindgen --target nodejs`
   produces a `require()`-able module; the harness in `harness/run.mjs`
   loads it and calls three entrypoints.
3. **An `agent run` entrypoint can be invoked from JS.** Because WASM has no
   `argv`/`env`, `warp_cli::Args::from_env()` returns defaults on wasm. The
   spike exposes an alternate entrypoint that builds the real `clap::Command`
   and parses a JS-provided argv (`agent_run_from_argv`) or a JSON config
   (`agent_run_from_config`) — exactly what `from_env` does on native targets,
   sourced from the JS caller instead of `std::env::args()`. The best-effort
   metadata parser accepts both `--flag value` and `--flag=value` long-option
   forms (matching clap), and the returned `argv` redacts any `--api-key`
   value (`<redacted>`) so the key is never echoed back through the result.
4. **The runtime supports outbound networking.** `http_get` makes a real
   `fetch` from inside the WASM module and returns the response — the same
   `fetch` primitive the browser wasm build's `http_client` uses, driven here
   through `wasm-bindgen-futures`.
5. **No real shell, no real filesystem.** The CLI layer runs without either —
   the spike deliberately stubs nothing about the CLI, but the CLI layer itself
   makes no FS/shell calls to parse args. This is the intentionally constrained
   runtime model the request asked for.

See `harness/run.mjs` output in the PR description for the end-to-end evidence.

## What this does NOT do (the gap)

It does **not** run the full agent execution loop
(`warp::ai::agent_sdk::run`). That function takes a `&mut AppContext` — the
GUI app's god-object holding auth state, the server API client, the AI client,
settings/preferences, telemetry, crash reporting, etc. Reaching it requires
booting the `warp` app (`run_internal` in `app/src/lib.rs`), which is the full
GUI initialization path.

Crucially: **that entire app already compiles to `wasm32-unknown-unknown`**
and ships to the browser via `script/wasm/bundle` (with features
`release_bundle,gui`, `wasm-bindgen --target web`, `wgpu`/`web-sys`). So the
agent execution path is not blocked on WASM compilation — it's blocked on
*retargeting the host* from a browser DOM/canvas/window to a Node process with
no display, and on slimming the `AppContext` bootstrap so it doesn't require
GUI-only subsystems.

## Build & run

### Requirements

- `rustup target add wasm32-unknown-unknown`
- `wasm-bindgen-cli` matching the resolved `wasm-bindgen` version
  (the workspace pins `0.2.89` but the lockfile resolves a newer patch; the
  CLI must match the resolved version exactly):
  ```bash
  cargo install wasm-bindgen-cli --version $(cargo tree -p warp_cli_wasm \
    --target wasm32-unknown-unknown -i wasm-bindgen | grep -oE 'v[0-9.]+' | head -1 | tr -d v)
  ```
- Node 18+ (for global `fetch`)

### Build

```bash
# From the repo root. Produces target/wasm-cli-pkg/warp_cli_wasm.js + _bg.wasm.
./script/wasm/build-cli-wasm

# Or a faster smoke test (cargo check only):
./script/wasm/build-cli-wasm --check
```

The underlying commands (if you need to run them by hand):

```bash
cargo build -p warp_cli_wasm --target wasm32-unknown-unknown --lib --release
wasm-bindgen --target nodejs --out-dir target/wasm-cli-pkg \
  target/wasm32-unknown-unknown/release/warp_cli_wasm.wasm
```

### Run

```bash
# The harness defaults to target/wasm-cli-pkg/warp_cli_wasm.js.
node crates/warp_cli_wasm/harness/run.mjs

# Or point it at a specific package + ping URL:
node crates/warp_cli_wasm/harness/run.mjs target/wasm-cli-pkg/warp_cli_wasm.js https://httpbin.org/get
```

The harness calls `agent_run_from_config`, `agent_run_from_argv`, and `http_get`,
printing the parsed `agent run` command (as JSON) and the HTTP response. It
redacts any `--api-key` value before logging argv, and exits with a non-zero
status if `http_get` fails, so a broken networking path can't look successful.

## Findings — what worked, what needed shims

### Worked with no changes

- **`warp_cli` itself.** The crate already has `cfg(target_family = "wasm")`
  gates (e.g. `Args::from_env()` returns defaults on wasm, `WorkerCommand`
  variants that need a Unix socket are `cfg(not(target_family = "wasm"))`).
  334 files across the workspace already use `target_family = "wasm"` gates —
  WASM is a first-class target family in this codebase.
- **The whole transitive CLI dep graph** (`warp_core`, `warp_util`,
  `warpui_core`, `local_control`, `settings`, `warp_server_auth`, etc.) compiles
  for wasm once the two shims below are in place.
- **`http_client`'s wasm story.** `crates/http_client` already has a
  `cfg(target_arch = "wasm32")` dep on `gloo` + `wasm-bindgen-futures` and uses
  `fetch` on wasm — the networking primitive an agent loop needs is already
  wired for the wasm target.

### Needed shims (two, both minimal and well-understood)

1. **`getrandom` `js` feature.** `getrandom` does not compile for
   `wasm32-unknown-unknown` by default ("the wasm*-unknown-unknown targets are
   not supported by default, you may need to enable the `js` feature"). The GUI
   `app` crate enables `getrandom = { version = "0.2", features = ["js"] }`,
   and feature unification propagates it to every crate in the GUI build graph.
   A standalone `warp_cli` wasm build does not include `app`, so the `js`
   feature is never activated. The spike enables it via a target-gated dep:
   ```toml
   [target.'cfg(target_arch = "wasm32")'.dependencies]
   getrandom = { version = "0.2", features = ["js"] }
   ```
   This is the same shim the browser wasm build relies on; it sources
   randomness from `crypto.getRandomValues`.

2. **`web-sys` `Window` + `Navigator` features.** `warpui_core`'s
   `platform/wasm.rs` calls `gloo::utils::window().navigator().user_agent()`,
   which needs `web-sys`'s `Window` and `Navigator` features. The workspace
   `web-sys` dep lists `Navigator` and `Window`, but again **feature
   unification only flows from `app`**, which pulls `web-sys` with the full
   workspace feature set. Without `app` in the graph, those features are off
   and `warpui_core` fails with `no method named navigator found`. The spike
   enables the minimal set:
   ```toml
   [target.'cfg(target_arch = "wasm32")'.dependencies]
   web-sys = { workspace = true, features = ["Window", "Navigator"] }
   ```

**Pattern:** both blockers are the same root cause — *standalone wasm builds of
CLI-adjacent crates miss the `web-sys`/`getrandom` feature unification that the
GUI `app` build provides for free.* Any future standalone wasm build of a
`warp_core`-dependent crate will hit a variant of this and should centralize
the feature activation (e.g. a small `wasm-features` helper crate, or enabling
them at the workspace level for the wasm target).

### What did NOT need stubbing (and why)

The request anticipated stubbing filesystem watchers, base indexing, local
filesystem-dependent behavior, and shell/session setup. None of that was needed
for the CLI layer, because `warp_cli` is *purely* argument parsing + the command
model — it doesn't touch the FS, spawn shells, or start watchers. Those
subsystems live in the `warp` app crate and only activate during `run_internal`
/ `agent_sdk::run`. The spike's constrained runtime (no FS, no shell) is
inherent to the CLI layer, not something we had to enforce.

## `wasm32-unknown-unknown` vs WASI

The spike uses **`wasm32-unknown-unknown`** (the same target the existing
browser wasm build uses), not WASI. Key observations:

- **`wasm32-unknown-unknown` is sufficient for the CLI layer.** All host
  interactions (randomness, HTTP, console) are bridged through `wasm-bindgen`
  imports that the JS host provides. There is no need for a WASI syscall
  surface because the spike deliberately runs with no real FS/shell.
- **WASI would provide** a POSIX-ish syscall surface (fd-based FS, environ,
  args, `clock_gettime`, etc.). That would let a *native-style* CLI run closer
  to unchanged (`Args::from_env` could read `std::env::args`), but it would
  also reintroduce the FS/shell surface the sandboxless model is trying to
  avoid, and the codebase's existing wasm investment is all
  `wasm32-unknown-unknown` + `wasm-bindgen` (browser), not WASI.
- **Recommendation:** stay on `wasm32-unknown-unknown` for the sandboxless
  orchestrator. It matches the existing wasm build, keeps the runtime
  intentionally constrained (the whole point of "sandboxless but isolated"),
  and the host-import pattern (`wasm-bindgen`) is already how the codebase
  bridges to the JS host. WASI is the right answer if we later want to run the
  *full native CLI* with real FS/shell inside the sandbox — but that's a
  different model (sandboxed, not sandboxless).

## Feasibility assessment

**Is this promising for a sandboxless orchestrator?** Yes, with a clear scope.

- **Cheap to start, small footprint.** ~18ms to load the WASM module and
  execute the first call; ~65MB RSS (Node + WASM); ~5MB processed artifact.
  This is the "runs only in the gaps between sub-agent execution" cost profile
  the request wanted — dramatically lighter than booting a full sandbox per
  orchestrator tick.
- **The CLI/orchestration-control layer is already wasm-ready.** Argument
  parsing, the command model, serde round-tripping, and outbound HTTP all work
  in the JS-hosted WASM runtime today, with two feature shims.
- **The agent execution loop is not blocked on WASM.** It already runs in the
  browser wasm build. The work is *host retargeting* (browser → Node) and
  *bootstrap slimming* (don't require GUI subsystems), not new WASM compilation.

**Biggest technical blockers (for the full agent loop, not this spike):**

1. **`AppContext` bootstrap coupling.** `agent_sdk::run` needs a fully booted
   `warp` app (`run_internal`): auth, server API client, settings, telemetry,
   crash reporting, the event loop. Most of that is `cfg(not(target_family =
   "wasm"))`-gated for native or `cfg(target_family = "wasm")`-gated for
   browser-DOM. A Node target would need a third cfg branch (or a
   headless/non-GUI wasm path) that boots the app context without a display.
2. **Host retargeting.** The browser wasm build assumes `web-sys` `Window`,
   canvas, wgpu/WebGPU, DOM events. A Node host has none of those. The
   agent-loop path (server API over HTTP/WebSocket, LLM calls) is host-agnostic
   and already wasm-supported via `http_client`/`websocket`, but anything that
   touches the UI/event loop needs a Node-shaped shim or to be feature-gated
   out.
3. **Feature unification ergonomics.** As shown above, standalone wasm builds
   miss `web-sys`/`getrandom` features that `app` provides. Scaling this to the
   full agent loop means either always building through `app` (and accepting its
   GUI dep surface) or factoring the wasm feature activation into a shared
   helper so non-`app` wasm builds get the right features.

### Next 3 steps

1. **Retarget the existing browser wasm build to Node for a headless slice.**
   Take `script/wasm/bundle`'s `cargo build --target wasm32-unknown-unknown`
   of the `warp` app and run `wasm-bindgen --target nodejs` (instead of
   `--target web`). Identify which `cfg(target_family = "wasm")` branches
   assume a browser DOM and gate them behind a finer `cfg` (e.g.
   `cfg(all(target_family = "wasm", feature = "browser"))` vs a Node/headless
   wasm). Measure how far `run_internal` gets under Node before hitting a
   `web-sys::Window` call.
2. **Add a headless `LaunchMode` that boots `AppContext` without GUI subsystems.**
   The CLI `LaunchMode::CommandLine` path already sets
   `ExecutionMode::Sdk` and skips most GUI setup, but it still goes through
   `run_internal`'s shared bootstrap (settings, crash reporting, event loop).
   Carve a minimal Node/wasm bootstrap that builds only the models
   `agent_sdk::run` needs (auth, server API, AI client) and skips UI, settings
   files, single-instance, crash reporting, font fallback, etc.
3. **Drive a real `agent_sdk::run` call from the Node harness.** Once steps
   1–2 land, extend this spike's harness to call `agent_sdk::run` (via a new
   `#[wasm_bindgen]` entrypoint in a `warp`-dependent cdylib) with a real
   `WARP_API_KEY` against a server, and observe a single MAA-style turn
   complete end-to-end inside the Node-hosted WASM module. That would convert
   this spike from "CLI layer works" to "agent loop works sandboxless in Node."

## Rough resource notes

- **Startup:** ~18–19ms from `require()` to the first `agent_run_from_config`
  return (Node 25, release wasm, warm filesystem cache). This is the
  orchestrator "tick" cost — loading the module and dispatching one command.
- **Memory:** ~65MB RSS (Node process + instantiated WASM); ~3.8MB JS heap,
  ~8MB external (WASM linear memory backing). The WASM linear memory grows on
  demand; the CLI layer barely uses it.
- **Artifact size:** 90MB raw `warp_cli_wasm.wasm` (release *with debuginfo*),
  **~5MB after `wasm-bindgen` processing** (`_bg.wasm`, debuginfo stripped,
  dead-code-eliminated). A `release-wasm`-profile build (opt-level `s`, LTO)
  would be smaller; this spike used the default `release` profile. Run
  `wasm-opt -Oz` on the processed artifact for further size reduction if
  needed.
- **Performance concerns:** none observed for the CLI layer. The real cost
  will be the `AppContext` bootstrap (step 2 above) — booting the full app
  context on every orchestrator tick would dwarf the 18ms module load, so the
  sandboxless model only wins if the bootstrap is either cached across ticks
  (keep the WASM instance alive between sub-agent runs) or slimmed to the
  headless subset. The "runs only in the gaps" framing implies a long-lived
  orchestrator WASM instance, which amortizes the bootstrap — consistent with
  the cheap-startup numbers above.

## Crate layout

- `src/lib.rs` — the `#[wasm_bindgen]` entrypoints (`agent_run_from_config`,
  `agent_run_from_argv`, `http_get`) and the host `fetch` binding.
- `harness/run.mjs` — the Node harness that loads the generated module and
  exercises the entrypoints.
- `script/wasm/build-cli-wasm` — build + `wasm-bindgen --target nodejs` helper.
- `Cargo.toml` — the two target-gated feature shims (`getrandom js`,
  `web-sys Window/Navigator`).
