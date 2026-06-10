use std::env;
use std::io::{self, Read};
use std::os::unix::io::{AsRawFd, RawFd};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};

use anyhow::{Context, Result};
use crossterm::terminal;
use nix::unistd::{close, pipe, read, write as nix_write};

use crate::block::BlockManager;
use crate::cell::{Cell, CellAttr, CellFlags, Color};
use crate::miniterm::MiniTerm;
use crate::pty::resize_pty;
use crate::screen::{self, Frame, StatusBar};
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

enum Mode {
    Shell,
    AgentInput { buffer: String, cursor: usize },
    AgentRunning,
}

impl Mode {
    fn label(&self) -> &'static str {
        match self {
            Mode::Shell => "SHELL",
            Mode::AgentInput { .. } => "AGENT",
            Mode::AgentRunning => "RUNNING",
        }
    }

    fn hint(&self) -> &'static str {
        match self {
            Mode::Shell => "Ctrl-A: agent",
            Mode::AgentInput { .. } => "Enter: run | Esc: cancel",
            Mode::AgentRunning => "Ctrl-C: cancel",
        }
    }
}

// ── Agent subprocess ─────────────────────────────────────────────────

#[allow(clippy::disallowed_types)]
fn resolve_agent_binary() -> String {
    if let Ok(bin) = env::var("WSH_AGENT_BINARY") {
        return bin;
    }

    // Try well-known CLI names on PATH.
    for name in ["warp", "oz"] {
        if std::process::Command::new(name)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
        {
            return name.to_string();
        }
    }

    // Fall back to macOS app bundle binaries.
    for path in [
        "/Applications/WarpDev.app/Contents/MacOS/dev",
        "/Applications/Warp.app/Contents/MacOS/stable",
    ] {
        if std::path::Path::new(path).exists() {
            return path.to_string();
        }
    }

    "warp".to_string()
}

struct AgentProcess {
    child: Child,
    stdout_fd: RawFd,
    line_buffer: Vec<u8>,
}

