use std::io::Write as _;

use anyhow::Context;
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, ContentArrangement, Table};
use jaq_all::data::Runner;
use jaq_all::fmts::Format;
use jaq_all::fmts::write::Writer;
// Use jaq_json directly to ensure serde support is included.
use jaq_json::{Val, write as jaq_write};
use serde::Serialize;
use tabwriter::TabWriter;
use warp_cli::agent::OutputFormat;
use warp_cli::json_filter::{JqFilter, JsonOutput};

pub fn standard_table() -> Table {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic);
    table
}

/// Trait for types that can be printed as a table.
pub trait TableFormat {
    fn header() -> Vec<Cell>;

    fn row(&self) -> Vec<Cell>;
}

/// Print a list of items to stdout, respecting the `output_format`.
pub fn print_list<I, T>(items: I, output_format: OutputFormat)
where
    I: IntoIterator<Item = T>,
    T: TableFormat + Serialize,
{
    if let Err(err) = write_list(items, output_format, &mut std::io::stdout()) {
        // If we can't write to stdout, try reporting to the log file.
        log::warn!("Unable to write to stdout: {err}");
    }
}

/// Write a serializable value to `output` as pretty JSON.
pub fn write_json<T, W>(value: &T, mut output: W) -> anyhow::Result<()>
where
    T: Serialize,
    W: std::io::Write,
{
    serde_json::to_writer_pretty(&mut output, value).context("unable to write JSON output")?;
    writeln!(&mut output)?;
    Ok(())
}

/// Write a serializable value to `output` as a single-line JSON record.
pub fn write_json_line<T, W>(value: &T, mut output: W) -> anyhow::Result<()>
where
    T: Serialize,
    W: std::io::Write,
{
    serde_json::to_writer(&mut output, value).context("unable to write JSON output")?;
    writeln!(&mut output)?;
    Ok(())
}
/// RAII guard that locks stdout and, on Unix, temporarily clears `O_NONBLOCK`
/// so writes block instead of returning `EAGAIN`. Restores the original flags
/// and releases the stdout lock on drop.
///
/// It's possible that stdout has `O_NONBLOCK` set, especially if it's a PTY.
/// If we're writing a large payload faster than it can be read, we can fill
/// up the PTY buffer, and get an `EAGAIN` error instead of blocking until the
/// reader consumes the data.
///
/// Rather than reimplement backoff based on `EAGAIN` ourselves, we can ensure
/// that writes will block for the lifetime of the guard.
struct StdoutBlockingGuard {
    /// Locked stdout handle. Declared first so that on drop, the `O_NONBLOCK`
    /// flag is restored (via `Drop for StdoutBlockingGuard`) before the lock
    /// is released.
    lock: std::io::StdoutLock<'static>,
    /// The original stdout flags to restore on drop, or `None` if stdout was
    /// already blocking (or we could not read/modify the flags).
    #[cfg(unix)]
    original_flags: Option<libc::c_int>,
}

impl StdoutBlockingGuard {
    fn new() -> Self {
        let lock = std::io::stdout().lock();

        #[cfg(unix)]
        {
            use libc::{F_GETFL, F_SETFL, O_NONBLOCK, STDOUT_FILENO, fcntl};
            // SAFETY: `fcntl` with `F_GETFL` / `F_SETFL` only reads and writes
            // the file status flags on stdout; it does not mutate Rust memory.
            let original_flags = unsafe {
                let flags = fcntl(STDOUT_FILENO, F_GETFL, 0);
                if flags < 0 {
                    let err = std::io::Error::last_os_error();
                    log::warn!("Unable to read stdout flags: {err}");
                    None
                } else if flags & O_NONBLOCK == 0 {
                    // Already blocking; nothing to change and nothing to restore.
                    None
                } else if fcntl(STDOUT_FILENO, F_SETFL, flags & !O_NONBLOCK) < 0 {
                    let err = std::io::Error::last_os_error();
                    log::warn!("Unable to clear O_NONBLOCK on stdout: {err}");
                    None
                } else {
                    Some(flags)
                }
            };
            Self {
                lock,
                original_flags,
            }
        }
        #[cfg(not(unix))]
        {
            Self { lock }
        }
    }
}

impl std::io::Write for StdoutBlockingGuard {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.lock.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.lock.flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.lock.write_all(buf)
    }
}

impl Drop for StdoutBlockingGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(flags) = self.original_flags {
            use libc::{F_SETFL, STDOUT_FILENO, fcntl};
            // SAFETY: `fcntl` with `F_SETFL` only updates the file status
            // flags on stdout; it does not mutate Rust memory.
            unsafe {
                if fcntl(STDOUT_FILENO, F_SETFL, flags) < 0 {
                    let err = std::io::Error::last_os_error();
                    log::warn!("Unable to restore stdout flags: {err}");
                }
            }
        }
    }
}

