use anyhow::{Result, anyhow};
use chrono_tz::Tz;
use clap::{Args, Parser, Subcommand};
use serde::Deserialize;
use std::fs;
use std::path::Path;
use std::{path::PathBuf, str::FromStr};

#[derive(Debug)]
pub struct Config {
    pub vault_path: Option<PathBuf>,
    pub file_path: Option<PathBuf>,
    pub task_path: PathBuf,
    pub direction: Direction,
    pub tz: chrono_tz::Tz,
}

const DEFAULT_PATH: &str = "~/.sharptask/config.toml";

#[derive(Deserialize)]
struct ConfigFile {
    vault_path: Option<PathBuf>,
    task_path: Option<PathBuf>,
    timezone: Option<String>,
}

impl Default for ConfigFile {
    fn default() -> Self {
        let tz = localzone::get_local_zone().unwrap_or(String::from("UTC"));
        ConfigFile {
            vault_path: None,
            task_path: Some(PathBuf::from("~/.task")),
            timezone: Some(tz),
        }
    }
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(flatten)]
    target: Target,
    #[arg(short, long)]
    task_db: Option<PathBuf>,
    #[arg(short, long)]
    config: Option<PathBuf>,
    #[arg(long = "tz")]
    timezone: Option<String>,
    #[command(subcommand)]
    dir: Direction,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Direction {
    MdToTc,
    TcToMd,
}

#[derive(Args, Debug)]
#[group(required = false, multiple = false)]
struct Target {
    #[arg(short, long)]
    vault: Option<PathBuf>,
    #[arg(short, long)]
    file: Option<PathBuf>,
}

pub fn get() -> Config {
    // First parse the CLI arguments
    let cli = Cli::parse();

    // Now try to parse the passed in config file if it exists or
    // the default config file if not
    let parsed_config = match cli.config {
        Some(config_path) => parse(&config_path),
        None => parse(DEFAULT_PATH),
    }
    .unwrap_or(ConfigFile::default());

    // The user can override a few of the options via CLI flags,
    // ensure that these items are either defined in the config or
    // via CLI
    let vault_path = cli.target.vault.or(parsed_config.vault_path).map(|path| {
        let path_str = path.to_string_lossy();
        let expanded = shellexpand::tilde(&path_str);
        PathBuf::from(expanded.into_owned())
    });

    let task_path = cli
        .task_db
        .or(parsed_config.task_path)
        .map(|path| {
            let path_str = path.to_string_lossy();
            let expanded = shellexpand::tilde(&path_str);
            PathBuf::from(expanded.into_owned())
        })
        .expect("Task DB path must be provided via --task_db or config");

    let tz: chrono_tz::Tz = cli
        .timezone
        .or(parsed_config.timezone)
        .expect(
            "TZ will default to the local timezone or UTC if not provided by commandline or config",
        )
        .parse()
        .expect("Unable to parse TZ");

    Config {
        vault_path,
        task_path,
        file_path: cli.target.file,
        direction: cli.dir,
        tz,
    }
}

fn parse<P: AsRef<Path>>(config_path: P) -> Result<ConfigFile> {
    let contents =
        fs::read_to_string(&config_path).map_err(|e| anyhow!("Cannot read config file: {}", e))?;
    let config: ConfigFile = toml::from_str(&contents).map_err(|_| anyhow!("Cannot parse TOML"))?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_config() {
        let test_config = r#"vault_path = "~/myVault"
                             task_path = "~/taskPath"
                         "#;
        let test_file = testfile::from(test_config);
        let my_config = parse(test_file).unwrap();
        assert_eq!(my_config.vault_path.unwrap(), PathBuf::from("~/myVault"));
        assert_eq!(my_config.task_path.unwrap(), PathBuf::from("~/taskPath"));
    }
}