impl AgentProcess {
    #[allow(clippy::disallowed_types)]
    fn spawn(prompt: &str, conversation_id: Option<&str>) -> Result<Self> {
        let binary = resolve_agent_binary();
        let mut args = vec!["agent", "run", "--prompt", prompt, "--output-format", "ndjson", "--model", "kimi-k26-fireworks"];
        let conv_flag;
        if let Some(id) = conversation_id {
            conv_flag = id.to_string();
            args.push("--conversation");
            args.push(&conv_flag);
        }
        let child = std::process::Command::new(&binary)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to spawn `{binary} agent run`"))?;

        let stdout = child.stdout.as_ref().expect("piped stdout");
        let stdout_fd = stdout.as_raw_fd();

        // Make stdout non-blocking for polling.
        let flags = unsafe { libc::fcntl(stdout_fd, libc::F_GETFL) };
        unsafe { libc::fcntl(stdout_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };

        Ok(Self {
            child,
            stdout_fd,
            line_buffer: Vec::new(),
        })
    }

    fn read_lines(&mut self) -> Vec<String> {
        let mut buf = [0u8; 4096];
        let mut lines = Vec::new();

        loop {
            let n = match read(self.stdout_fd, &mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            self.line_buffer.extend_from_slice(&buf[..n]);
        }

        // Extract complete lines from the buffer.
        while let Some(pos) = self.line_buffer.iter().position(|&b| b == b'\n') {
            let line = self.line_buffer.drain(..=pos).collect::<Vec<_>>();
            if let Ok(s) = String::from_utf8(line) {
                let trimmed = s.trim().to_string();
                if !trimmed.is_empty() {
                    lines.push(trimmed);
                }
            }
        }

        lines
    }

    fn is_finished(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── NDJSON event parsing ─────────────────────────────────────────────

fn parse_agent_event(line: &str, cols: usize) -> Option<(String, Color, CellFlags)> {
    let json: serde_json::Value = serde_json::from_str(line).ok()?;
    let event_type = json.get("type")?.as_str()?;

    match event_type {
        "system" => {
            // Return conversation_id extraction as a special marker.
            // The caller will handle it separately.
            None
        }
        "agent" | "agent_reasoning" => {
            let text = json.get("text")?.as_str()?;
            if text.trim().is_empty() {
                return None;
            }
            let color = if event_type == "agent_reasoning" {
                Color::Indexed(8) // dim gray for reasoning
            } else {
                Color::Default
            };
            Some((text.to_string(), color, CellFlags::empty()))
        }
        "tool_call" => {
            let tool = json.get("tool").and_then(|t| t.as_str()).unwrap_or("tool");
            let summary = json
                .get("summary")
                .and_then(|s| s.as_str())
                .or_else(|| json.get("command").and_then(|c| c.as_str()))
                .unwrap_or("");
            let display = if summary.is_empty() {
                format!("⚡ {tool}")
            } else {
                let max = cols.saturating_sub(4);
                let s = if summary.len() > max {
                    format!("{}…", &summary[..max.saturating_sub(1)])
                } else {
                    summary.to_string()
                };
                format!("⚡ {tool}: {s}")
            };
            Some((display, Color::Indexed(3), CellFlags::DIM)) // yellow
        }
        "tool_result" => {
            // Show a condensed result — just the first meaningful line.
            let output = json
                .get("output")
                .and_then(|o| o.as_str())
                .unwrap_or("");
            let first_line = output.lines().next().unwrap_or("(done)");
            let max = cols.saturating_sub(4);
            let display = if first_line.len() > max {
                format!("  ↪ {}…", &first_line[..max.saturating_sub(3)])
            } else {
                format!("  ↪ {first_line}")
            };
            Some((display, Color::Indexed(8), CellFlags::DIM)) // dim gray
        }
        "tool_error" => {
            let error = json
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown error");
            Some((format!("✗ {error}"), Color::Indexed(1), CellFlags::empty())) // red
        }
        "tool_canceled" => {
            Some(("✗ canceled".to_string(), Color::Indexed(3), CellFlags::DIM))
        }
        _ => {
            // Unknown event type — show raw for debugging.
            let preview = if line.len() > cols {
                format!("{}…", &line[..cols.saturating_sub(1)])
            } else {
                line.to_string()
            };
            Some((preview, Color::Indexed(8), CellFlags::DIM))
        }
    }
}

// ── Interleaved OSC parser ──────────────────────────────────────────

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

// ── Public entry point ──────────────────────────────────────────────

pub fn run(master_fd: RawFd) -> Result<()> {
    let sigwinch_fd = install_sigwinch_handler()?;

    let (cols, rows) = terminal::size().context("terminal::size")?;
    let usable = rows.saturating_sub(1);
    resize_pty(master_fd, cols, usable)?;

    screen::enter_alt_screen().context("enter_alt_screen")?;

    let mut state = Wsh {
        mode: Mode::Shell,
        master_fd,
        osc_parser: OscParser::new(),
        miniterm: MiniTerm::new(cols, usable),
        blocks: BlockManager::new(),
        agent: None,
        conversation_id: None,
        cols,
        rows,
        should_exit: false,
        spinner_tick: 0,
        streaming_lines: Vec::new(),
        streaming_col: 0,
    };
    state.render_frame();

    let result = state.run_loop(sigwinch_fd);

    // Clean up agent subprocess if still running.
    if let Some(mut agent) = state.agent.take() {
        agent.kill();
    }

    let _ = screen::leave_alt_screen();

    let wfd = SIGWINCH_WRITE_FD.swap(-1, Ordering::Relaxed);
    if wfd >= 0 {
        let _ = close(wfd);
    }
    let _ = close(sigwinch_fd);

    result
}

// ── Core state ──────────────────────────────────────────────────────

const SPINNER: &[&str] = &[
    "⠋ agent working...",
    "⠙ agent working...",
    "⠹ agent working...",
    "⠸ agent working...",
    "⠼ agent working...",
    "⠴ agent working...",
    "⠦ agent working...",
    "⠧ agent working...",
    "⠇ agent working...",
    "⠏ agent working...",
];

struct Wsh {
    mode: Mode,
    master_fd: RawFd,
    osc_parser: OscParser,
    miniterm: MiniTerm,
    blocks: BlockManager,
    agent: Option<AgentProcess>,
    conversation_id: Option<String>,
    cols: u16,
    rows: u16,
    should_exit: bool,
    spinner_tick: usize,
    streaming_lines: Vec<Vec<Cell>>,
    streaming_col: usize,
}

impl Wsh {
    fn run_loop(&mut self, sigwinch_fd: RawFd) -> Result<()> {
        let stdin_fd: RawFd = libc::STDIN_FILENO;
        let mut buf = [0u8; 4096];

        loop {
            if self.should_exit {
                break;
            }

            // Build poll fds: stdin, master, sigwinch, and optionally agent stdout.
            let agent_fd = self.agent.as_ref().map(|a| a.stdout_fd).unwrap_or(-1);
            let mut pollfds = [
                libc::pollfd { fd: stdin_fd, events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: self.master_fd, events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: sigwinch_fd, events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: agent_fd, events: libc::POLLIN, revents: 0 },
            ];
            let nfds = if agent_fd >= 0 { 4 } else { 3 };

            // Use a short timeout when the agent is running so the spinner animates.
            let timeout_ms = if matches!(self.mode, Mode::AgentRunning) { 120 } else { -1 };
            let ret = unsafe {
                libc::poll(pollfds.as_mut_ptr(), nfds as libc::nfds_t, timeout_ms)
            };
            if ret < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err).context("poll");
            }

            // Timeout with no events — still need to re-render for spinner.
            let mut needs_render = ret == 0 && matches!(self.mode, Mode::AgentRunning);

            // SIGWINCH
            if pollfds[2].revents & libc::POLLIN != 0 {
                self.handle_resize(sigwinch_fd)?;
                needs_render = true;
            }

            // PTY output
            if pollfds[1].revents & libc::POLLIN != 0 {
                match read(self.master_fd, &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        self.handle_pty_output(&buf[..n]);
                        needs_render = true;
                    }
                }
            }

            // Child shell exited
            if pollfds[1].revents & libc::POLLHUP != 0 {
                self.drain_pty(&mut buf);
                self.render_frame();
                break;
            }

            // Agent subprocess output
            if nfds == 4 && pollfds[3].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
                self.handle_agent_output();
                needs_render = true;
            }

            // stdin
            if pollfds[0].revents & libc::POLLIN != 0 {
                let n = io::stdin().read(&mut buf).context("read stdin")?;
                if n == 0 {
                    break;
                }
                self.handle_stdin(&buf[..n])?;
                needs_render = true;
            }

            if needs_render {
                if matches!(self.mode, Mode::AgentRunning) {
                    self.spinner_tick = self.spinner_tick.wrapping_add(1);
                }
                self.render_frame();
            }
        }

        Ok(())
    }

