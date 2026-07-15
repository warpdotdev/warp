//! Windows-host-side WSL routing for subprocess invocations made
//! through this crate's [`Command`](crate::r#async::Command) and
//! [`Command`](crate::blocking::Command) wrappers.
//!
//! On a native Windows build, Warp represents the working directory of a
//! WSL session as a `\\wsl$\<distro>\...` UNC path. Running the Windows
//! `git.exe` against such a path is broken: it triggers "dubious
//! ownership" errors, produces bogus diffs, and can hang. The fix is to
//! run the Linux-side git inside the distribution instead.
//!
//! [`translate_for_wsl_unc_cwd`] detects a bare `git` invocation whose
//! working directory is a WSL UNC path and rewrites it to
//! `wsl.exe --distribution <distro> --cd <linux_path> --exec git <args...>`.
//! `gh` is intentionally left untouched (see issue #8410 follow-up).
//!
//! The parsing and rewriting logic is expressed as pure functions so it
//! can be compiled and unit-tested on every platform; only the
//! spawn-time hook in the wrappers is gated on `#[cfg(windows)]`.
//!
//! The `WSLENV=<K>/u:...` propagation format mirrors the precedent in
//! `app/src/terminal/model/session/command_executor/wsl_command_executor.rs`.

use std::ffi::{OsStr, OsString};
use std::path::Path;

#[cfg(test)]
#[path = "wsl_windows_tests.rs"]
mod tests;

/// A working directory that lives inside a WSL distribution, expressed
/// on the Windows host as a UNC path.
///
/// Public as a pure, testable value type. Beyond the spawn-time hook in
/// this crate, it is expected to be reused from the app layer for the
/// #6645 (`@` attachment detection) and #8410 (Code Review diff)
/// follow-ups, which need the same UNC-to-Linux mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslUncPath {
    /// The distribution name exactly as it appears in the UNC path
    /// (case preserved).
    pub distro: String,
    /// The corresponding Linux absolute path, using `/` separators. A
    /// UNC path that points at the distribution root maps to `/`.
    pub linux_path: String,
}

/// The rewritten command produced by [`translate_for_wsl_unc_cwd`].
///
/// Public together with [`translate_for_wsl_unc_cwd`] so the pure
/// rewriting logic can be unit-tested (mirroring the "exposed for unit
/// testing" precedent on
/// [`resolve_binary_in_wsl_safe_path`](crate::wsl::resolve_binary_in_wsl_safe_path))
/// and reused from the app layer for the #6645 / #8410 follow-ups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslatedCommand {
    /// The program to spawn (always `wsl.exe`).
    pub program: OsString,
    /// The full argument vector for `wsl.exe`.
    pub args: Vec<OsString>,
    /// The value for the `WSLENV` variable that propagates the
    /// explicitly-set environment variables into the distribution, or
    /// `None` when no environment variables were set.
    pub wslenv: Option<String>,
}

/// The verbatim UNC prefix (`\\?\UNC\`), normalized to forward slashes.
const VERBATIM_UNC_PREFIX: &str = "//?/UNC/";

/// The host components that identify a WSL UNC path. Matched the same
/// way `wsl.rs`'s `KNOWN_NAMES` is: a plain slice of string literals.
const WSL_HOSTS: &[&str] = &["wsl$", "wsl.localhost"];

/// Parses a WSL UNC path into its distribution and Linux path. Accepts
/// the `\\wsl$\...`, `\\wsl.localhost\...`, verbatim `\\?\UNC\wsl$\...`,
/// and forward-slash `//wsl$/...` spellings; the host component is
/// matched case-insensitively. Returns `None` for non-WSL UNC paths,
/// drive-letter paths, and relative paths.
///
/// Pure — public so it can be unit-tested without a real WSL host (the
/// same rationale as
/// [`resolve_binary_in_wsl_safe_path`](crate::wsl::resolve_binary_in_wsl_safe_path))
/// and reused from the app layer for the #6645 / #8410 follow-ups.
pub fn parse_wsl_unc_path(path: &Path) -> Option<WslUncPath> {
    parse_wsl_unc_str(path.to_str()?)
}

/// String-level implementation of [`parse_wsl_unc_path`]. Kept separate
/// so it can also parse individual command-line arguments, which may be
/// UNC paths in their own right.
fn parse_wsl_unc_str(raw: &str) -> Option<WslUncPath> {
    // Normalize both separators to `/` so the forward-slash and
    // backslash spellings share a single parser.
    let normalized = raw.replace('\\', "/");

    // Strip the leading UNC marker, handling the verbatim form first.
    let rest = match strip_prefix_ci(&normalized, VERBATIM_UNC_PREFIX) {
        Some(after_verbatim) => after_verbatim,
        None => normalized.strip_prefix("//")?,
    };

    // The host component runs up to the next separator; without a
    // separator there is no room for a distribution name.
    let (host, after_host) = match rest.split_once('/') {
        Some((host, after_host)) => (host, after_host),
        None => return None,
    };
    if !WSL_HOSTS.iter().any(|h| host.eq_ignore_ascii_case(h)) {
        return None;
    }

    // The distribution name is the next component; whatever follows is
    // the Linux path.
    let (distro, linux_rest) = match after_host.split_once('/') {
        Some((distro, linux_rest)) => (distro, linux_rest),
        None => (after_host, ""),
    };
    if distro.is_empty() {
        return None;
    }

    // Trailing separators are dropped; the distribution root becomes `/`.
    let trimmed = linux_rest.trim_end_matches('/');
    let linux_path = if trimmed.is_empty() {
        "/".to_string()
    } else {
        format!("/{trimmed}")
    };

    Some(WslUncPath {
        distro: distro.to_string(),
        linux_path,
    })
}

