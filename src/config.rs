use anyhow::{Context, Result, anyhow};
use clap::{Args, Parser, Subcommand};
use serde::Deserialize;
use shellexpand::full;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug)]
pub struct Config {
    pub vault_path: Option<PathBuf>,
    pub file_path: Option<PathBuf>,
    pub task_path: PathBuf,
    pub direction: Direction,
    pub tz: chrono_tz::Tz,
}

const DEFAULT_PATH: &str = "~/.sharptask/config.toml";

#[derive(Deserialize, Debug)]
struct ConfigFile {
    #[serde(default)]
    vault_path: Option<PathBuf>,
    #[serde(default = "default_task_path")]
    task_path: Option<PathBuf>,
    #[serde(default = "default_timezone")]
    timezone: Option<String>,
}

fn default_task_path() -> Option<PathBuf> {
    Some(PathBuf::from("~/.task"))
}

fn default_timezone() -> Option<String> {
    let tz = localzone::get_local_zone().unwrap_or(String::from("UTC"));
    Some(tz)
}

impl Default for ConfigFile {
    fn default() -> Self {
        ConfigFile {
            vault_path: None,
            task_path: default_task_path(),
            timezone: default_timezone(),
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
    let path = shellexpand::full(
        config_path
            .as_ref()
            .to_str()
            .context("Path contains invalid unicode characters")?,
    )
    .context("Unable to expand environment in path")?;
    let contents = fs::read_to_string(path.into_owned())
        .map_err(|e| anyhow!("Cannot read config file: {}", e))?;
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