    // ── Render ────────────────────────────────────────────────────────

    fn render_frame(&self) {
        let completed = self.blocks.collected_rows();
        let frame = Frame {
            completed_rows: &completed,
            streaming_rows: &self.streaming_lines,
            active_grid: self.miniterm.grid(),
            active_cursor: self.miniterm.cursor_pos(),
            status_bar: StatusBar {
                mode: self.mode.label(),
                model: "warp",
                hint: self.mode.hint(),
            },
            scroll_offset: self.blocks.scroll_offset(),
            agent_input: match &self.mode {
                Mode::AgentInput { buffer, cursor } => Some((buffer.as_str(), *cursor)),
                _ => None,
            },
            agent_status: match &self.mode {
                Mode::AgentRunning => Some(SPINNER[self.spinner_tick % SPINNER.len()]),
                _ => None,
            },
            total_rows: self.rows,
            total_cols: self.cols,
            show_cursor: matches!(self.mode, Mode::Shell),
        };
        let _ = screen::render(&frame);
    }

    // ── Resize ────────────────────────────────────────────────────────

    fn handle_resize(&mut self, sigwinch_fd: RawFd) -> Result<()> {
        let mut drain = [0u8; 64];
        while let Ok(n) = read(sigwinch_fd, &mut drain) {
            if n == 0 { break; }
        }
        let (cols, rows) = terminal::size().context("terminal::size on resize")?;
        self.cols = cols;
        self.rows = rows;
        let usable = rows.saturating_sub(1);
        resize_pty(self.master_fd, cols, usable)?;
        self.miniterm.resize(cols, usable);
        Ok(())
    }

