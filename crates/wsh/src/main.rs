use anyhow::Result;

fn main() -> Result<()> {
    env_logger::init();

    let pair = wsh::pty::spawn_shell()?;
    wsh::event_loop::run(pair.master_fd)
}
