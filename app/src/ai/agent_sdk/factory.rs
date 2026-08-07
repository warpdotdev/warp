use anyhow::Result;
use serde_json::json;
use warp_cli::factory::{FactoryCommand, FactoryDefaultCommand};
use warp_core::factory_config::{self, FactoryConfigError};

/// Runs a `warp factory` command. These operate purely on the local
/// `factory/config.json` file, so no server or app context is needed.
pub fn run(command: FactoryCommand) -> Result<()> {
    match command {
        FactoryCommand::Default(default_cmd) => run_default(default_cmd),
    }
}

fn run_default(command: FactoryDefaultCommand) -> Result<()> {
    match command {
        FactoryDefaultCommand::Get => {
            match factory_config::resolve_default() {
                Ok(Some(default)) => println!(
                    "{}",
                    json!({
                        "default_factory_uid": default.uid,
                        "default_factory_name": default.name,
                    })
                ),
                Ok(None) => println!("{{}}"),
                // A malformed file is surfaced as a warning and treated as "no
                // default", so a caller can fall back to discovery rather than
                // failing. The file itself is never modified.
                Err(FactoryConfigError::Malformed { path, .. }) => {
                    eprintln!(
                        "warning: the default factory config at {} is unreadable and will be ignored",
                        path.display()
                    );
                    println!("{{}}");
                }
                Err(err) => return Err(err.into()),
            }
            Ok(())
        }
        FactoryDefaultCommand::Set(args) => {
            factory_config::set_default(&args.uid, args.name.as_deref())?;
            println!("Set default factory to {}", args.uid);
            Ok(())
        }
        FactoryDefaultCommand::Clear => {
            factory_config::clear_default()?;
            println!("Cleared the default factory");
            Ok(())
        }
    }
}
