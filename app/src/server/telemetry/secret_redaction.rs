//! Best-effort secret redaction for telemetry payloads.
//!
//! Unlike the AI-side secret redaction in `app/src/ai/blocklist/block/secret_redaction.rs`,
//! which is gated on the user's secret-redaction (a.k.a. "safe mode") setting and is used
//! for visual obfuscation in the terminal, the redaction in this module is unconditional:
//! we always do a redaction pass on telemetry payloads that may contain user-generated
//! content, regardless of the user's safe-mode setting. The two settings are deliberately
//! decoupled — visual obfuscation is a UX preference, while telemetry-side redaction is a
//! defence-in-depth measure for data leaving the device.
//!
//! The regex used for redaction always includes the default patterns defined in
//! `crate::terminal::model::secrets::regexes::DEFAULT_REGEXES_WITH_NAMES`. Any custom
//! patterns the user has configured (or that their organization has configured via
//! enterprise secret redaction) are layered on top of those defaults.
//!
//! This module is intentionally lightweight: it does byte-range matching only and does
//! not track `SecretLevel`s or character ranges, since the telemetry path doesn't need
//! either.
//!
//! ## Cache pool design
//!
//! `regex_automata::meta::Regex` maintains an internal pool of `Cache` objects that grows
//! to accommodate the peak number of *concurrent* callers. With many simultaneous async
//! tokio tasks each calling `redact_secrets_in_value`, the pool would create one cache per
//! concurrent task and retain them all — each cache for our 20-pattern regex consumes
//! hundreds of MiB, leading to multi-GB memory usage over time.
//!
//! The fix: instead of sharing one `Regex` (and therefore one pool) across all threads,
//! each OS thread keeps a *clone* of the regex in thread-local storage. Cloning a `Regex`
//! is cheap — the compiled NFA is shared via `Arc` — but each clone gets its own pool.
//! Because the thread-local clone is used by at most one task at a time per thread, the
//! pool for each clone never grows beyond a single cached `Cache`.
use std::cell::RefCell;
use std::collections::HashSet;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use lazy_static::lazy_static;
use parking_lot::RwLock;
use regex_automata::meta::Regex;
use serde_json::Value;

use crate::terminal::model::secrets::regexes::DEFAULT_REGEXES_WITH_NAMES;
const REDACTION_REPLACEMENT_CHARACTER: &str = "*";

/// Version counter for the global telemetry regex. Incremented each time
/// [`update_telemetry_secrets_regex`] successfully rebuilds the regex, so that
/// thread-local clones can detect staleness and refresh.
static REGEX_VERSION: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// Thread-local clone of the telemetry secrets regex.
    ///
    /// Each OS thread keeps its own clone with its own internal cache pool.
    /// Cloning a `regex_automata::meta::Regex` shares the compiled NFA via
    /// `Arc` (cheap) but gives the clone a fresh, empty pool. Because the
    /// thread-local clone is only ever used by one task at a time, the pool
    /// never accumulates more than one `Cache` entry, bounding the per-thread
    /// memory to a single cache regardless of peak concurrency.
    ///
    /// The `u64` field records the version of the global regex at the time
    /// this thread's clone was last refreshed, so stale clones are detected
    /// and updated on the next call.
    static THREAD_LOCAL_REGEX: RefCell<(u64, Option<Regex>)> = const { RefCell::new((0, None)) };
}

