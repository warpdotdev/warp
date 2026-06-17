use std::io::{self, Read, Write};
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicI32, Ordering};

use anyhow::{Context, Result};
use crossterm::terminal;
use nix::unistd::{close, pipe, read, write as nix_write};

use crate::pty::resize_pty;
use crate::renderer::{self, AgentBlock, BlockKind};
use crate::shell_integration::{OscParser, ShellEvent};

// ── SIGWINCH self-pipe ───────────────────────────────────────────────

static SIGWINCH_WRITE_FD: AtomicI32 = AtomicI32::new(-1);

extern "C" fn sigwinch_handler(_sig: libc::c_int) {
    let fd = SIGWINCH_WRITE_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        unsafe {
            libc::write(fd, b"W".as_ptr() as *const libc::c_void, 1);
        }
    }
}

fn install_sigwinch_handler() -> Result<RawFd> {
    let (read_fd, write_fd) = pipe().context("pipe for SIGWINCH")?;
    for fd in [read_fd, write_fd] {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    }
    SIGWINCH_WRITE_FD.store(write_fd, Ordering::Relaxed);
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = sigwinch_handler as usize;
        sa.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);
        if libc::sigaction(libc::SIGWINCH, &sa, std::ptr::null_mut()) < 0 {
            return Err(anyhow::anyhow!(
                "sigaction SIGWINCH: {}",
                io::Error::last_os_error()
            ));
        }
    }
    Ok(read_fd)
}

// ── Mode state machine ──────────────────────────────────────────────

enum AgentPhase {
    WaitingForEcho,
    Capturing,
    WaitingForPrompt,
}

enum Mode {
    Shell,
    AgentInput { buffer: String, cursor: usize },
    AgentRunning {
        command: String,
        output: Vec<u8>,
        phase: AgentPhase,
    },
}

impl Mode {
    fn label(&self) -> &'static str {
        match self {
            Mode::Shell => "SHELL",
            Mode::AgentInput { .. } => "AGENT",
            Mode::AgentRunning { .. } => "RUNNING",
        }
    }

    fn hint(&self) -> &'static str {
        match self {
            Mode::Shell => "Ctrl-A: agent",
            Mode::AgentInput { .. } => "Enter: run | Esc: cancel",
            Mode::AgentRunning { .. } => "Ctrl-C: cancel",
        }
    }
}

// ── Interleaved OSC parser output ───────────────────────────────────

enum ParsedItem {
    Output(Vec<u8>),
    Event(ShellEvent),
}

fn feed_interleaved(parser: &mut OscParser, input: &[u8]) -> Vec<ParsedItem> {
    let mut items = Vec::new();
    for &byte in input {
        let (out, events) = parser.feed(&[byte]);
        if !out.is_empty() {
            if let Some(ParsedItem::Output(ref mut prev)) = items.last_mut() {
                prev.extend_from_slice(&out);
            } else {
                items.push(ParsedItem::Output(out));
            }
        }
        for event in events {
            items.push(ParsedItem::Event(event));
        }
    }
    items
}

// ── Helpers ─────────────────────────────────────────────────────────

fn write_stdout(bytes: &[u8]) {
    let _ = io::stdout().write_all(bytes);
    let _ = io::stdout().flush();
}

fn update_status_bar(mode: &Mode, cols: u16, rows: u16) {
    let mut out = Vec::new();
    out.extend_from_slice(&renderer::save_cursor());
    out.extend_from_slice(&renderer::move_to_status_bar_row(rows));
    out.extend_from_slice(&renderer::clear_line());
    out.extend_from_slice(&renderer::render_status_bar(
        mode.label(),
        "mock",
        mode.hint(),
        cols,
    ));
    out.extend_from_slice(&renderer::restore_cursor());
    write_stdout(&out);
}

// ── Public entry point ──────────────────────────────────────────────

pub fn run(master_fd: RawFd) -> Result<()> {
    let sigwinch_fd = install_sigwinch_handler()?;

    let (cols, rows) = terminal::size().context("terminal::size")?;
    // Child PTY gets one fewer row — the bottom row is the status bar.
    resize_pty(master_fd, cols, rows.saturating_sub(1))?;

    terminal::enable_raw_mode().context("enable_raw_mode")?;

    // Set scroll region and park cursor.
    write_stdout(&renderer::set_scroll_region(1, rows.saturating_sub(1)));
    write_stdout(b"\x1b[1;1H");

    let mut state = Wsh {
        mode: Mode::Shell,
        master_fd,
        osc_parser: OscParser::new(),
        cols,
        rows,
        should_exit: false,
    };
    update_status_bar(&state.mode, state.cols, state.rows);

    let result = state.run_loop(sigwinch_fd);

    // Restore terminal.
    let _ = terminal::disable_raw_mode();
    write_stdout(&renderer::reset_scroll_region());
    write_stdout(b"\r\n");

    let wfd = SIGWINCH_WRITE_FD.swap(-1, Ordering::Relaxed);
    if wfd >= 0 {
        let _ = close(wfd);
    }
    let _ = close(sigwinch_fd);

    result
}

