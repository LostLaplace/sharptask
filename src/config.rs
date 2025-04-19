use std::path::PathBuf;
use std::fs;
use serde::Deserialize;
use anyhow::{anyhow, Result, Context};
use std::path::Path;

const DEFAULT_PATH: &str = "~/.sharptask/config.toml";

#[derive(Deserialize)]
pub struct Config {
    pub vault_path: Option<PathBuf>,
    pub task_path: Option<PathBuf>,
    pub timezone: Option<String>
}

pub fn defaults() -> Config {
    Config {
        vault_path: None,
        task_path: Some(PathBuf::from("~/.task")),
        timezone: None
    }
}

pub fn parse<P: AsRef<Path>>(config_path: P) -> Result<Config> {
    let contents = fs::read_to_string(&config_path)
        .context(format!("Failed to read from file: {}", config_path.as_ref().display()))
        .map_err(|e| anyhow!("Cannot read config file: {}", e))?;
    let config: Config = toml::from_str(&contents)
        .context(format!("Failed to parse TOML: \n{}", &contents))
        .map_err(|_| anyhow!("Cannot parse TOML"))?;
    Ok(config)
}

pub fn parse_from_default_path() -> Result<Config> {
    let config_path = PathBuf::from(DEFAULT_PATH);
    parse(&config_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_get_defaults() {
        let my_config = defaults();
        assert_eq!(my_config.vault_path, None);
        assert_eq!(my_config.task_path.unwrap(), PathBuf::from("~/.task"));
        assert_eq!(my_config.timezone, None);
    }

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
