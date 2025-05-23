use std::{fs::File, io::BufWriter, path::Path, str::FromStr};

use grep::{regex::RegexMatcher, searcher::Searcher, searcher::sinks};
use ignore::{WalkBuilder, types::TypesBuilder};
//use taskchampion::{storage::AccessMode, Operations, Replica, Status, StorageConfig, Uuid};
//use regex::Regex;
use anyhow::{Context, Result, anyhow};
use colored::Colorize;
use tasksync::TaskWarriorSync;

mod config;
mod taskparser;
mod tasksync;

#[cfg(test)]
mod testutil;

fn main() -> Result<()> {
    let cfg = config::get();

    let mut paths = Vec::new();
    if let Some(file_path) = cfg.file_path {
        paths.push(file_path);
    } else {
        // Search vault for markdown files
    }

    for path in paths {
        let task_matcher = RegexMatcher::new_line_matcher(r"- \[ |-|x\] .*")
            .expect("Failed to build regex matcher");
        let mut lines = Vec::new();
        let sink = sinks::UTF8(|offset, text| {
            lines.push((offset, text.to_string()));
            Ok(true)
        });
        Searcher::new()
            .search_path(task_matcher, path.clone(), sink)
            .context("Failed during search")?;

        let file = File::options().write(true).open(&path)?;
        let mut buf_writer = BufWriter::new(file);

        for line in lines {
            if cfg.direction == config::Direction::MdToTc {
                if let Some(task) = taskparser::parse(line.1) {
                    let mut ts = TaskWarriorSync::new(&cfg.task_path)?;
                    let update = ts.md_to_tc(&task, path.clone(), cfg.vault_path.clone());
                    if update.is_ok() && update.unwrap() {
                        let new_task = task.to_string();
                    }
                }
            } else {
                // TODO: handle TcToMd case
            }
        }
    }

    Ok(())
    /*
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
    */
}
