use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about)]
pub struct Cli {
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Init,
    Accounts,
    Auth {
        #[arg(long, value_enum)]
        tan_type: Option<TanType>,
    },
    Sync,
    Enrich,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum TanType {
    #[value(name = "M_TAN")]
    MTan,
    #[value(name = "P_TAN")]
    PTan,
    #[value(name = "P_TAN_PUSH")]
    PTanPush,
}

impl TanType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TanType::MTan => "M_TAN",
            TanType::PTan => "P_TAN",
            TanType::PTanPush => "P_TAN_PUSH",
        }
    }
}
