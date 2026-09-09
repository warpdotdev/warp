# Nested spacectl discovery and bounded concurrent detection

Linear: [REMOTE-3146](https://linear.app/warpdotdev/issue/REMOTE-3146/discover-nested-build-tools-with-concurrent-spacectl-cache-setup)

Originating Slack thread:
[C0BDQDW8V5E / 1788767403.717799](https://warpdev.slack.com/archives/C0BDQDW8V5E/p1788767403717799)

Code references use warp commit
[`51242b5f0af80fff81613ff6561eed29ba8922fa`](https://github.com/warpdotdev/warp/tree/51242b5f0af80fff81613ff6561eed29ba8922fa)
on `master`.

## Summary

Build-cache setup detects tools only at each repository root. Nested projects are missed.
Implement one ordered blocking discovery producer that scans repositories for detector-aligned
markers and sends the bounded candidate set through a bounded channel into one shared concurrent
detector. Keep cache-directory creation single-file and keep all real mounts serial. Keep the
synthetic global mount last.

## Context

- [`prepare_environment_impl`](https://github.com/warpdotdev/warp/blob/51242b5f0af80fff81613ff6561eed29ba8922fa/app/src/ai/agent_sdk/driver/environment.rs#L373-L452)
  runs cache setup after cloning and before setup commands. Cache failures do not abort environment
  preparation.
- [`setup_caches`](https://github.com/warpdotdev/warp/blob/51242b5f0af80fff81613ff6561eed29ba8922fa/app/src/ai/agent_sdk/driver/cache_setup.rs#L44-L112)
  creates one `RepositoryCacheSource` per checkout and reports invocation failures.
- [`setup_cache`](https://github.com/warpdotdev/warp/blob/51242b5f0af80fff81613ff6561eed29ba8922fa/crates/build_cache/src/lib.rs#L450-L646)
  detects repositories serially, constructs a plan, and applies every mount serially.
- [`construct_plan`](https://github.com/warpdotdev/warp/blob/51242b5f0af80fff81613ff6561eed29ba8922fa/crates/build_cache/src/lib.rs#L688-L773)
  appends a global union of all detected modes. The global configuration must remain last because a
  mode can mix cwd-relative paths with shared paths.
- [`run_spacectl_mount`](https://github.com/warpdotdev/warp/blob/51242b5f0af80fff81613ff6561eed29ba8922fa/crates/build_cache/src/spacectl.rs#L116-L193)
  uses the command cwd for both detection and mounting. The default runner applies a 60-second
  timeout and `kill_on_drop(true)`.
- `spacectl` 0.12.2 detection is cwd-only. A local triage fixture measured four detects at about
  196 ms serially and 127 ms concurrently. These measurements show benefit, not a performance
  guarantee.

## Technical design

### 1. Produce candidate roots with `walkdir`

Add `walkdir.workspace = true` to `crates/build_cache/Cargo.toml`. In one
`tokio::task::spawn_blocking` task, sort `RepositoryCacheSource` values by `RepoCacheKey`, send each
repository root first, then drive one `walkdir::WalkDir` iterator for that repository. All traversal
state stays local to the blocking task.

Configure each iterator with:

- `min_depth(1)`, because the producer handles the always-included root separately;
- `max_depth(7)`, which bounds traversal while still reaching `.config/mise/config.toml` for a
  candidate root at depth 4;
- `follow_links(false)` and `follow_root_links(false)`;
- `sort_by_file_name()`; and
- `into_iter().filter_entry(...)` to reject ignored directory entries and symlink entries before
  descent.

`WalkDir` yields a directory before its contents and uses depth-first traversal. Sorted sibling
names therefore define deterministic depth-first selection. Do not reconstruct breadth-first
traversal around `WalkDir`; that would restore a custom directory queue and defeat the reuse.
Changing from breadth-first to depth-first can change which 32 roots a truncated repository retains.
This is intentional and is covered by fixtures.

Apply these limits and error rules:

- Always include the repository root. It has depth 0 and does not count against the child limit.
- Accept every candidate whose marker is reached by the bounded walk. Do not apply another
  candidate-depth restriction after traversal.
- Visit at most 10,000 non-ignored, non-symlink directories per repository, including the root.
  Files do not count against this limit.
- Retain at most 32 child candidates per repository. When a 33rd distinct child candidate is found,
  mark the scan truncated and stop that repository's iterator.
- When the directory limit is reached with iterator work remaining, mark the scan truncated and stop
  that repository's iterator.
- On truncation, retain the root and the deterministic candidates already yielded. Continue cache
  setup.
- Reject a directory entry in `filter_entry` when its name is `.git`, `node_modules`, `target`,
  `Pods`, `vendor`, `dist`, `build`, `.venv`, `.tox`, or `DerivedData`. The rejected directory and
  its subtree do not count as visited. Do not reject a file with one of these names.
- Reject symlink entries in `filter_entry`. `follow_links(false)` prevents descent through nested
  links. `follow_root_links(false)` prevents the special default behavior that otherwise follows a
  symlink passed as the traversal root. A symlink is not a marker.
- Handle every `walkdir::Error` in place, emit a warning with `Error::depth()` and the underlying
  `io::ErrorKind` when present, then continue iteration. `WalkDir` does not descend when it cannot
  open a directory. Do not log raw error paths. A missing or unreadable repository root still
  proceeds to root detection, which preserves the existing per-invocation error path.
- Do not set `max_open`; use the crate's bounded default. This setting changes the file-descriptor
  versus memory trade-off, not yielded results.

Normalize a child root into a `PathBuf` by stripping the repository root and accepting only
non-empty normal UTF-8 components. Preserve case and Unicode bytes. Skip a child path that is
non-UTF-8 or contains a root, prefix, `.` or `..` component. Do not canonicalize child paths or
resolve symlinks. Hash the normalized path's `OsStr` encoded byte slice directly. Accepted UTF-8
paths have the same encoding on Namespace Linux and macOS.

Deduplicate exact normalized roots. A directory with multiple markers is one candidate. Retain both
a parent project root and a nested project root when each has a marker.

### 2. Align marker rules with spacectl

The marker table must mirror the detector inputs in the spacectl version shipped on Namespace
workers. For spacectl 0.12.2, use these rules:

- Exact entries: `Brewfile`, `bun.lock`, `Podfile`, `composer.json`, `deno.lock`, `go.mod`,
  `go.work`, `.golangci.yml`, `.golangci.yaml`, `gradlew`, `build.gradle`, `pom.xml`, `mise.toml`,
  `.mise.toml`, `.tool-versions`, `flake.nix`, `shell.nix`, `default.nix`, `package-lock.json`,
  `pnpm-lock.yaml`, `poetry.lock`, `requirements.txt`, `Gemfile`, `Cargo.toml`, `Package.swift`,
  `Tuist.swift`, `tuist.toml`, `uv.lock`, and `yarn.lock`.
- Exact relative entries: `mise/config.toml`, `.mise/config.toml`, `.config/mise.toml`, and
  `.config/mise/config.toml`. The candidate is the ancestor from which spacectl checks that relative
  path, not the marker's immediate parent.
- Directory entry: `Tuist`.
- Suffix entries: directories ending in `.xcodeproj` or `.xcworkspace`.

For exact, directory, and suffix entries, the candidate is the directory that contains the matched
entry. A marker entry is never itself the candidate. When one file matches multiple marker rules,
select only the longest relative marker so `.config/mise.toml` and
`.config/mise/config.toml` identify the directory containing `.config`.

Do not add looser markers that 0.12.2 does not use, including bare `package.json`,
`pyproject.toml`, `settings.gradle`, or `build.gradle.kts`. Tool-binary checks remain spacectl's
responsibility. Binary-only modes such as `apt`, Kotlin Native, and Playwright are discovered at the
always-included repository root; they do not cause child candidates.

Before implementation, verify the worker's shipped spacectl version and compare its provider source
with this table. If detector semantics differ, update this spec and the table in the same PR.

### 3. Prepare stable isolated cache roots

After receiving a candidate and before yielding its detection future, create that candidate's
configuration root and await any permission fallback. The receiving stream prepares only one
directory at a time. A preparation may overlap already-running dry-run detections, but it must not
overlap another preparation or any real mount. This overlap is safe because each candidate has a
distinct cache root, and dry-run detection does not apply mounts. A creation failure yields a keyed
non-fatal degradation result for that candidate and does not schedule spacectl.

- Preserve the current root cache path: `repos/<repo-key>`.
- Use `repos/<repo-key>/nested/<stable-id>` for a child root.
- Compute `<stable-id>` as lowercase hexadecimal SHA-256 of the normalized relative path's encoded
  bytes. Do not hash an absolute checkout path.
- Validate that all configuration cache paths are safe relative paths and unique.
- If two distinct roots produce the same configuration path, reject the plan before real mounts,
  record one non-fatal plan-invariant degradation, and continue environment preparation. Never share
  the path.

This scheme preserves existing root cache hits and isolates equal relative mount names such as
`frontend/target` and `backend/target`.

### 4. Pipeline candidates through one shared detector limit

Add `futures.workspace = true` and Tokio with its `rt` and `sync` features to the normal dependencies
in `crates/build_cache/Cargo.toml`. Use `tokio::task::spawn_blocking` for the synchronous filesystem
walk and a `tokio::sync::mpsc` channel with capacity 8 between discovery and the async receiving
stream. Use `futures::stream::StreamExt::buffer_unordered(8)` as the detection-concurrency primitive.
Do not add a custom semaphore.

The blocking producer owns the sorted repositories and keeps the current `WalkDir`, counters,
deduplication state, and scan diagnostics as local variables. It uses `blocking_send`, so a full
channel blocks traversal instead of accumulating an unbounded candidate queue. The async receiver
prepares each candidate's cache directory serially and yields its detection future. Apply
`buffer_unordered(8)` once to this stream and collect the results.

- The buffer's limit of 8 is the only detection limit and is shared across all repositories.
- At most eight yielded detection futures are in flight. The receiver pulls and prepares another
  candidate only when the detector buffer has capacity. At most eight additional unprepared
  candidates wait in the bounded channel; no unbounded candidate queue or channel is permitted.
- Selection remains deterministic even though production is demand-driven. The producer alone
  advances each sorted `WalkDir` iterator and applies that repository's 10,000-directory and
  32-child limits. Detection completion order can change when production resumes, but it cannot
  change the next candidate selected.
- Change the command hook from exclusive `FnMut` use to a concurrency-safe `Fn` shape. Wrap it in
  `Arc` inside `setup_cache`; each yielded future owns an `Arc` clone. Pass shared references to
  both directory preparation and spacectl invocation. Tests must put mutable fake-runner state
  behind shared synchronization such as `Arc<Mutex<...>>`; do not serialize the production
  scheduler behind the fake-runner API.
- Run `spacectl cache mount --detect='*' --dry_run=true` with each candidate as cwd and its isolated
  cache root.
- Preserve the 60-second timeout and `kill_on_drop(true)` for every invocation.
- Dropping cache setup drops the channel receiver. A producer blocked in `blocking_send` wakes with
  an error and exits; between sends it checks `Sender::is_closed()` on each `WalkDir` entry and
  exits. Tokio cannot forcibly abort a running `spawn_blocking` closure, so an in-progress
  filesystem operation must return before the closure observes receiver closure. No producer task
  or queued candidate keeps cache setup resources alive after that point.
- An invocation failure, timeout, malformed response, or empty mode set affects only that root.
- Do not cancel siblings after a failure.
- Attach the canonical key `(RepoCacheKey, root-first flag, normalized child path)` to each
  preparation failure and detection result. Sort all keyed results before constructing the plan or
  returning the report. Completion order must not affect the plan, mount order, environment
  overlay, telemetry report order, or truncation result.

### 5. Plan and apply mounts serially

Create one repository-scoped `CacheConfiguration` for every successful non-empty detection. Multiple
configurations may share a `RepoCacheKey`, but every configuration must have a unique cwd and cache
directory.

- Update `CacheSetupPlan::validate` and its documentation to permit repeated ordered repository keys
  and require unique repository configuration paths.
- Sort repository configurations by repo key, then root before child, then normalized child path.
- Union all successful detected modes with `additional_global_modes` for one global configuration.
- Run every real repository mount serially in canonical plan order.
- Run the global mount serially after all repository mounts.
- Create the global cache directory serially.
- Preserve current last-successful-repository environment overlay behavior and global-environment
  precedence. Resolve any duplicate repository environment keys by canonical plan order.
- Preserve `prepare_environment_impl` behavior: any cache degradation is reported, but environment
  preparation continues.

Do not attempt concurrent real mounts in v1. Rust, for example, can combine `./target` with shared
Cargo paths. Concurrent mounts can race even when cache-root leaves differ.

Nested discovery applies wherever the existing build-cache gate enables setup. V1 must work on
Namespace Linux and macOS without enabling caching on any new platform. Keep filesystem helpers and
unit tests platform-neutral so the crate continues to compile on other supported targets.

### 6. Logging and telemetry

Create one child span for the whole discovery process and one child span per repository. Record
visited directory count, selected child count, and truncation reason (`directory_limit` or
`candidate_limit`) on each repository span. Record total scheduled detects and the configured
detection limit on the cache-setup span.

Add the stable child ID to detection spans. Do not put raw absolute checkout paths in safe logs or
Sentry extras. Emit one warning per truncated repository and one privacy-safe warning with error
depth and `io::ErrorKind` per unreadable entry. Expected limit truncation is non-fatal and must not
cancel detection or mounting. Create the whole-discovery span under the active cache-setup span and
enter it in the blocking closure so every repository discovery span and warning remains in the
setup trace.

## Decisions

- **Marker scan instead of spacectl in every directory.** A bounded marker scan avoids process spam
  and matches cwd-based detector semantics. Calling spacectl for every directory was rejected
  because repository breadth and 60-second per-process timeouts make latency unbounded.
- **Pipeline discovery into detection.** Completing every scan before detection is simpler, but it
  adds scan latency to the critical path and retains the full candidate set. One blocking producer
  and a bounded channel keep traversal state local while providing backpressure. Selection limits
  belong only to producer state, each cache root is prepared before its future is yielded, and
  keyed results are sorted after completion.
- **Use `buffer_unordered` instead of a custom limiter.** The workspace already depends on
  `futures`. `StreamExt::buffer_unordered(8)` directly bounds a stream of detection futures and
  provides backpressure. A custom futures semaphore would duplicate this behavior.
- **Use the app's Tokio runtime for blocking discovery.** `build_cache` is a native-only dependency
  of the app, whose native runtime is Tokio. `spawn_blocking` removes the boxed iterator and
  resumable discovery structs without adding a runtime to wasm builds. Standalone native callers,
  including `validate_spacectl`, must enter a Tokio runtime before calling `setup_cache`.
- **Use sorted depth-first `WalkDir` traversal.** `WalkDir` supplies bounded descriptors, depth
  limits, symlink controls, subtree filtering, and recoverable errors. Retaining breadth-first
  selection would require a custom queue. Sorted depth-first selection is deterministic and makes
  the 32-root truncation policy explicit.
- **Eight shared detection slots.** This captures the measured concurrency benefit while bounding
  process and detector fan-out. A per-repository limit was rejected because multiple repositories
  could exceed the intended host-wide limit.
- **Serial real mounts.** Concurrent mounts were rejected for v1 because isolated cache leaves do
  not isolate shared destination paths. The global mount remains last.
- **Preserve the root cache path.** Moving all roots under a new namespace was rejected because it
  would discard existing root cache hits.
- **Hash normalized child paths.** Raw relative paths are easier to inspect but can be long and
  platform-sensitive. SHA-256 of the path's encoded bytes produces a stable safe component across
  Namespace Linux and macOS. Telemetry retains the stable ID for correlation.

## Assumptions

- The Namespace worker still ships spacectl detector semantics equivalent to 0.12.2. Implementation
  must verify this before coding.
- Repository-relative project paths are UTF-8. A non-UTF-8 child path is skipped rather than given a
  platform-specific cache identity.
- The current cache setup remains before user setup commands. Tools installed only by setup commands
  remain unavailable to detection.
- Overlapping real spacectl mounts are not proven safe on Linux or macOS. V1 does not rely on that
  behavior.
- `WalkDir::sort_by_file_name()` is deterministic for a fixed filesystem and platform. Cross-platform
  traversal order for non-UTF-8 entry names is not part of the cache identity contract; such paths
  cannot become candidates.

## Out of scope

- Recursive detection changes in spacectl or Namespace.
- Calling spacectl in directories without detector-aligned markers.
- Moving cache setup after user setup commands.
- Concurrent cache-directory creation or real mount invocations. Cache-directory preparation may
  overlap dry-run detection for a different candidate.
- New detectors or support for looser manifests that the shipped spacectl does not recognize.
- UI changes or computer-use verification.

## Validation criteria

1. `cargo nextest run -p build_cache` passes and includes unit coverage for:
   - representative direct markers and every relative, directory, and suffix marker rule;
   - non-markers such as bare `package.json`, ignored trees, and symlinks;
   - exact deduplication while retaining marked parent and child roots;
   - sorted depth-first `WalkDir` selection, the 10,000-directory limit, and 32 children plus root;
   - deterministic truncation and unreadable-entry isolation;
   - `max_depth(7)` finding `.config/mise/config.toml` for a depth-4 candidate and accepting deeper
     candidates whose markers the walk reaches;
   - ignored directory subtrees, symlinked nested directories, and a symlink traversal root are not
     followed;
   - stable Linux/macOS child IDs, preserved root cache paths, and unique safe cache paths;
   - a fake runner that observes more than one and no more than eight simultaneous detects across
     multiple repositories;
   - detection starts before the final scan completes, cache-directory preparations never overlap,
     and a preparation can overlap an active dry-run detection;
   - producer backpressure keeps at most eight queued candidates and at most eight detection
     futures in flight, receiver drop stops the blocking producer, and selection is identical
     across deliberately permuted completion orders;
   - per-root failure and timeout isolation, `kill_on_drop`, deterministic keyed report ordering,
     serial mount execution, and the global mount last;
   - repeated ordered repository keys and unique cache-directory plan invariants.
2. `cargo nextest run -p warp cache_setup` passes to confirm Namespace gating, source mapping,
   degradation reporting, and environment export behavior remain compatible.
3. Extend `crates/build_cache/examples/validate_spacectl.rs` with one repository containing root,
   `frontend`, and `backend` fixtures. With the worker's spacectl version available,
   `cargo run -p build_cache --example validate_spacectl -- --reset` must run within the validator's
   Tokio runtime without a missing-reactor panic and show:
   - one detect per selected root;
   - the expected nested modes;
   - distinct nested cache roots;
   - serial real mounts in canonical order;
   - one final global mount.
4. Record five-run medians for a 32-child fixture with serial detection and the concurrency-8
   implementation on a Namespace Linux worker. Concurrent median wall time must not exceed the
   serial median. Record scan time and process counts; do not add a hardware-dependent unit-test
   latency threshold.
5. Verify the marker table against the exact spacectl provider source deployed on the validation
   worker. Link the source tag or commit in the implementation PR.
6. Before any follow-up enables concurrent real mounts, run controlled overlapping-mount tests for
   mixed relative/global modes on Namespace Linux and macOS. V1 passes without this experiment
   because all real mounts remain serial.
7. Run `./script/format`, the clippy command selected by `./script/presubmit`, and `git diff --check`
   before implementation review. No computer-use artifact is required.

## Parallelization

Use one implementer for discovery, plan changes, runner refactoring, and unit tests because these
changes share the `setup_cache` contract and fake-runner seam. After unit tests pass, Linux timing
validation and the optional macOS mount-safety investigation can run independently. Land all spec,
implementation, and validation updates in this PR.