lazy_static! {
    /// Regex used to redact secrets from telemetry payloads. Initialized with the
    /// default patterns so that redaction works even before the user's privacy
    /// settings are loaded (and even for users who have never configured any
    /// custom patterns).
    static ref TELEMETRY_SECRETS_REGEX: RwLock<Regex> = RwLock::new(build_default_regex());
}
/// Builds a regex containing only the default patterns. Used to seed the static
/// regex before the privacy settings are loaded.
fn build_default_regex() -> Regex {
    let patterns: Vec<&str> = DEFAULT_REGEXES_WITH_NAMES
        .iter()
        .map(|d| d.pattern)
        .collect();
    Regex::new_many(&patterns).expect("default secret patterns should compile")
}
/// Rebuilds [`TELEMETRY_SECRETS_REGEX`] from the user's and enterprise's secret
/// regex lists, layered on top of the default patterns. The default patterns are
/// always included, so redaction works even when the user has not configured any
/// custom patterns.
pub fn update_telemetry_secrets_regex<'a, U, E>(user_secrets: U, enterprise_secrets: E)
where
    U: IntoIterator<Item = &'a regex::Regex>,
    E: IntoIterator<Item = &'a regex::Regex>,
{
    let patterns = compose_patterns(
        user_secrets.into_iter().map(regex::Regex::as_str),
        enterprise_secrets.into_iter().map(regex::Regex::as_str),
    );
    match Regex::new_many(&patterns) {
        Ok(regex) => {
            *TELEMETRY_SECRETS_REGEX.write() = regex;
            // Signal thread-local clones to refresh on their next use.
            REGEX_VERSION.fetch_add(1, Ordering::Release);
        }
        Err(err) => log::error!("Failed to build telemetry secrets regex: {err:?}"),
    }
}
/// Composes the full list of patterns to compile into the telemetry regex,
/// ordered enterprise → user → defaults, with later occurrences of an already-
/// seen pattern string deduped out.
fn compose_patterns<'a>(
    user: impl Iterator<Item = &'a str>,
    enterprise: impl Iterator<Item = &'a str>,
) -> Vec<&'a str> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut patterns: Vec<&str> = Vec::new();
    let all = enterprise
        .chain(user)
        .chain(DEFAULT_REGEXES_WITH_NAMES.iter().map(|d| d.pattern));
    for pattern in all {
        if seen.insert(pattern) {
            patterns.push(pattern);
        }
    }
    patterns
}
/// Replaces every detected secret in `input` with a run of asterisks of the same
/// byte length. Overlapping matches (which can occur when multiple patterns match
/// the same region) are merged before replacement, so each character is replaced
/// at most once.
pub fn redact_secrets_in_string(input: &mut String) {
    let ranges: Vec<Range<usize>> = with_thread_local_regex(|regex| {
        regex.find_iter(input.as_str()).map(|m| m.range()).collect()
    });
    replace_byte_ranges_with_asterisks(input, ranges);
}

/// Calls `f` with a reference to this thread's local clone of the telemetry
/// secrets regex, refreshing the clone if the global regex has been updated
/// since the last call.
///
/// Each thread's clone has its own `Cache` pool that never grows beyond a
/// single entry, avoiding the multi-GB pool accumulation that occurs when
/// many concurrent async tasks all share the same global `Regex`.
fn with_thread_local_regex<R>(f: impl FnOnce(&Regex) -> R) -> R {
    THREAD_LOCAL_REGEX.with(|cell| {
        let mut state = cell.borrow_mut();
        let current_version = REGEX_VERSION.load(Ordering::Acquire);
        if state.1.is_none() || state.0 != current_version {
            // Thread-local copy is absent or stale; clone the latest global regex.
            state.1 = Some(TELEMETRY_SECRETS_REGEX.read().clone());
            state.0 = current_version;
        }
        // Safety: we just ensured state.1 is Some above.
        f(state.1.as_ref().unwrap())
    })
}
/// Replaces each byte range in `input` with a run of asterisks of the same byte
/// length. Handles overlapping ranges by merging them first, and replaces from
/// the end of the string so earlier byte indices stay valid as we mutate.
fn replace_byte_ranges_with_asterisks(input: &mut String, mut ranges: Vec<Range<usize>>) {
    if ranges.is_empty() {
        return;
    }
    // Sort and merge overlapping ranges so we don't double-replace.
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    // Replace from the end of the string so earlier byte indices stay valid.
    for range in merged.into_iter().rev() {
        let len = range.end - range.start;
        input.replace_range(range, &REDACTION_REPLACEMENT_CHARACTER.repeat(len));
    }
}
/// Walks a [`Value`] and runs [`redact_secrets_in_string`] on every string within
/// it. Non-string scalars (numbers, booleans, nulls) are left untouched.
pub fn redact_secrets_in_value(value: &mut Value) {
    match value {
        Value::String(s) => redact_secrets_in_string(s),
        Value::Array(arr) => arr.iter_mut().for_each(redact_secrets_in_value),
        Value::Object(obj) => obj.values_mut().for_each(redact_secrets_in_value),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}
#[cfg(test)]
#[path = "secret_redaction_tests.rs"]
mod tests;