    // ── PTY output ────────────────────────────────────────────────────

    fn handle_pty_output(&mut self, raw: &[u8]) {
        let items = feed_interleaved(&mut self.osc_parser, raw);
        for item in items {
            match item {
                ParsedItem::Output(bytes) => self.miniterm.process_bytes(&bytes),
                ParsedItem::Event(event) => self.process_event(event),
            }
        }
    }

    fn process_event(&mut self, event: ShellEvent) {
        match event {
            ShellEvent::PromptStart => {
                self.capture_completed_block();
                self.blocks.scroll_to_bottom();
            }
            ShellEvent::PromptEnd => {
                // If agent finished and shell prompt returned, go back to Shell.
                if matches!(self.mode, Mode::AgentRunning) && self.agent.is_none() {
                    self.mode = Mode::Shell;
                }
            }
            _ => {}
        }
    }

    fn capture_completed_block(&mut self) {
        let mut rows = self.miniterm.take_scrolled_out();
        rows.extend(self.miniterm.grid().iter().cloned());
        self.blocks.add_block(rows);
        // Reset rather than resize: resize preserves content, which causes
        // the same grid to be re-captured on every subsequent PromptStart
        // (duplicate blocks) and keeps a stale cursor position that inflates
        // active_height, starving the scrollback region of display rows.
        self.miniterm.reset();
    }

