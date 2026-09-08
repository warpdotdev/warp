use anyhow::{Result, anyhow};

pub fn run() -> Result<()> {
    Err(anyhow!("Plugin host unsupported on WASM"))
}
