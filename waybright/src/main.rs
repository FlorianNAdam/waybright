use std::error::Error;

use clap::{Parser, Subcommand};
use serde_json::{Map, json};
use waybright::{client, daemon};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Daemon {
        #[arg(long, default_value_t = 50)]
        hardware_min: u8,
    },
    List {
        #[arg(long)]
        json: bool,
    },
    Get {
        name: String,
    },
    Set {
        name: String,
        #[arg(allow_hyphen_values = true)]
        percent: String,
    },
    Refresh,
}

fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Daemon { hardware_min } => daemon::run(daemon::Config { hardware_min }),
        Command::List { json } => list_outputs(json),
        Command::Get { name } => {
            println!("{}%", client::get(&name)?);
            Ok(())
        }
        Command::Set { name, percent } => {
            client::set(&name, &percent)?;
            Ok(())
        }
        Command::Refresh => {
            client::refresh()?;
            Ok(())
        }
    }
}

fn list_outputs(as_json: bool) -> Result<(), Box<dyn Error>> {
    let outputs = client::list()?;
    if as_json {
        let values = outputs
            .into_iter()
            .map(|output| {
                (
                    output.name,
                    json!({
                        "brightness": output.brightness,
                        "hardware": output.hardware,
                        "software": output.software,
                    }),
                )
            })
            .collect::<Map<_, _>>();
        println!("{}", serde_json::to_string_pretty(&values)?);
    } else {
        for output in outputs {
            println!("{}", output.name);
            println!("  brightness: {}%", output.brightness);
            match output.hardware {
                Some(hardware) => println!("  hardware: {hardware}%"),
                None => println!("  hardware: unavailable"),
            }
            match output.software {
                Some(software) => println!("  software: {software}%"),
                None => println!("  software: unavailable"),
            }
        }
    }
    Ok(())
}
