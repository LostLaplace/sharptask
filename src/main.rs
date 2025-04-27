use ignore::{types::TypesBuilder, WalkBuilder};
use grep::{regex::RegexMatcher, searcher::Searcher, searcher::sinks};
//use taskchampion::{storage::AccessMode, Operations, Replica, Status, StorageConfig, Uuid};
//use regex::Regex;
use colored::Colorize;
use anyhow::{Result, anyhow};

mod config;
mod taskparser;
mod tasksync;

fn main() -> Result<()> {
    let cfg = config::get(); 

    // 1. Find all md files in vault
    let md_types = TypesBuilder::new().add_defaults().select("markdown").build().expect("Failed to build type matcher");
    let paths: Vec<ignore::DirEntry> = WalkBuilder::new(cfg.vault_path).types(md_types)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map_or(false, |ft| ft.is_file())).collect();

    // 2. Search all files for matches for tasks
    for path in paths {
        let task_matcher = RegexMatcher::new_line_matcher(r"- \[ |-|x] .*").expect("Failed to build regex matcher");
        let sink = sinks::UTF8(|offset, text| {
            // 3. Parse each task line 
            Ok(true)
        });
        let _ = Searcher::new().search_path(task_matcher, path.path(), sink);
    }

    Ok(())
}
