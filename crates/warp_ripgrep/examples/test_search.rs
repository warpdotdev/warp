use std::path::PathBuf;

use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <pattern> [paths...]", args[0]);
        std::process::exit(1);
    }

    let pattern = args[1].clone();
    let paths: Vec<PathBuf> = args[2..].iter().map(PathBuf::from).collect();

    println!("Searching for pattern: {pattern}");
    println!("In paths: {paths:?}");

    warp_ripgrep::search::run_search_subprocess(&[pattern], paths, false, false, None)?;

    Ok(())
}
