use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "extd", version, about = "The extboard server")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    // Run the HTTP server on loopback.
    Serve {
        // Port to bind on 127.0.0.1.
        #[arg(long, default_value_t = 7777)]
        port: u16,
    },
}
