use std::path::{Path, PathBuf};

use futures::StreamExt as _;

use super::proto::{
    RipgrepSearchError, RipgrepSearchMatch, RipgrepSearchRequest, RipgrepSearchResponse,
    RipgrepSearchSubmatch, RipgrepSearchSuccess, ripgrep_search_response,
};

/// Server-side cap on the number of matched lines returned by `RipgrepSearch`.
const MAX_RIPGREP_SEARCH_MATCH_CAP: usize = 5_000;
/// Approximate payload budget for one remote search response.
///
/// Eight MB keeps transfer latency and memory well below the protocol's
/// 64 MB frame limit. Individual matches are never truncated because doing so
/// could remove a late submatch and corrupt its preview and click location.
const MAX_RIPGREP_SEARCH_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

pub(super) struct RipgrepSearchParams {
    pattern: String,
    roots: Vec<PathBuf>,
    ignore_case: bool,
    multiline: bool,
    match_cap: usize,
}

pub(super) fn validate_request(msg: RipgrepSearchRequest) -> Result<RipgrepSearchParams, String> {
    if msg.pattern.is_empty() || msg.roots.is_empty() {
        return Err("RipgrepSearch requires a pattern and at least one root".to_string());
    }
    if let Some(root) = msg.roots.iter().find(|root| !Path::new(root).is_absolute()) {
        return Err(format!("RipgrepSearch root must be absolute: {root}"));
    }

    let roots = msg.roots.iter().map(PathBuf::from).collect();
    let match_cap = match msg.max_matches as usize {
        0 => MAX_RIPGREP_SEARCH_MATCH_CAP,
        requested => requested.min(MAX_RIPGREP_SEARCH_MATCH_CAP),
    };

    Ok(RipgrepSearchParams {
        pattern: msg.pattern,
        roots,
        ignore_case: msg.ignore_case,
        multiline: msg.multiline,
        match_cap,
    })
}

pub(super) async fn run_search(
    params: RipgrepSearchParams,
) -> anyhow::Result<RipgrepSearchSuccess> {
    let stream = warp_ripgrep::search::search_streaming(
        std::slice::from_ref(&params.pattern),
        &params.roots,
        params.ignore_case,
        params.multiline,
    )?;
    futures::pin_mut!(stream);

    let mut matches = Vec::new();
    let mut response_bytes: usize = 0;
    let mut capped = false;
    while let Some(m) = stream.next().await {
        if matches.len() >= params.match_cap {
            capped = true;
            break;
        }
        let m = ripgrep_match_to_proto(m);
        let match_bytes = m
            .file_path
            .len()
            .saturating_add(m.line_text.len())
            .saturating_add(
                m.submatches
                    .len()
                    .saturating_mul(2 * std::mem::size_of::<u64>()),
            );
        if response_bytes.saturating_add(match_bytes) > MAX_RIPGREP_SEARCH_RESPONSE_BYTES {
            capped = true;
            break;
        }

        response_bytes += match_bytes;
        matches.push(m);
    }

    Ok(RipgrepSearchSuccess { matches, capped })
}

pub(super) fn error_response(message: String) -> RipgrepSearchResponse {
    RipgrepSearchResponse {
        result: Some(ripgrep_search_response::Result::Error(RipgrepSearchError {
            message,
        })),
    }
}

pub(super) fn search_result_to_response(
    result: anyhow::Result<RipgrepSearchSuccess>,
) -> RipgrepSearchResponse {
    match result {
        Ok(success) => RipgrepSearchResponse {
            result: Some(ripgrep_search_response::Result::Success(success)),
        },
        Err(err) => error_response(format!("{err:#}")),
    }
}

/// Converts a ripgrep match to its proto form without altering line text or
/// submatch offsets. Response-wide caps bound payload size without corrupting
/// individual matches.
fn ripgrep_match_to_proto(m: warp_ripgrep::search::Match) -> RipgrepSearchMatch {
    RipgrepSearchMatch {
        file_path: m.file_path.to_string_lossy().to_string(),
        line_number: m.line_number,
        line_text: m.line_text,
        submatches: m
            .submatches
            .into_iter()
            .map(|submatch| RipgrepSearchSubmatch {
                byte_start: submatch.byte_start.as_usize() as u64,
                byte_end: submatch.byte_end.as_usize() as u64,
            })
            .collect(),
    }
}

#[cfg(test)]
#[path = "ripgrep_search_tests.rs"]
mod tests;
