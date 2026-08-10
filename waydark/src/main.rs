use std::{error::Error, io};

use clap::{Parser, Subcommand};
use waylevel::{PercentChange, apply_percent_change, parse_percent_change};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    List,
    Daemon,
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

fn list_outputs() -> Result<(), Box<dyn Error>> {
    match waydark::daemon_list_outputs() {
        Ok(outputs) => {
            for output in outputs {
                println!("{}", output.name);
                println!("  brightness: {}%", output.brightness);
                println!("  output: {}", output.label);
            }
        }
        Err(_) => {
            for output in waydark::list_outputs()? {
                println!("{}", output.name);
                println!("  output: {}", output.label);
            }
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
    match Cli::parse().command.unwrap_or(Command::List) {
        Command::List => list_outputs(),
        Command::Daemon => waydark::run_daemon(),
        Command::Get { name } => get_brightness(&name),
        Command::Set { name, percent } => set_brightness(&name, &percent),
    }
}
