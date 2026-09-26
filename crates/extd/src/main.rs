//! `extd` — the one server. axum on 127.0.0.1:7777, plus the CLI subcommands
//! that make the model testable from a shell.
//!
//! See PLAN.md for the phase each module is filled in.

mod api;
mod cli;
mod commands;
mod events;
mod project;
mod store;
mod view;

use clap::Parser;

#[tokio::main]
async fn main() {
    // Display, not Debug: `Box<dyn Error>`'s Debug quotes the message.
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    match cli::Cli::parse().command {
        cli::Command::Serve { port } => api::serve(port).await?,
        cli::Command::Fmt { file, check } => cli::fmt(&file, check)?,
    }
    Ok(())
}
