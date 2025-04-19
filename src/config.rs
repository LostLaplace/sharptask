use std::{path::PathBuf, str::FromStr};
use std::fs;
use chrono_tz::Tz;
use serde::Deserialize;
use anyhow::{anyhow, Result};
use std::path::Path;
use clap::Parser;

#[derive(Debug)]
pub struct Config {
    pub vault_path: PathBuf,
    pub task_path: PathBuf,
    pub timezone: Tz
}

const DEFAULT_PATH: &str = "~/.sharptask/config.toml";

#[derive(Deserialize)]
struct ConfigFile {
    vault_path: Option<PathBuf>,
    task_path: Option<PathBuf>,
    timezone: Option<String>
}

impl Default for ConfigFile {
    fn default() -> Self {
        ConfigFile {
            vault_path: None,
            task_path: Some(PathBuf::from("~/.task")),
            timezone: localzone::get_local_zone()
        }
    }
}


#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    vault: Option<PathBuf>,
    #[arg(short, long)]
    task_db: Option<PathBuf>,
    #[arg(short, long)]
    config: Option<PathBuf>,
}

pub fn get() -> Config {
    // First parse the CLI arguments
    let cli = Cli::parse();

    // Now try to parse the passed in config file if it exists or
    // the default config file if not
    let parsed_config = match cli.config {
        Some(config_path) => parse(&config_path),
        None => parse(DEFAULT_PATH)
    }.unwrap_or(ConfigFile::default());

    // The user can override a few of the options via CLI flags,
    // ensure that these items are either defined in the config or 
    // via CLI
    let vault_path = cli.vault.or(parsed_config.vault_path)
        .map(|path| {
            let path_str = path.to_string_lossy();
            let expanded = shellexpand::tilde(&path_str);
            PathBuf::from(expanded.into_owned())
        }).expect("Vault must be provided via CLI or config");

    let task_path = cli.task_db.or(parsed_config.task_path)
        .map(|path| {
            let path_str = path.to_string_lossy();
            let expanded = shellexpand::tilde(&path_str);
            PathBuf::from(expanded.into_owned())
        }).expect("Task DB path must be provided via --task_db or config");

    let timezone = parsed_config.timezone.or(localzone::get_local_zone())
        .expect("Cannot determine a timezone");

    let tz = Tz::from_str(&timezone).expect(&format!("Cannot process timezone: {}", timezone));

    Config {
        vault_path,
        task_path,
        timezone: tz
    }
}

fn parse<P: AsRef<Path>>(config_path: P) -> Result<ConfigFile> {
    let contents = fs::read_to_string(&config_path)
        .map_err(|e| anyhow!("Cannot read config file: {}", e))?;
    let config: ConfigFile = toml::from_str(&contents)
        .map_err(|_| anyhow!("Cannot parse TOML"))?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_config() {
        let test_config = r#"vault_path = "~/myVault"
                             task_path = "~/taskPath"
                             timezone = "US/Central"
                         "#;
        let test_file = testfile::from(test_config);
        let my_config = parse(test_file).unwrap();
        assert_eq!(my_config.vault_path.unwrap(), PathBuf::from("~/myVault"));
        assert_eq!(my_config.task_path.unwrap(), PathBuf::from("~/taskPath"));
        assert_eq!(my_config.timezone.unwrap(), "US/Central".to_string());
    }
}
