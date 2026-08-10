use std::{error::Error, io};

use clap::{Parser, Subcommand};
use serde_json::{Map, json};
use waylevel::{PercentChange, apply_percent_change, parse_percent_change};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Daemon,
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
}

fn parse_brightness_change(value: &str) -> io::Result<PercentChange> {
    parse_percent_change(value)
}

fn resolve_output_name(name: &str) -> io::Result<String> {
    if name == "@focused" {
        return wayfocus::focused_output();
    }

    Ok(name.to_owned())
}

fn list_outputs(json: bool) -> Result<(), Box<dyn Error>> {
    let outputs = waydark::daemon_list_outputs()?;

    if json {
        let mut json_outputs = Map::new();
        for output in outputs {
            json_outputs.insert(
                output.name.clone(),
                json!({
                    "brightness": output.brightness,
                    "output": output.label,
                }),
            );
        }

        println!("{}", serde_json::to_string_pretty(&json_outputs)?);
    } else {
        for output in outputs {
            println!("{}", output.name);
            println!("  brightness: {}%", output.brightness);
            println!("  output: {}", output.label);
        }
    }

    Ok(())
}

fn get_brightness(name: &str) -> Result<(), Box<dyn Error>> {
    let name = resolve_output_name(name)?;
    println!("{}%", waydark::daemon_get_brightness(&name)?);
    Ok(())
}

fn set_brightness(name: &str, percent: &str) -> Result<(), Box<dyn Error>> {
    let change = parse_brightness_change(percent)?;

    if name == "@all" {
        for output in waydark::daemon_list_outputs()? {
            let percent = apply_percent_change(Some(u32::from(output.brightness)), change);
            waydark::daemon_set_brightness(&output.name, percent)?;
        }

        return Ok(());
    }

    let name = resolve_output_name(name)?;
    let current = waydark::daemon_get_brightness(&name)?;
    let percent = apply_percent_change(Some(u32::from(current)), change);
    waydark::daemon_set_brightness(&name, percent)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Daemon => waydark::run_daemon(),
        Command::List { json } => list_outputs(json),
        Command::Get { name } => get_brightness(&name),
        Command::Set { name, percent } => set_brightness(&name, &percent),
    }
}
