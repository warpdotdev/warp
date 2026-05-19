//! Resolves the verb to display in the default "Warping..." spinner state.
//!
//! When [`FeatureFlag::CustomWarpingVerbs`] is enabled and the user has
//! populated [`AISettings::custom_warping_verbs`], this helper picks a verb
//! from that list instead of the built-in "Warping..." default. A
//! [`WarpingVerbSelector`] caches the current pick per-session so that the
//! shimmer animation does not reset every render frame.

use std::cell::RefCell;

use rand::seq::SliceRandom;
use warp_core::features::FeatureFlag;
use warpui::{AppContext, SingletonEntity};

use crate::settings::ai::AISettings;

/// Fallback display text shown when no custom verbs are configured.
pub const DEFAULT_WARPING_VERB: &str = "Warping...";

/// Maximum number of custom verbs allowed in the persisted list.
pub const MAX_CUSTOM_WARPING_VERBS: usize = 50;

/// Maximum display length (in chars, not bytes) of a single verb before the
/// trailing ellipsis is appended. Longer entries are truncated.
pub const MAX_WARPING_VERB_CHARS: usize = 40;

/// Trims, sentence-capitalizes, and validates a single verb. Returns `None` if
/// the trimmed value is empty. Over-long entries are truncated at
/// [`MAX_WARPING_VERB_CHARS`].
///
/// Trailing "." and "…" characters are stripped so that the render-time
/// formatter can append a single canonical "..." without double-dotting.
pub fn normalize_warping_verb(verb: &str) -> Option<String> {
    let trimmed = verb.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Strip trailing ellipses / periods; render-time formatting appends "...".
    let mut stripped: String = trimmed.to_owned();
    while let Some(last) = stripped.chars().last() {
        if last == '.' || last == '…' {
            stripped.pop();
        } else {
            break;
        }
    }
    let stripped = stripped.trim_end().to_owned();
    if stripped.is_empty() {
        return None;
    }

    let truncated = truncate_warping_verb(&stripped);
    let capitalized = sentence_capitalize_warping_verb(&truncated);
    let normalized = truncate_warping_verb(&capitalized);

    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn truncate_warping_verb(verb: &str) -> String {
    // Truncate to MAX_WARPING_VERB_CHARS (chars, not bytes, to avoid splitting
    // multi-byte codepoints).
    if verb.chars().count() > MAX_WARPING_VERB_CHARS {
        verb.chars()
            .take(MAX_WARPING_VERB_CHARS)
            .collect::<String>()
            .trim_end()
            .to_owned()
    } else {
        verb.to_owned()
    }
}

fn sentence_capitalize_warping_verb(verb: &str) -> String {
    let mut chars = verb.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut capitalized = first.to_uppercase().collect::<String>();
    capitalized.extend(chars);
    capitalized
}

/// Normalizes a list of verbs: trims, drops empties, truncates over-long
/// entries, and caps the list at [`MAX_CUSTOM_WARPING_VERBS`].
pub fn normalize_warping_verbs(verbs: Vec<String>) -> Vec<String> {
    verbs
        .into_iter()
        .filter_map(|v| normalize_warping_verb(&v))
        .take(MAX_CUSTOM_WARPING_VERBS)
        .collect()
}

/// Formats a verb for display in the spinner, appending "..." if the verb does
/// not already end with punctuation.
fn format_for_display(verb: &str) -> String {
    let trimmed = verb.trim_end();
    if trimmed.is_empty() {
        return DEFAULT_WARPING_VERB.to_owned();
    }
    let capitalized = sentence_capitalize_warping_verb(trimmed);
    match capitalized.chars().last() {
        Some('.') | Some('!') | Some('?') | Some('…') => capitalized,
        _ => format!("{capitalized}..."),
    }
}

/// Caches the currently-selected verb for one "warping session" to keep the
/// shimmer animation stable between renders.
///
/// A session is identified by an opaque `session_key` provided by the caller;
/// the preferred key is the current response stream id. When the key changes, a
/// fresh random verb is picked.
///
/// Uses interior mutability so it can be used from `&self` render paths.
#[derive(Debug, Default)]
pub struct WarpingVerbSelector {
    cached: RefCell<Option<CachedVerb>>,
}

#[derive(Debug, Clone)]
struct CachedVerb {
    session_key: String,
    /// Raw verb (pre-format) so the next session can avoid repeating it when
    /// alternatives are available.
    raw: String,
    /// Display form with trailing "..." applied.
    display: String,
}

impl WarpingVerbSelector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolves the verb to display for the default warping state. If the
    /// user's `custom_warping_verbs` list is empty or the feature flag is off,
    /// returns the built-in [`DEFAULT_WARPING_VERB`].
    ///
    /// `session_key` should identify the current warping session. A new verb is
    /// picked whenever the key changes.
    pub fn resolve(&self, session_key: &str, app: &AppContext) -> String {
        if !FeatureFlag::CustomWarpingVerbs.is_enabled() {
            self.cached.replace(None);
            return DEFAULT_WARPING_VERB.to_owned();
        }

        let verbs = AISettings::as_ref(app).custom_warping_verbs.clone();
        self.resolve_from_verbs(session_key, &verbs)
    }

    fn resolve_from_verbs(&self, session_key: &str, verbs: &[String]) -> String {
        // Cache hit: same session. Settings changes take effect on the next
        // session so one output keeps a single stable verb while it streams.
        if let Some(cached) = self.cached.borrow().as_ref() {
            if cached.session_key == session_key {
                return cached.display.clone();
            }
        }

        // Settings file edits and synced values can bypass
        // AISettings::set_custom_warping_verbs, so normalize again at the
        // renderer boundary before picking a display value.
        let verbs = normalize_warping_verbs(verbs.to_vec());
        if verbs.is_empty() {
            self.cached.replace(None);
            return DEFAULT_WARPING_VERB.to_owned();
        }
        let previous_raw = self.cached.borrow().as_ref().map(|c| c.raw.clone());
        let picked = pick_verb(&verbs, previous_raw.as_deref());
        let display = format_for_display(&picked);
        self.cached.replace(Some(CachedVerb {
            session_key: session_key.to_owned(),
            raw: picked,
            display: display.clone(),
        }));
        display
    }
}

/// Picks a verb from `verbs` that ideally differs from `previous`. Assumes
/// `verbs` is non-empty.
fn pick_verb(verbs: &[String], previous: Option<&str>) -> String {
    debug_assert!(!verbs.is_empty());
    let mut rng = rand::thread_rng();
    if verbs.len() == 1 {
        return verbs[0].clone();
    }
    if let Some(prev) = previous {
        let candidates: Vec<&String> = verbs.iter().filter(|v| v.as_str() != prev).collect();
        if !candidates.is_empty() {
            return candidates
                .choose(&mut rng)
                .map(|v| (*v).clone())
                .unwrap_or_else(|| verbs[0].clone());
        }
    }
    verbs
        .choose(&mut rng)
        .cloned()
        .unwrap_or_else(|| verbs[0].clone())
}

#[cfg(test)]
#[path = "warping_verb_tests.rs"]
mod tests;
