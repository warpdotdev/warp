use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::anyhow;
use grep::printer::JSONBuilder;
use grep::regex::RegexMatcherBuilder;
use grep::searcher::{BinaryDetection, SearcherBuilder};
use ignore::{WalkBuilder, WalkState};
use string_offset::ByteOffset;

const SEARCHER_LINE_HEAP_LIMIT: usize = 64 * 1024;
const SEARCHER_MULTILINE_HEAP_LIMIT: usize = 8 * 1024 * 1024;
const JSON_RECORD_LIMIT_ERROR: &str = "ripgrep JSON record exceeded its limit";
/// JSON escaping expands one input byte to at most six bytes; the remaining
/// headroom covers record metadata and filesystem paths.
const MAX_JSON_RECORD_BYTES: usize = SEARCHER_MULTILINE_HEAP_LIMIT * 8;

/// A single submatch span within a matched line.
#[derive(Clone, Debug)]
pub struct Submatch {
    /// Byte offset into `line_text` where this submatch starts.
    pub byte_start: ByteOffset,
    /// Byte offset into `line_text` where this submatch ends (exclusive).
    pub byte_end: ByteOffset,
}

/// A single search match: one line in one file, with submatch highlights.
#[derive(Clone, Debug)]
pub struct Match {
    pub file_path: PathBuf,
    pub line_number: u32,
    pub line_text: String,
    pub submatches: Vec<Submatch>,
}

/// One item emitted by a streaming search.
#[derive(Clone, Debug)]
pub enum SearchEvent {
    /// A file-content match.
    Match(Match),
    /// At least one file was skipped after reaching the searcher heap limit.
    LimitReached,
}

struct JsonRecordWriter<'a, W> {
    output: &'a Mutex<W>,
    output_failed: &'a AtomicBool,
    record: Vec<u8>,
}

impl<'a, W: Write> JsonRecordWriter<'a, W> {
    fn new(output: &'a Mutex<W>, output_failed: &'a AtomicBool) -> Self {
        Self {
            output,
            output_failed,
            record: Vec::new(),
        }
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.record.len().saturating_add(bytes.len()) > MAX_JSON_RECORD_BYTES {
            return Err(io::Error::other(JSON_RECORD_LIMIT_ERROR));
        }
        self.record.extend_from_slice(bytes);
        Ok(())
    }

    fn emit_record(&mut self) -> io::Result<()> {
        let mut output = self.output.lock().map_err(|_| {
            self.output_failed.store(true, Ordering::Relaxed);
            io::Error::other("ripgrep output mutex was poisoned")
        })?;
        if let Err(err) = output.write_all(&self.record) {
            self.output_failed.store(true, Ordering::Relaxed);
            return Err(err);
        }
        self.record.clear();
        Ok(())
    }
}

impl<W: Write> Write for JsonRecordWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut record_start = 0;
        for (index, byte) in buf.iter().enumerate() {
            if *byte != b'\n' {
                continue;
            }
            self.append(&buf[record_start..=index])?;
            self.emit_record()?;
            record_start = index + 1;
        }
        self.append(&buf[record_start..])?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.record.is_empty() {
            return Err(io::Error::other(
                "ripgrep attempted to flush an incomplete JSON record",
            ));
        }
        let mut output = self.output.lock().map_err(|_| {
            self.output_failed.store(true, Ordering::Relaxed);
            io::Error::other("ripgrep output mutex was poisoned")
        })?;
        output.flush().inspect_err(|_| {
            self.output_failed.store(true, Ordering::Relaxed);
        })
    }
}

/// Entry point for the ripgrep subprocess.
///
/// Runs a ripgrep search in-process and writes JSON results to stdout.
/// The main Warp process spawns this via the `ripgrep-search` CLI
/// subcommand and reads the JSON output.
pub fn run_search_subprocess(
    patterns: &[String],
    paths: Vec<PathBuf>,
    ignore_case: bool,
    multiline: bool,
    #[cfg_attr(not(unix), allow(unused_variables))] parent_pid: Option<u32>,
) -> anyhow::Result<()> {
    #[cfg(unix)]
    crate::monitor_parent_and_exit_on_change(parent_pid);
    search_to_writer(patterns, paths, ignore_case, multiline, std::io::stdout())?;
    Ok(())
}

fn search_to_writer<W: Write + Send>(
    patterns: &[String],
    paths: Vec<PathBuf>,
    ignore_case: bool,
    multiline: bool,
    output: W,
) -> anyhow::Result<W> {
    search_to_writer_inner(patterns, paths, ignore_case, multiline, output, None, || {})
}

