mod cli;
mod comdirect;
mod commands;
mod config;
mod op;
mod paths;
mod paypal;
mod prompt;
mod ynab;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cli = cli::Cli::parse();
    let paths = paths::Paths::new(cli.config)?;
    let command = cli.command.unwrap_or(cli::Command::Sync);

    match command {
        cli::Command::Init => commands::run_init(&paths).await,
        cli::Command::Accounts => commands::run_accounts(&paths).await,
        cli::Command::Auth { tan_type } => commands::run_auth(&paths, tan_type).await,
        cli::Command::Sync => commands::run_sync(&paths).await,
        cli::Command::Enrich => commands::run_enrich(&paths).await,
    }
}
