use std::env;
use std::fs;
use std::io::Write;

use anyhow::{Context, Result};

const ZSH_INIT_SCRIPT: &str = include_str!("../assets/wsh_zsh_init.sh");

fn setup_shell_integration() -> Result<std::path::PathBuf> {
    let tmp_dir = env::temp_dir().join(format!("wsh-{}", std::process::id()));
    fs::create_dir_all(&tmp_dir).context("create temp ZDOTDIR")?;

    // Determine the user's real ZDOTDIR (or HOME as fallback).
    let real_zdotdir = env::var("ZDOTDIR")
        .unwrap_or_else(|_| env::var("HOME").unwrap_or_else(|_| "/".to_string()));

    // Write a .zshenv that sources the user's real one.
    let zshenv_content = format!(
        "if [[ -f \"{real_zdotdir}/.zshenv\" ]]; then\n  source \"{real_zdotdir}/.zshenv\"\nfi\n"
    );
    fs::write(tmp_dir.join(".zshenv"), zshenv_content).context("write .zshenv")?;

    // Write our init script as .zshrc (it sources the user's real .zshrc internally).
    let mut f = fs::File::create(tmp_dir.join(".zshrc")).context("create .zshrc")?;
    f.write_all(ZSH_INIT_SCRIPT.as_bytes())
        .context("write .zshrc")?;

    env::set_var("WSH_REAL_ZDOTDIR", &real_zdotdir);
    env::set_var("ZDOTDIR", &tmp_dir);

    Ok(tmp_dir)
}

fn main() -> Result<()> {
    env_logger::init();

    let tmp_dir = setup_shell_integration()?;
    let pair = wsh::pty::spawn_shell()?;
    let result = wsh::event_loop::run(pair.master_fd);

    let _ = fs::remove_dir_all(&tmp_dir);
    result
}