fn search_to_writer_inner<W, F>(
    patterns: &[String],
    paths: Vec<PathBuf>,
    ignore_case: bool,
    multiline: bool,
    output: W,
    worker_threads: Option<usize>,
    on_search_start: F,
) -> anyhow::Result<W>
where
    W: Write + Send,
    F: Fn() + Sync,
{
    if patterns.is_empty() {
        return Err(anyhow!("No patterns specified"));
    }
    if paths.is_empty() {
        return Err(anyhow!("No paths specified"));
    }

    let mut matcher_builder = RegexMatcherBuilder::new();
    matcher_builder.case_insensitive(ignore_case);
    if multiline {
        matcher_builder.line_terminator(None);
    }
    let matcher = matcher_builder.build_many(patterns)?;

    let output = Mutex::new(output);
    let output_failed = AtomicBool::new(false);

    let mut walker_builder = WalkBuilder::new(&paths[0]);
    for path in paths.iter().skip(1) {
        walker_builder.add(path);
    }
    if let Some(worker_threads) = worker_threads {
        walker_builder.threads(worker_threads);
    }
    let walker = walker_builder.build_parallel();

    walker.run(|| {
        let matcher = matcher.clone();
        let output = &output;
        let output_failed = &output_failed;
        let on_search_start = &on_search_start;

        let heap_limit = if multiline {
            SEARCHER_MULTILINE_HEAP_LIMIT
        } else {
            SEARCHER_LINE_HEAP_LIMIT
        };
        let mut searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(b'\x00'))
            .line_number(true)
            .multi_line(multiline)
            .heap_limit(Some(heap_limit))
            .build();

        Box::new(move |entry| {
            if output_failed.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    log::warn!("ripgrep walk error: {err}");
                    return WalkState::Continue;
                }
            };

            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                return WalkState::Continue;
            }

            on_search_start();
            let search_result = {
                let writer = JsonRecordWriter::new(output, output_failed);
                let mut printer = JSONBuilder::new().build(writer);
                searcher.search_path(
                    &matcher,
                    entry.path(),
                    printer.sink_with_path(&matcher, entry.path()),
                )
            };
            if output_failed.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            if let Err(err) = search_result {
                let error = err.to_string();
                if error.contains("configured allocation limit")
                    || error.contains(JSON_RECORD_LIMIT_ERROR)
                {
                    let mut writer = JsonRecordWriter::new(output, output_failed);
                    if writer.write_all(b"{\"type\":\"limit_reached\"}\n").is_err()
                        || writer.flush().is_err()
                    {
                        return WalkState::Quit;
                    }
                } else {
                    log::warn!(
                        "ripgrep search error for {}: {}",
                        entry.path().display(),
                        err
                    );
                }
            }

            WalkState::Continue
        })
    });

    output
        .into_inner()
        .map_err(|_| anyhow!("ripgrep output mutex was poisoned"))
}

#[cfg(not(target_family = "wasm"))]
mod process_impl {
    use std::path::PathBuf;
    use std::process::Stdio;

    use futures::StreamExt as _;
    use futures::io::{AsyncBufReadExt as _, BufReader};
    use futures::stream::Stream;

    use super::{Match, SearchEvent, Submatch};
    use crate::types::RipgrepMessage;

    /// Searches `paths` for lines matching `patterns` and returns all results.
    ///
    /// Spawns a child process that runs the ripgrep search, collects every
    /// match, and returns them once the search is complete.
    ///
    /// For incremental results, use [`search_streaming`] instead.
    pub async fn search(
        patterns: &[String],
        paths: &[PathBuf],
        ignore_case: bool,
        multiline: bool,
    ) -> anyhow::Result<Vec<Match>> {
        let stream = search_streaming(patterns, paths, ignore_case, multiline)?;
        Ok(stream
            .filter_map(|event| async move {
                match event {
                    SearchEvent::Match(m) => Some(m),
                    SearchEvent::LimitReached => None,
                }
            })
            .collect()
            .await)
    }

    /// Searches `paths` for lines matching `patterns`, returning matches and
    /// limit notifications as they are found.
    ///
    /// This is the preferred entry point when responsiveness matters (e.g.
    /// the global search UI). The caller controls batching and throttling.
    pub fn search_streaming(
        patterns: &[String],
        paths: &[PathBuf],
        ignore_case: bool,
        multiline: bool,
    ) -> anyhow::Result<impl Stream<Item = SearchEvent>> {
        let child = spawn_search_process(patterns, paths, ignore_case, multiline)?;
        Ok(match_stream_from_child(child))
    }

    /// Spawns the warp CLI with the `ripgrep-search` subcommand.
    fn spawn_search_process(
        patterns: &[String],
        paths: &[PathBuf],
        ignore_case: bool,
        multiline: bool,
    ) -> Result<async_process::Child, std::io::Error> {
        let current_exe = std::env::current_exe()?;
        let mut cmd = command::r#async::Command::new(current_exe);

        cmd.arg(warp_cli::ripgrep_search_subcommand())
            .arg(warp_cli::parent_flag());

        if ignore_case {
            cmd.arg("--ignore-case");
        }

        if multiline {
            cmd.arg("--multiline");
        }

        for pattern in patterns {
            cmd.arg(pattern);
        }

        for path in paths {
            cmd.arg(path);
        }

        cmd.kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped());

        cmd.spawn()
    }

    /// Turns a child process (with piped stdout) into a stream of parsed matches.
    fn match_stream_from_child(mut child: async_process::Child) -> impl Stream<Item = SearchEvent> {
        let stdout = child
            .stdout
            .take()
            .expect("spawn_search_process must pipe stdout");
        let reader = BufReader::new(stdout);

        futures::stream::unfold((reader, child), |(mut reader, child)| async move {
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => return None,
                    Ok(_) => {}
                }
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<RipgrepMessage>(&line) {
                    Ok(RipgrepMessage::Match { data }) => {
                        let submatches = data
                            .submatches
                            .into_iter()
                            .map(|s| Submatch {
                                byte_start: s.start,
                                byte_end: s.end,
                            })
                            .collect();
                        let m = Match {
                            file_path: PathBuf::from(data.path.text),
                            line_number: data.line_number,
                            line_text: data.lines.text,
                            submatches,
                        };
                        return Some((SearchEvent::Match(m), (reader, child)));
                    }
                    Ok(RipgrepMessage::Begin | RipgrepMessage::End) => continue,
                    Ok(RipgrepMessage::LimitReached) => {
                        return Some((SearchEvent::LimitReached, (reader, child)));
                    }
                    Err(err) => {
                        log::warn!("ripgrep: failed to parse JSON line: {err}");
                        continue;
                    }
                }
            }
        })
    }
}

#[cfg(not(target_family = "wasm"))]
pub use process_impl::{search, search_streaming};

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
