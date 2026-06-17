# StandardizedPath Tech Spec

## Problem Statement
The codebase currently uses `CanonicalizedPath` (a wrapper around `PathBuf` that calls `dunce::canonicalize`) and raw `PathBuf` to represent file paths in coding features (file tree, editor tabs, repo metadata, AI context). This has three fundamental issues:

1. **`PathBuf` assumes local OS** — `std::path::PathBuf` uses the host platform's path encoding. A macOS client cannot represent a `/home/user/project/main.rs` path from a remote Linux session as a native `PathBuf` without conflating path separators or case-sensitivity rules.
2. **Canonicalization requires the file to exist** — `dunce::canonicalize` performs syscalls (`realpath`). It fails for deleted files, and resolves symlinks in ways we may not want (e.g. a user opens `~/projects/mylink/foo.rs` via a symlink, but canonicalization resolves it to `/real/path/foo.rs`, breaking the user's mental model).
3. **Canonicalization has I/O cost** — every `TryFrom<PathBuf> for CanonicalizedPath` makes a filesystem call. This is unnecessary for paths that are already normalized.

## Current State

### `CanonicalizedPath` (repo_metadata/src/lib.rs)
- Wraps `PathBuf`, constructed via `dunce::canonicalize`.
- Implements `Display`, `Hash`, `Eq`, `Borrow<Path>`, `Borrow<PathBuf>`.
- Four `TryFrom` impls (`PathBuf`, `&Path`, `&PathBuf`, `&str`), all call `dunce::canonicalize`.
- Used as the key in `LocalRepoMetadataModel.repositories: HashMap<CanonicalizedPath, IndexedRepoState>` and `lazy_loaded_paths`.
- Used in `Repository.root_dir`, `external_git_directory`, `common_git_directory`.
- Used in `RepositoryIdentifier::Local(CanonicalizedPath)`.
- Used in `DetectedRepositories` for repo root tracking.

### `RemoteRepositoryIdentifier` (repo_metadata/src/repository_identifier.rs)
- Uses raw `PathBuf` for the remote path (cannot canonicalize remotely).
- Pairs with `SessionId` for disambiguation.

### `typed-path` crate (v0.10.0, already a workspace dependency)
- Already used in `warp_util::path` for MSYS2/WSL path conversion and in `ai::paths` for cross-platform path joining and normalization.
- Provides `TypedPathBuf` (enum over Unix/Windows path buffers), `TypedPath`, `.normalize()` (removes `.` and `..` without I/O), and platform-aware path operations.

### Path usage in coding features
- **File tree**: `FileTreeEntry` stores `Arc<Path>` for root and per-entry paths. `FileTreeEntryState` variants hold `Arc<Path>` per node.
- **Editor tabs**: `CodeSource` variants hold `PathBuf`. `CodeManager` deduplicates via `HashMap<CodeSource, CodePaneData>` keyed on path equality.
- **Editor events**: `CodeViewEvent::FileOpened { file_path: PathBuf }`, `TabChanged { file_path: Option<PathBuf> }`.

## Proposed Changes

### 1. Introduce `StandardizedPath`
New struct in `warp_util::path` (or a new `warp_util::standardized_path` module), wrapping `TypedPathBuf`:

```rust
/// A normalized, platform-aware path that does not require the file to exist.
///
/// Unlike `CanonicalizedPath`, construction does NOT perform filesystem I/O.
/// Normalization removes `.` and `..` segments and collapses separators, but
/// does not resolve symlinks or verify existence.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct StandardizedPath(TypedPathBuf);
```

#### Construction APIs
All constructors enforce that the path is absolute and return `Result` so callers don't need to pre-validate input.

```rust
impl StandardizedPath {
    /// Create from a string, inferring Unix vs Windows encoding.
    /// Normalizes the path (removes `.`/`..`, collapses separators).
    /// Returns an error if the path is not absolute.
    pub fn try_new(path: &str) -> Result<Self, InvalidPathError> { ... }

    /// Create with an explicit path type (Unix or Windows).
    /// Returns an error if the path is not absolute.
    pub fn try_with_encoding(path: &str, path_type: PathType) -> Result<Self, InvalidPathError> { ... }

    /// Create from a local `std::path::Path`, inferring encoding from
    /// the compile target. Normalizes but does NOT canonicalize.
    /// Returns an error if the path is not absolute.
    pub fn try_from_local(path: &Path) -> Result<Self, InvalidPathError> { ... }

    /// Create from a local path with full canonicalization (resolves
    /// symlinks, verifies existence). This is the I/O-performing
    /// equivalent of `CanonicalizedPath::try_from`.
    /// Use at shell boundaries when receiving paths from the OS.
    /// The resulting path is always absolute (canonicalization produces absolute paths).
    /// Returns an error if canonicalization fails (e.g. path does not exist).
    pub fn from_local_canonicalized(path: &Path) -> io::Result<Self> { ... }
}
```

#### Query APIs

```rust
impl StandardizedPath {
    /// Returns the underlying `TypedPathBuf`.
    pub fn as_typed_path(&self) -> TypedPath<'_> { ... }

    /// Returns the file name component, if any.
    pub fn file_name(&self) -> Option<&str> { ... }

    /// Returns the extension, if any.
    pub fn extension(&self) -> Option<&str> { ... }

    /// Returns the parent path, if any.
    pub fn parent(&self) -> Option<StandardizedPath> { ... }

    /// Whether this path starts with the given prefix.
    pub fn starts_with(&self, base: &StandardizedPath) -> bool { ... }

    /// Join a relative segment.
    pub fn join(&self, segment: &str) -> StandardizedPath { ... }

    /// Whether the path uses Unix encoding.
    pub fn is_unix(&self) -> bool { ... }

    /// Whether the path uses Windows encoding.
    pub fn is_windows(&self) -> bool { ... }

}
```

#### Conversion APIs

```rust
impl StandardizedPath {
    /// Convert to a local `PathBuf` if the encoding matches the current OS.
    /// Returns `None` for a Unix-encoded path on Windows or vice versa.
    pub fn to_local_path(&self) -> Option<PathBuf> { ... }
}

impl Display for StandardizedPath {
    /// Displays the path string using the path's native separators.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.to_string_lossy())
    }
}

impl From<CanonicalizedPath> for StandardizedPath { ... }
```

#### Display safety — UNC prefix handling
`Display` delegates to `TypedPathBuf::to_string_lossy()`. UNC prefixes (`\\?\`) are handled as follows:

- Non-canonicalizing constructors (`try_new`, `try_with_encoding`, `try_from_local`) use `TypedPathBuf::normalize()` — pure string manipulation that never introduces UNC prefixes.
- `from_local_canonicalized` uses `dunce::canonicalize`, which strips UNC prefixes on Windows *when safe*. However, `dunce` intentionally **preserves** the UNC prefix for paths that exceed 260 characters, contain reserved DOS filenames (`CON`, `NUL`, `COM1`–`COM9`, etc.), or have components with trailing spaces/dots. In these edge cases, the UNC prefix will survive into the `TypedPathBuf`.
- `TypedPathBuf` itself stores the literal string representation and never calls Windows APIs.

To ensure `Display` never emits UNC prefixes, `from_local_canonicalized` will call `dunce::simplified` on the canonicalized path before feeding it into `TypedPathBuf`. This strips the UNC prefix whenever safe. For the rare cases where `dunce::simplified` cannot strip it (path >260 chars, reserved names), the UNC prefix will remain — this is correct behavior, as such paths require the extended-length prefix to be valid on Windows.

#### Serde support
Serialize as the string representation; deserialize by normalizing.

### 2. `StandardizedPath` does NOT encode local vs remote
`StandardizedPath` is agnostic to whether the path refers to a local or remote file. Remote context is handled by a separate wrapper at the model layer:

```rust
/// A path on a specific remote session.
pub struct RemotePath {
    pub session_id: SessionId,
    pub path: StandardizedPath,
}
```

This replaces the current `RemoteRepositoryIdentifier { session_id, path: PathBuf }` with `RemoteRepositoryIdentifier { session_id, path: StandardizedPath }`.

Rationale:
- Avoids invalid states where a single data structure (e.g. a file tree) holds paths from different sessions.
- Avoids duplicating `SessionId` across every path in structures that are inherently single-session.
- The model layer decides whether `StandardizedPath` or `RemotePath` is appropriate for each context.

### 3. Shell boundary canonicalization
When the app receives a path from the shell (e.g. via `cwd` reporting, file link clicks, CLI args), convert it using `StandardizedPath::from_local_canonicalized()`. This is the only place where I/O-based canonicalization occurs.

For remote paths received over the wire, infer the encoding from the remote OS and use `StandardizedPath::try_with_encoding(path, path_type)` — no I/O possible.

### 4. Migration strategy for `CanonicalizedPath`

**Phase 1 (this work):**
- Add `StandardizedPath` to `warp_util`.
- Add `From<CanonicalizedPath> for StandardizedPath` bridge.
- Update `RepositoryIdentifier` to use `StandardizedPath` in the `Local` variant.
- Update `RemoteRepositoryIdentifier.path` from `PathBuf` to `StandardizedPath`.
- Update `Repository.root_dir` and related fields.

**Phase 2 (follow-up):**
- Migrate `FileTreeEntry` and `FileTreeEntryState` path storage from `Arc<Path>` to `Arc<StandardizedPath>`.
- Migrate `CodeSource` variants from `PathBuf` to `StandardizedPath`.
- Migrate `CodeViewEvent` path fields.
- Migrate `LocalRepoMetadataModel.repositories` key.
- Migrate `DetectedRepositories` internals.

**Phase 3 (cleanup):**
- Remove `CanonicalizedPath` entirely once all consumers are migrated.
- Audit and remove any remaining raw `PathBuf` usage in coding feature paths.

## Design Decisions

**Why `TypedPathBuf` over a custom representation?**
`typed-path` (already a dependency) provides battle-tested cross-platform path normalization, component iteration, and encoding detection. Wrapping it avoids reimplementing path normalization and separator handling.

**Why not just use `TypedPathBuf` directly?**
A newtype wrapper provides:
- A guarantee that the inner path is always normalized (enforced at construction).
- A place to hang domain-specific methods (e.g. `from_local_canonicalized`, `to_local_path`).
- Clearer API boundaries — callers can't accidentally construct un-normalized paths.

**Why keep `to_local_path() -> Option<PathBuf>`?**
Filesystem operations (`std::fs::read`, `std::fs::write`, `notify` watchers) require `std::path::Path`. `to_local_path()` is the controlled exit point. Returning `Option` makes encoding mismatches explicit (e.g. trying to open a Unix path on Windows).

**Case sensitivity**
`StandardizedPath` does not perform case-folding. On macOS (case-insensitive HFS+/APFS), two paths differing only in case will not be equal under `StandardizedPath`. This matches the behavior of `TypedPathBuf` and avoids platform-specific equality semantics leaking into the type. If case-insensitive deduplication is needed (e.g. for file tree keys on macOS), it should be handled at the call site or via a separate wrapper/comparator.

## Resolved Questions
1. **`Borrow<TypedPath>` — no, use `as_typed_path()` instead.** `TypedPath<'a>` is a sized enum, not an unsized type like `std::path::Path`, so `Borrow::borrow()` cannot return `&TypedPath<'_>` (the value is a temporary from `TypedPathBuf::as_path()`). The `CanonicalizedPath` → `Borrow<Path>` precedent does not transfer. Use the `as_typed_path()` method for access to the underlying `TypedPath`.
2. **Arc wrapping — `Arc<StandardizedPath>`.** File tree entries will use `Arc<StandardizedPath>` to match the current `Arc<Path>` sharing pattern between parent-child nodes.
3. **Case sensitivity — not handled by `StandardizedPath`.** `StandardizedPath` does not perform case-folding. On case-insensitive filesystems (e.g. macOS HFS+/APFS), callers relying on `from_local_canonicalized` will get OS-canonical casing, but other constructors preserve input casing. Case-insensitive deduplication is a call-site concern.
4. **Absolute paths only.** `StandardizedPath` enforces absolute paths. All constructors return errors on relative input. Relative path display (e.g. relative to repo root) is a presentation concern handled at the call site.