/// Write a raw `serde_json::Value` to stdout as pretty-printed JSON.
///
/// Useful for CLI commands that want to pass through server responses verbatim
/// when `--output-format json` is set, rather than re-serializing through a
/// typed model.
///
/// When `json_output.filter` is `Some`, the JSON value is instead passed
/// through the given filter, each output of which is printed to stdout.
pub fn print_raw_json(value: serde_json::Value, json_output: &JsonOutput) -> anyhow::Result<()> {
    // Ensure that writes to stdout are blocking, to enforce backpressure as we
    // potentially write a large amount of data. Otherwise, writes may fail with
    // `EAGAIN`, which we can't easily handle while serializing JSON. The guard
    // also locks stdout for the duration of the write, and restores the prior
    // blocking mode when it goes out of scope.
    //
    // Stream directly to stdout so we don't buffer a large `String`
    // unnecessarily, and so we can return I/O errors rather than potentially
    // panicking (as `println!` would).
    let mut out = StdoutBlockingGuard::new();
    match json_output.filter.as_ref() {
        None => {
            serde_json::to_writer_pretty(&mut out, &value)
                .context("unable to write JSON output")?;
            writeln!(&mut out)?;
        }
        Some(filter) => run_jq_filter(value, filter, &mut out)?,
    }
    out.flush()?;
    Ok(())
}

/// Run `jq_filter` on `value` and write each output to `out` on its own line.
///
/// Top-level scalar outputs are written as raw text (see [`write_filter_output`]).
/// Runtime errors from the filter are returned as `anyhow::Error`; any outputs
/// produced before the error are still written to `out`, matching jq's behavior.
fn run_jq_filter<W: std::io::Write>(
    value: serde_json::Value,
    jq_filter: &JqFilter,
    out: &mut W,
) -> anyhow::Result<()> {
    let input_result = serde_json::from_value::<Val>(value);

    let runner = Runner {
        null_input: false,
        color_err: false,
        writer: Writer {
            format: Format::Json,
            pp: pretty_pp(),
            join: false,
        },
    };

    jaq_all::data::run(
        &runner,
        jq_filter,
        Default::default(),
        [input_result].into_iter(),
        // Callback to format invalid input errors.
        |err| anyhow::anyhow!("Invalid data: {err}"),
        // Callback to handle filter outputs.
        |result| match result {
            Ok(val) => write_filter_output(&val, out),
            Err(err) => anyhow::bail!("jq filter error: {err}"),
        },
    )?;

    Ok(())
}

/// Pretty-printer configuration used for non-scalar filter output. Matches
/// `serde_json`'s pretty printer: two-space indent, space after `:`, no
/// trailing space after `,` (since commas sit at end-of-line).
fn pretty_pp() -> jaq_write::Pp {
    jaq_write::Pp {
        indent: Some("  ".to_string()),
        sep_space: true,
        ..jaq_write::Pp::default()
    }
}

/// Write a single filter output, unwrapping top-level scalars to raw text.
///
/// - `Null` -> `null`
/// - `Bool` -> `true` / `false`
/// - `Num` -> its decimal form
/// - `TStr` / `BStr` -> the unescaped string content (no surrounding quotes)
/// - `Arr` / `Obj` -> pretty-printed JSON via `jaq_json::write`, with the same
///   formatting conventions as the non-filtered `--output-format json` path.
///
/// Every output is followed by a newline.
fn write_filter_output<W: std::io::Write>(val: &Val, out: &mut W) -> anyhow::Result<()> {
    match val {
        Val::Null => writeln!(out, "null")?,
        Val::Bool(b) => writeln!(out, "{b}")?,
        Val::Num(n) => writeln!(out, "{n}")?,
        Val::TStr(bytes) | Val::BStr(bytes) => {
            out.write_all(bytes)?;
            writeln!(out)?;
        }
        Val::Arr(_) | Val::Obj(_) => {
            jaq_write::write(&mut *out, &pretty_pp(), 0, val)
                .context("unable to write jq output as JSON")?;
            writeln!(out)?;
        }
    }
    Ok(())
}

/// Write a list of items to `output`, respecting the `output_format`.
pub fn write_list<I, T, W>(
    items: I,
    output_format: OutputFormat,
    mut output: W,
) -> anyhow::Result<()>
where
    I: IntoIterator<Item = T>,
    T: TableFormat + Serialize,
    W: std::io::Write,
{
    match output_format {
        OutputFormat::Json => {
            let items = items.into_iter().collect::<Vec<_>>();
            serde_json::to_writer(&mut output, &items).context("unable to write JSON output")
        }
        OutputFormat::Ndjson => {
            for item in items {
                write_json_line(&item, &mut output)?;
            }
            Ok(())
        }
        OutputFormat::Pretty => {
            // Use comfy-table to print a table with terminal formatting.
            let mut table = standard_table();
            table.set_header(T::header());
            for item in items {
                table.add_row(T::row(&item));
            }
            writeln!(&mut output, "{table}")?;
            Ok(())
        }
        OutputFormat::Text => {
            // Print a plain-text table.
            let mut tw = TabWriter::new(output);

            for (idx, column) in T::header().iter().enumerate() {
                if idx > 0 {
                    write!(&mut tw, "\t")?;
                }
                write!(&mut tw, "{}", column.content())?;
            }
            writeln!(&mut tw)?;

            for item in items {
                for (idx, column) in T::row(&item).iter().enumerate() {
                    if idx > 0 {
                        write!(&mut tw, "\t")?;
                    }
                    write!(&mut tw, "{}", column.content())?;
                }
                writeln!(&mut tw)?;
            }
            tw.flush()?;
            Ok(())
        }
    }
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