    fn drain_pty(&mut self, buf: &mut [u8]) {
        loop {
            match read(self.master_fd, buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let (filtered, _) = self.osc_parser.feed(&buf[..n]);
                    if !filtered.is_empty() {
                        self.miniterm.process_bytes(&filtered);
                    }
                }
            }
        }
    }

    // ── Agent subprocess output ───────────────────────────────────────

    fn handle_agent_output(&mut self) {
        let agent = match self.agent.as_mut() {
            Some(a) => a,
            None => return,
        };

        let lines = agent.read_lines();
        let cols = self.cols as usize;

        for line in &lines {
            self.maybe_extract_conversation_id(line);
            self.process_agent_line(line, cols);
        }

        // Check if the agent process has finished.
        if let Some(agent) = self.agent.as_mut() {
            if agent.is_finished() {
                let final_lines = agent.read_lines();
                for line in &final_lines {
                    self.maybe_extract_conversation_id(line);
                    self.process_agent_line(line, cols);
                }
                self.flush_streaming_lines();
                self.agent = None;
                self.blocks.add_styled_line(
                    "✓ agent finished",
                    Color::Indexed(2),
                    CellFlags::DIM,
                    cols,
                );
                self.mode = Mode::Shell;
            }
        }
    }

    fn process_agent_line(&mut self, line: &str, cols: usize) {
        let json: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return,
        };
        let event_type = match json.get("type").and_then(|t| t.as_str()) {
            Some(t) => t,
            None => return,
        };

        match event_type {
            "agent" | "agent_reasoning" => {
                let text = match json.get("text").and_then(|t| t.as_str()) {
                    Some(t) if !t.trim().is_empty() => t,
                    _ => return,
                };
                let color = if event_type == "agent_reasoning" {
                    Color::Indexed(8)
                } else {
                    Color::Default
                };
                let flags = CellFlags::empty();
                self.append_streaming_text(text, color, flags);
            }
            "system" => {}
            _ => {
                // Non-text events: flush streaming text first, then add as permanent scrollback.
                self.flush_streaming_lines();
                if let Some((text, color, flags)) = parse_agent_event(line, cols) {
                    for text_line in text.lines() {
                        self.blocks.add_styled_line(text_line, color, flags, cols);
                    }
                }
            }
        }
    }

    fn maybe_extract_conversation_id(&mut self, line: &str) {
        if self.conversation_id.is_some() {
            return;
        }
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if json.get("type").and_then(|t| t.as_str()) == Some("system") {
                if let Some(id) = json.get("conversation_id").and_then(|c| c.as_str()) {
                    self.conversation_id = Some(id.to_string());
                }
            }
        }
    }

    // ── Stdin handling ────────────────────────────────────────────────

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
                    self.blocks.scroll_to_bottom();
                    nix_write(self.master_fd, bytes).context("write to pty")?;
                }
            }
            Mode::AgentInput { .. } => {
                self.handle_agent_input_bytes(bytes)?;
            }
            Mode::AgentRunning => {
                if bytes.contains(&0x03) {
                    self.cancel_agent();
                }
            }
        }
        Ok(())
    }

    // ── Agent input mode ──────────────────────────────────────────────

    fn enter_agent_mode(&mut self) {
        self.mode = Mode::AgentInput {
            buffer: String::new(),
            cursor: 0,
        };
    }

    fn exit_agent_mode(&mut self) {
        self.mode = Mode::Shell;
        let _ = nix_write(self.master_fd, b"\n");
    }

    fn cancel_agent(&mut self) {
        if let Some(mut agent) = self.agent.take() {
            agent.kill();
        }
        self.flush_streaming_lines();
        self.mode = Mode::Shell;
        self.blocks.add_styled_line(
            "✗ agent canceled",
            Color::Indexed(3),
            CellFlags::DIM,
            self.cols as usize,
        );
    }

    fn append_streaming_text(&mut self, text: &str, color: Color, flags: CellFlags) {
        let cols = self.cols as usize;
        let attr = CellAttr { fg: color, bg: Color::Default, flags };
        for ch in text.chars() {
            if ch == '\n' || self.streaming_col >= cols {
                self.streaming_lines.push(vec![Cell::default(); cols]);
                self.streaming_col = 0;
                if ch == '\n' {
                    continue;
                }
            }
            if self.streaming_lines.is_empty() {
                self.streaming_lines.push(vec![Cell::default(); cols]);
            }
            let last = self.streaming_lines.last_mut().unwrap();
            if self.streaming_col < cols {
                last[self.streaming_col] = Cell::with_char(ch, attr);
                self.streaming_col += 1;
            }
        }
    }

    fn flush_streaming_lines(&mut self) {
        if self.streaming_lines.is_empty() {
            return;
        }
        let rows = std::mem::take(&mut self.streaming_lines);
        self.streaming_col = 0;
        self.blocks.add_block(rows);
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
        Ok(())
    }

    // ── Agent submission ──────────────────────────────────────────────

    fn submit_agent_query(&mut self) -> Result<()> {
        let prompt = match &self.mode {
            Mode::AgentInput { buffer, .. } => buffer.clone(),
            _ => return Ok(()),
        };

        if prompt.trim().is_empty() {
            self.exit_agent_mode();
            return Ok(());
        }

        if prompt.trim() == "exit" {
            self.should_exit = true;
            return Ok(());
        }

        self.blocks.add_styled_line(
            &format!("🤖 {prompt}"),
            Color::Indexed(5),
            CellFlags::empty(),
            self.cols as usize,
        );

        // Spawn the real agent subprocess, continuing the conversation if one exists.
        match AgentProcess::spawn(&prompt, self.conversation_id.as_deref()) {
            Ok(agent) => {
                self.agent = Some(agent);
                self.mode = Mode::AgentRunning;
            }
            Err(e) => {
                self.blocks.add_styled_line(
                    &format!("✗ failed to start agent: {e}"),
                    Color::Indexed(1),
                    CellFlags::empty(),
                    self.cols as usize,
                );
                self.mode = Mode::Shell;
                let _ = nix_write(self.master_fd, b"\n");
            }
        }

        Ok(())
    }
}
