//use ignore::{types::TypesBuilder, WalkBuilder};
//use grep::{regex::RegexMatcher, searcher::Searcher, searcher::sinks};
use clap::Parser;
//use taskchampion::{storage::AccessMode, Operations, Replica, Status, StorageConfig, Uuid};
use std::path::PathBuf;
//use regex::Regex;
use colored::Colorize;
use anyhow::{Result, anyhow};

mod config;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    vault: Option<PathBuf>,
    task_db: Option<PathBuf>,
    config: Option<PathBuf>,
}

#[derive(Debug)]
struct AppConfig {
    vault_path: PathBuf,
    task_path: PathBuf,
    timezone: String
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let parsed_config = match cli.config {
        Some(config_path) => config::parse(&config_path),
        None => config::parse_from_default_path()
    }.unwrap_or(config::defaults());

    let vault_path = cli.vault.or(parsed_config.vault_path)
        .ok_or_else(|| anyhow!("Vault must be provided via --vault or config".red()))?;
    let task_path = cli.task_db.or(parsed_config.task_path)
        .ok_or_else(|| anyhow!("Task DB path must be provided via --task_db or config".red()))?;
    let timezone = parsed_config.timezone.or(localzone::get_local_zone())
        .ok_or_else(|| anyhow!("Cannot determine a timezone"))?;

    let my_config = AppConfig {
        vault_path,
        task_path,
        timezone
    };

    println!("{:?}", my_config);

    Ok(())

}