/// Case-insensitive [`str::strip_prefix`]. Uses [`str::get`] so a prefix
/// length that lands inside a multi-byte character is treated as a
/// non-match rather than panicking.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// Rewrites a bare `git` invocation whose working directory is a WSL UNC
/// path into an equivalent `wsl.exe` invocation. Returns `None` when the
/// command should be left unchanged (non-`git` program, path-qualified
/// `git`, or a working directory that is not a WSL UNC path).
///
/// `env` is the list of `(key, value)` environment variables explicitly
/// set on the command. Non-`PATH` variables are advertised through
/// `WSLENV` so they cross into the distribution (see [`build_wslenv`]).
///
/// An explicitly set `PATH` is instead carried as an argv element:
/// `... --exec /usr/bin/env PATH=<value> git <args...>`. This is the
/// `--exec` analogue of the inline `PATH=...; cmd` assignment in
/// `app/src/terminal/model/session/command_executor/wsl_command_executor.rs`.
/// Routing `PATH` through argv completely bypasses Windows' non-disableable
/// Windows-to-WSL `PATH` conversion, which would otherwise truncate a
/// caller-supplied Linux-form `PATH` (the value `run_git_command_with_env`
/// sets so hook tools such as `git-lfs` resolve inside the distribution).
///
/// Pure — public so the rewriting logic can be unit-tested (mirroring
/// the "exposed for unit testing" precedent on
/// [`resolve_binary_in_wsl_safe_path`](crate::wsl::resolve_binary_in_wsl_safe_path))
/// and reused from the app layer for the #6645 / #8410 follow-ups.
pub fn translate_for_wsl_unc_cwd(
    program: &OsStr,
    args: &[OsString],
    cwd: Option<&Path>,
    env: &[(OsString, OsString)],
) -> Option<TranslatedCommand> {
    if !is_bare_git(program) {
        return None;
    }
    let unc = parse_wsl_unc_path(cwd?)?;

    let mut translated_args = vec![
        OsString::from("--distribution"),
        OsString::from(&unc.distro),
        OsString::from("--cd"),
        OsString::from(&unc.linux_path),
        OsString::from("--exec"),
    ];
    // A caller-supplied `PATH` is prepended to the executed program as an
    // `env` assignment rather than propagated through `WSLENV`; see the
    // rationale above.
    if let Some(path_value) = env
        .iter()
        .find(|(key, _)| is_path_env_key(key))
        .map(|(_, value)| value)
    {
        let mut path_arg = OsString::from("PATH=");
        path_arg.push(path_value);
        translated_args.push(OsString::from("/usr/bin/env"));
        translated_args.push(path_arg);
    }
    translated_args.push(OsString::from("git"));
    for arg in args {
        translated_args.push(translate_arg(arg, &unc.distro));
    }

    Some(TranslatedCommand {
        program: OsString::from("wsl.exe"),
        args: translated_args,
        wslenv: build_wslenv(env),
    })
}

/// True when `program` is the bare name `git`. Path-qualified programs
/// (for example `/usr/bin/git` or `C:\Program Files\Git\git.exe`) do not
/// compare equal and are therefore left untouched.
fn is_bare_git(program: &OsStr) -> bool {
    match program.to_str() {
        Some(s) => s == "git",
        None => false,
    }
}

/// Rewrites a single argument: an argument that is itself a WSL UNC path
/// for the *same* distribution is converted to its Linux path, so paths
/// passed to git resolve inside the distribution. Arguments for other
/// distributions, non-UNC arguments, and non-UTF-8 arguments are passed
/// through unchanged.
fn translate_arg(arg: &OsStr, distro: &str) -> OsString {
    let Some(s) = arg.to_str() else {
        return arg.to_owned();
    };
    match parse_wsl_unc_str(s) {
        Some(parsed) if parsed.distro.eq_ignore_ascii_case(distro) => {
            OsString::from(parsed.linux_path)
        }
        Some(_) | None => arg.to_owned(),
    }
}

/// Builds the `WSLENV` value that advertises the explicitly-set
/// environment variables to the distribution, using the `/u` suffix so
/// each variable is shared when invoking WSL from Windows. Returns
/// `None` when no propagatable variables were set.
///
/// `PATH` is deliberately excluded (case-insensitively): Windows applies
/// a non-disableable Windows-to-WSL `PATH` conversion, and a `PATH` that
/// is already in Linux form — as it is when a WSL session's environment
/// is threaded through `run_git_command_with_env` — fails that
/// conversion and gets truncated. `PATH` is instead carried as an argv
/// element by [`translate_for_wsl_unc_cwd`]. This mirrors the `PATH`
/// handling in
/// `app/src/terminal/model/session/command_executor/wsl_command_executor.rs`.
fn build_wslenv(env: &[(OsString, OsString)]) -> Option<String> {
    let joined = env
        .iter()
        .map(|(key, _)| key)
        .filter(|key| !is_path_env_key(key))
        .map(|key| format!("{}/u", key.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(":");
    if joined.is_empty() {
        return None;
    }
    Some(joined)
}

/// True when `key` names the `PATH` environment variable, compared
/// case-insensitively. Used to keep a Linux-form `PATH` out of both
/// `WSLENV` and the environment handed to `wsl.exe` (see [`build_wslenv`]
/// and the `apply_wsl_unc_translation` hooks).
pub(crate) fn is_path_env_key(key: &OsStr) -> bool {
    key.to_str()
        .is_some_and(|key| key.eq_ignore_ascii_case("PATH"))
}