// ── Core state ──────────────────────────────────────────────────────

struct Wsh {
    mode: Mode,
    master_fd: RawFd,
    osc_parser: OscParser,
    cols: u16,
    rows: u16,
    should_exit: bool,
}

impl Wsh {
    fn run_loop(&mut self, sigwinch_fd: RawFd) -> Result<()> {
        let stdin_fd: RawFd = libc::STDIN_FILENO;
        let mut buf = [0u8; 4096];

        loop {
            if self.should_exit {
                break;
            }

            let mut pollfds = [
                libc::pollfd { fd: stdin_fd, events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: self.master_fd, events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: sigwinch_fd, events: libc::POLLIN, revents: 0 },
            ];

            let ret = unsafe {
                libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, -1)
            };
            if ret < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err).context("poll");
            }

            if pollfds[2].revents & libc::POLLIN != 0 {
                self.handle_resize(sigwinch_fd)?;
            }

            if pollfds[1].revents & libc::POLLIN != 0 {
                match read(self.master_fd, &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => self.handle_pty_output(&buf[..n]),
                }
            }

            if pollfds[1].revents & libc::POLLHUP != 0 {
                self.drain_pty(&mut buf);
                break;
            }

            if pollfds[0].revents & libc::POLLIN != 0 {
                let n = io::stdin().read(&mut buf).context("read stdin")?;
                if n == 0 {
                    break;
                }
                self.handle_stdin(&buf[..n])?;
            }
        }

        Ok(())
    }

    // ── Resize ──────────────────────────────────────────────────────

    fn handle_resize(&mut self, sigwinch_fd: RawFd) -> Result<()> {
        let mut drain = [0u8; 64];
        while let Ok(n) = read(sigwinch_fd, &mut drain) {
            if n == 0 { break; }
        }
        let (cols, rows) = terminal::size().context("terminal::size on resize")?;
        self.cols = cols;
        self.rows = rows;
        resize_pty(self.master_fd, cols, rows.saturating_sub(1))?;
        write_stdout(&renderer::set_scroll_region(1, rows.saturating_sub(1)));
        update_status_bar(&self.mode, self.cols, self.rows);
        Ok(())
    }

    // ── PTY output handling ─────────────────────────────────────────

    fn handle_pty_output(&mut self, raw: &[u8]) {
        let items = feed_interleaved(&mut self.osc_parser, raw);
        for item in items {
            match item {
                ParsedItem::Output(bytes) => self.handle_filtered_output(&bytes),
                ParsedItem::Event(event) => self.process_event(event),
            }
        }
    }

    fn handle_filtered_output(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        match &mut self.mode {
            Mode::Shell | Mode::AgentInput { .. } => write_stdout(bytes),
            Mode::AgentRunning { phase, output, .. } => match phase {
                AgentPhase::WaitingForEcho => { /* discard shell echo */ }
                AgentPhase::Capturing => output.extend_from_slice(bytes),
                AgentPhase::WaitingForPrompt => write_stdout(bytes),
            },
        }
    }

    fn process_event(&mut self, event: ShellEvent) {
        match event {
            ShellEvent::CommandStart => {
                if let Mode::AgentRunning { phase, .. } = &mut self.mode {
                    if matches!(phase, AgentPhase::WaitingForEcho) {
                        *phase = AgentPhase::Capturing;
                    }
                }
            }
            ShellEvent::CommandFinished { exit_code } => {
                if matches!(
                    &self.mode,
                    Mode::AgentRunning { phase: AgentPhase::Capturing, .. }
                ) {
                    self.render_agent_result(exit_code);
                }
            }
            ShellEvent::PromptEnd => {
                if matches!(
                    &self.mode,
                    Mode::AgentRunning { phase: AgentPhase::WaitingForPrompt, .. }
                ) {
                    self.mode = Mode::Shell;
                    update_status_bar(&self.mode, self.cols, self.rows);
                }
            }
            _ => {}
        }
    }

    fn render_agent_result(&mut self, exit_code: Option<i32>) {
        let old = std::mem::replace(&mut self.mode, Mode::Shell);
        if let Mode::AgentRunning { output, command, .. } = old {
            let output_str = String::from_utf8_lossy(&output);
            // Strip \r from PTY output — the line discipline converts \n → \r\n,
            // and embedded \r would overwrite the left border when rendering.
            let body = output_str.replace('\r', "");
            let body = body.trim_end().to_string();
            let block = AgentBlock {
                kind: if exit_code == Some(0) || exit_code.is_none() {
                    BlockKind::CommandOutput
                } else {
                    BlockKind::Error
                },
                header: Some(format!(
                    "Output{}",
                    exit_code.map_or(String::new(), |c| format!(" (exit {c})"))
                )),
                body,
            };
            write_stdout(&renderer::render_block(&block, self.cols));
            self.mode = Mode::AgentRunning {
                command,
                output: Vec::new(),
                phase: AgentPhase::WaitingForPrompt,
            };
        }
    }

    fn drain_pty(&mut self, buf: &mut [u8]) {
        loop {
            match read(self.master_fd, buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let (filtered, _) = self.osc_parser.feed(&buf[..n]);
                    if !filtered.is_empty() {
                        write_stdout(&filtered);
                    }
                }
            }
        }
    }

    // ── Stdin handling ──────────────────────────────────────────────

    fn handle_stdin(&mut self, bytes: &[u8]) -> Result<()> {
        match &self.mode {
            Mode::Shell => {
                if let Some(pos) = bytes.iter().position(|&b| b == 0x01) {
                    if pos > 0 {
                        nix_write(self.master_fd, &bytes[..pos]).context("write to pty")?;
                    }
                    self.enter_agent_mode();
                    if pos + 1 < bytes.len() {
                        return self.handle_agent_input_bytes(&bytes[pos + 1..]);
                    }
                } else {
                    nix_write(self.master_fd, bytes).context("write to pty")?;
                }
            }
            Mode::AgentInput { .. } => {
                self.handle_agent_input_bytes(bytes)?;
            }
            Mode::AgentRunning { .. } => {
                // Ctrl-C cancels the running agent.
                if bytes.contains(&0x03) {
                    self.cancel_agent();
                }
            }
        }
        Ok(())
    }

    // ── Agent input mode ────────────────────────────────────────────

    fn enter_agent_mode(&mut self) {
        self.mode = Mode::AgentInput {
            buffer: String::new(),
            cursor: 0,
        };
        write_stdout(b"\r\n");
        self.render_input_prompt();
        update_status_bar(&self.mode, self.cols, self.rows);
    }

    fn exit_agent_mode(&mut self) {
        write_stdout(&renderer::clear_line());
        self.mode = Mode::Shell;
        // Send newline to child to re-display the prompt.
        let _ = nix_write(self.master_fd, b"\n");
        update_status_bar(&self.mode, self.cols, self.rows);
    }

    fn cancel_agent(&mut self) {
        self.mode = Mode::Shell;
        write_stdout(b"\r\n");
        // Forward Ctrl-C so the child shell can handle any in-flight command.
        let _ = nix_write(self.master_fd, b"\x03");
        update_status_bar(&self.mode, self.cols, self.rows);
    }

    fn render_input_prompt(&self) {
        if let Mode::AgentInput { buffer, cursor } = &self.mode {
            let mut out = renderer::clear_line();
            out.extend_from_slice(&renderer::render_agent_input_prompt(buffer, *cursor));
            write_stdout(&out);
        }
    }

    fn handle_agent_input_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        for &b in bytes {
            match b {
                0x0d | 0x0a => return self.submit_agent_query(),
                0x1b | 0x03 => {
                    self.exit_agent_mode();
                    return Ok(());
                }
                0x7f | 0x08 => {
                    if let Mode::AgentInput { buffer, cursor } = &mut self.mode {
                        if *cursor > 0 {
                            *cursor -= 1;
                            buffer.remove(*cursor);
                        }
                    }
                }
                b if (0x20..0x7f).contains(&b) => {
                    if let Mode::AgentInput { buffer, cursor } = &mut self.mode {
                        buffer.insert(*cursor, b as char);
                        *cursor += 1;
                    }
                }
                _ => {}
            }
        }
        self.render_input_prompt();
        Ok(())
    }

    // ── Mock agent ──────────────────────────────────────────────────

    fn submit_agent_query(&mut self) -> Result<()> {
        let command = match &self.mode {
            Mode::AgentInput { buffer, .. } => buffer.clone(),
            _ => return Ok(()),
        };

        if command.trim().is_empty() {
            self.exit_agent_mode();
            return Ok(());
        }

        write_stdout(&renderer::clear_line());

        // Built-in commands.
        match command.trim() {
            "help" => {
                let block = AgentBlock {
                    kind: BlockKind::AgentText,
                    header: Some("wsh mock agent".into()),
                    body: "Type any shell command and I'll execute it.\n\
                           Special commands:\n  \
                           help  — show this message\n  \
                           exit  — quit wsh"
                        .into(),
                };
                write_stdout(&renderer::render_block(&block, self.cols));
                self.mode = Mode::Shell;
                let _ = nix_write(self.master_fd, b"\n");
                update_status_bar(&self.mode, self.cols, self.rows);
                return Ok(());
            }
            "exit" => {
                self.should_exit = true;
                return Ok(());
            }
            _ => {}
        }

        // Show thinking indicator and "Running" block.
        write_stdout(&renderer::render_thinking_indicator());
        let run_block = AgentBlock {
            kind: BlockKind::CommandRun,
            header: Some(format!("Running: {command}")),
            body: String::new(),
        };
        write_stdout(&renderer::render_block(&run_block, self.cols));

        // Write command to child PTY.
        let cmd = format!("{command}\n");
        nix_write(self.master_fd, cmd.as_bytes()).context("write command to pty")?;

        self.mode = Mode::AgentRunning {
            command,
            output: Vec::new(),
            phase: AgentPhase::WaitingForEcho,
        };
        update_status_bar(&self.mode, self.cols, self.rows);
        Ok(())
    }
}
