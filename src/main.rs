use grep::{regex::RegexMatcher, searcher::Searcher, searcher::sinks};
use ignore::{WalkBuilder, types::TypesBuilder};
use anyhow::{Context, Result, anyhow};
use colored::Colorize;
use tasksync::{TaskWarriorSync, UpdateContext, update_obsidian_tasks};

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
        let md_types = TypesBuilder::new()
            .add_defaults()
            .select("markdown")
            .build()
            .expect("Failed to build type matcher");
        let walk_paths = WalkBuilder::new(cfg.vault_path.as_ref().expect("No vault set"))
            .types(md_types)
            .build()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().map_or(false, |ft| ft.is_file()))
            .map(|x| x.into_path());
        paths.extend(walk_paths);
    }

    let mut errors = 0;
    for path in paths {
        println!("{}", format!("Processing: {}", &path.display()).blue());
        let task_matcher = RegexMatcher::new_line_matcher(r"- \[ |-|x\] .*")
            .expect("Failed to build regex matcher");
        let mut lines = Vec::new();
        let sink = sinks::UTF8(|offset, text| {
            let task_option = taskparser::parse(text.to_string(), &cfg.tz);
            if let Some(task) = task_option {
                lines.push(UpdateContext {
                    line: usize::try_from(offset - 1).expect("Offset should fit"),
                    task,
                });
            } else {
                println!("  {}", format!("{} {}", "Failed to parse:", text).red());
            }
            Ok(true)
        });
        Searcher::new()
            .search_path(task_matcher, path.clone(), sink)
            .context("Failed during search")?;

        let mut updates = Vec::new();
        for line in lines.iter_mut() {
            let mut sync = TaskWarriorSync::new(&cfg.task_path, &cfg.tz)
                .context("Failed to open task database")
                .expect("Should be able to access task database");
            if cfg.direction == config::Direction::MdToTc {
                let update = sync.md_to_tc(&mut line.task, path.clone(), cfg.vault_path.clone());
                if update.is_ok() && update.unwrap() {
                    updates.push(line.clone());
                }
            } else {
                let update = sync.tc_to_md(&line.task, &cfg.tz);
                if let Some(task) = update {
                    let updated_line = UpdateContext { task, ..*line };
                    updates.push(updated_line);
                }
            }
        }
        let result = update_obsidian_tasks(&path, &updates);
        if result.is_err() {
            errors += 1;
        }
    }
    if errors > 0 {
        return Err(anyhow!("{errors} files failed to update"));
    } else {
        return Ok(());
    }
}
