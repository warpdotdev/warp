use std::path::PathBuf;

use anyhow::Result;
use futures::StreamExt as _;
use instant::Instant;
use num_traits::SaturatingSub;
use regex::escape;
use string_offset::ByteOffset;
use warp_ripgrep::search::{Match as RipgrepMatch, Submatch};
use warpui::r#async::SpawnedFutureHandle;
use warpui::{Entity, ModelContext, ModelSpawner};

use crate::workspace::view::global_search::view::GlobalSearchEvent;
use crate::workspace::view::global_search::SearchConfig;

const START_BATCH_AFTER_COUNT: usize = 50;
const MAX_BATCH_SIZE: usize = 512;
const MAX_BATCH_AGE_MS: u64 = 4000;

pub struct GlobalSearch {
    search_handle: Option<SpawnedFutureHandle>,
    // track the search ID so that we only show results for the current search
    next_search_id: u32,
}

impl Entity for GlobalSearch {
    type Event = GlobalSearchEvent;
}

async fn flush_batch(
    spawner: &ModelSpawner<GlobalSearch>,
    search_id: u32,
    batch: &mut Vec<RipgrepMatch>,
) {
    if batch.is_empty() {
        return;
    }

    let items = std::mem::take(batch);

    let _ = spawner
        .spawn(move |_me, ctx| {
            ctx.emit(GlobalSearchEvent::ProgressBatch { search_id, items });
        })
        .await;
}

impl GlobalSearch {
    pub fn new() -> Self {
        GlobalSearch {
            search_handle: None,
            next_search_id: 1,
        }
    }

    pub fn abort_search(&mut self) {
        if let Some(handle) = self.search_handle.take() {
            handle.abort();
        }
    }

    pub fn run_search(
        &mut self,
        pattern: String,
        roots: Vec<PathBuf>,
        search_config: SearchConfig,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(handle) = self.search_handle.take() {
            log::info!("GlobalSearch: aborting previous search");
            handle.abort();
        }

        let search_id = self.next_search_id;
        self.next_search_id += 1;

        ctx.emit(GlobalSearchEvent::Started { search_id });

        let spawner = ctx.spawner();
        let effective_pattern = if search_config.use_regex {
            pattern
        } else {
            escape(&pattern)
        };
        let ignore_case = !search_config.use_case_sensitivity;
        let multiline = effective_pattern.contains('\n');

        let handle = ctx.spawn(
            async move {
                Self::run_warp_ripgrep_cli(
                    search_id,
                    effective_pattern,
                    roots,
                    ignore_case,
                    multiline,
                    spawner,
                )
                .await
            },
            move |_, result, ctx| match result {
                Ok(total_match_count) => {
                    ctx.emit(GlobalSearchEvent::Completed {
                        search_id,
                        total_match_count,
                    });
                }
                Err(err) => {
                    log::error!("GlobalSearch: warp_ripgrep CLI search failed or aborted: {err}");
                    ctx.emit(GlobalSearchEvent::Failed {
                        search_id,
                        error: "Global search failed.".to_string(),
                    });
                }
            },
        );

        self.search_handle = Some(handle);
    }

    async fn run_warp_ripgrep_cli(
        search_id: u32,
        pattern: String,
        roots: Vec<PathBuf>,
        ignore_case: bool,
        multiline: bool,
        spawner: ModelSpawner<GlobalSearch>,
    ) -> Result<usize> {
        let roots_display: Vec<_> = roots.iter().map(|r| r.display().to_string()).collect();
        log::info!(
            "GlobalSearch: starting warp_ripgrep CLI search with pattern={pattern}, roots={:?}",
            roots_display
        );

        let stream =
            warp_ripgrep::search::search_streaming(&[pattern], &roots, ignore_case, multiline)?;
        futures::pin_mut!(stream);

        let mut total_match_count: usize = 0;
        let mut num_unbatched_emitted: usize = 0;
        let mut batch: Vec<RipgrepMatch> = Vec::new();
        let mut last_batch_flush_at = Instant::now();

        while let Some(raw_match) = stream.next().await {
            // Expand each submatch into its own result row (matching
            // the old per-submatch behavior). Each row gets the line
            // text trimmed up to that particular submatch.
            for per_submatch in Self::expand_submatches(raw_match) {
                total_match_count += 1;

                if num_unbatched_emitted < START_BATCH_AFTER_COUNT {
                    num_unbatched_emitted += 1;

                    let _ = spawner
                        .spawn(move |_me, ctx| {
                            ctx.emit(GlobalSearchEvent::Progress {
                                search_id,
                                result: per_submatch,
                            });
                        })
                        .await;
                } else {
                    batch.push(per_submatch);

                    let too_big = batch.len() >= MAX_BATCH_SIZE;
                    let too_old =
                        last_batch_flush_at.elapsed().as_millis() >= MAX_BATCH_AGE_MS as u128;

                    if too_big || too_old {
                        flush_batch(&spawner, search_id, &mut batch).await;
                        last_batch_flush_at = Instant::now();
                    }
                }
            }
        }

        if !batch.is_empty() {
            flush_batch(&spawner, search_id, &mut batch).await;
        }

        Ok(total_match_count)
    }

    /// Expand a single ripgrep match (which may contain multiple submatches
    /// on the same line) into one result per submatch. Each result gets the
    /// line text trimmed of leading whitespace up to that submatch.
    fn expand_submatches(m: RipgrepMatch) -> Vec<RipgrepMatch> {
        if m.submatches.len() <= 1 {
            return vec![Self::trim_leading_whitespace_for_submatch(
                &m.line_text,
                m.file_path,
                m.line_number,
                m.submatches.into_iter().next(),
            )];
        }

        m.submatches
            .into_iter()
            .map(|sub| {
                Self::trim_leading_whitespace_for_submatch(
                    &m.line_text,
                    m.file_path.clone(),
                    m.line_number,
                    Some(sub),
                )
            })
            .collect()
    }

    /// Trim leading whitespace from a line up to the given submatch,
    /// adjusting the submatch offset accordingly.
    fn trim_leading_whitespace_for_submatch(
        original_line: &str,
        file_path: PathBuf,
        line_number: u32,
        submatch: Option<Submatch>,
    ) -> RipgrepMatch {
        let submatch_start = submatch
            .as_ref()
            .map(|s| s.byte_start)
            .unwrap_or(ByteOffset::zero());

        let mut leading_trimmed_bytes = ByteOffset::zero();
        for (byte_index, ch) in original_line.char_indices() {
            if byte_index >= submatch_start.as_usize() {
                break;
            }
            if !ch.is_ascii_whitespace() {
                break;
            }
            leading_trimmed_bytes += ch.len_utf8();
        }

        let trimmed_line = original_line[leading_trimmed_bytes.as_usize()..].to_string();

        let submatches = if let Some(sub) = submatch {
            vec![Submatch {
                byte_start: sub.byte_start.saturating_sub(&leading_trimmed_bytes),
                byte_end: sub.byte_end.saturating_sub(&leading_trimmed_bytes),
            }]
        } else {
            Vec::new()
        };

        RipgrepMatch {
            file_path,
            line_number,
            line_text: trimmed_line,
            submatches,
        }
    }
}

impl Default for GlobalSearch {
    fn default() -> Self {
        Self::new()
    }
}
