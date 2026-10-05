mod api;
mod cli;
mod events;
mod project;
mod store;

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
        cli::Command::Serve { port, dist } => api::serve(port, dist).await?,
        cli::Command::Project { file, selection } => cli::project(&file, &selection)?,
        cli::Command::Unproject { file } => cli::unproject(&file)?,
        cli::Command::Fmt { file, check } => cli::fmt(&file, check)?,
        cli::Command::Pull {
            file,
            space,
            server,
        } => cli::pull(&file, space.as_deref(), server.as_deref())?,
        cli::Command::Push {
            file,
            space,
            server,
            base,
        } => cli::push(&file, space.as_deref(), server.as_deref(), &base)?,
    }
    Ok(())
}
