use std::env;
use std::ffi::CString;
use std::os::unix::io::RawFd;

use anyhow::{Context, Result};
use nix::fcntl::{fcntl, FcntlArg, FdFlag};
use nix::pty::openpty;
use nix::unistd::{close, dup2, execvp, fork, setsid, ForkResult, Pid};

pub struct PtyPair {
    pub master_fd: RawFd,
    pub child_pid: Pid,
}

pub fn spawn_shell() -> Result<PtyPair> {
    let pty = openpty(None, None).context("openpty failed")?;
    let master_fd = pty.master;
    let slave_fd = pty.slave;

    // Set FD_CLOEXEC on the master so it isn't leaked to child processes.
    let flags = fcntl(master_fd, FcntlArg::F_GETFD).context("F_GETFD")?;
    let mut fd_flags = FdFlag::from_bits_truncate(flags);
    fd_flags.insert(FdFlag::FD_CLOEXEC);
    fcntl(master_fd, FcntlArg::F_SETFD(fd_flags)).context("F_SETFD")?;

    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());

    match unsafe { fork().context("fork failed")? } {
        ForkResult::Child => {
            // Close master in child — it's only needed by the parent.
            let _ = close(master_fd);

            // Create a new session and set the slave as the controlling terminal.
            setsid().context("setsid")?;
            unsafe {
                if libc::ioctl(slave_fd, libc::TIOCSCTTY.into(), 0) < 0 {
                    return Err(anyhow::anyhow!("TIOCSCTTY failed"));
                }
            }

            // Redirect stdio to the slave PTY.
            dup2(slave_fd, libc::STDIN_FILENO).context("dup2 stdin")?;
            dup2(slave_fd, libc::STDOUT_FILENO).context("dup2 stdout")?;
            dup2(slave_fd, libc::STDERR_FILENO).context("dup2 stderr")?;
            if slave_fd > libc::STDERR_FILENO {
                let _ = close(slave_fd);
            }

            let shell_c = CString::new(shell.as_str()).context("CString")?;
            execvp(&shell_c, &[&shell_c]).context("execvp")?;

            // execvp only returns on error — unreachable on success.
            unreachable!();
        }
        ForkResult::Parent { child } => {
            // Close slave in parent — it's only needed by the child.
            let _ = close(slave_fd);

            Ok(PtyPair {
                master_fd,
                child_pid: child,
            })
        }
    }
}

pub fn resize_pty(master_fd: RawFd, cols: u16, rows: u16) -> Result<()> {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let ret = unsafe { libc::ioctl(master_fd, libc::TIOCSWINSZ, &ws) };
    if ret < 0 {
        Err(anyhow::anyhow!("TIOCSWINSZ failed: {}", std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

/// Copy the current host terminal size to the PTY master.
pub fn sync_pty_size(master_fd: RawFd) -> Result<(u16, u16)> {
    let (cols, rows) = crossterm::terminal::size().context("terminal::size")?;
    resize_pty(master_fd, cols, rows)?;
    Ok((cols, rows))
}
