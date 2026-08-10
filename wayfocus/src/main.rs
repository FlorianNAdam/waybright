use std::error::Error;

use clap::{Parser, Subcommand};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    FocusedOutput,
    Compositor,
}

fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command.unwrap_or(Command::FocusedOutput) {
        Command::FocusedOutput => {
            println!("{}", wayfocus::focused_output()?);
        }
        Command::Compositor => {
            println!("{}", compositor_name(wayfocus::detect_compositor()));
        }
    }

    Ok(())
}

fn compositor_name(compositor: wayfocus::Compositor) -> &'static str {
    match compositor {
        wayfocus::Compositor::Sway => "sway",
        wayfocus::Compositor::Hyprland => "hyprland",
        wayfocus::Compositor::Niri => "niri",
        wayfocus::Compositor::Kwin => "kwin",
        wayfocus::Compositor::Unknown => "unknown",
    }
}
