use anyhow::{Context, Result, anyhow};
use colored::Colorize;
use grep::{regex::RegexMatcher, searcher::Searcher, searcher::sinks};
use ignore::{WalkBuilder, types::TypesBuilder};
use tasksync::{TaskWarriorSync, UpdateContext, update_obsidian_tasks};

mod config;
mod hookhandler;
mod taskparser;
mod tasknotes;
mod tasksync;

#[cfg(test)]
mod testutil;

fn main() -> Result<()> {
    let cfg = config::get();

    // ── on-modify hook mode ────────────────────────────────────────────────────
    if cfg.direction == config::Direction::Hook {
        let new_task_json =
            hookhandler::run(cfg.tasknotes_path.as_ref(), &cfg.tz).unwrap_or_else(|e| {
                eprintln!("sharptask hook error: {}", e);
                String::new()
            });
        if !new_task_json.is_empty() {
            println!("{}", new_task_json);
        }
        return Ok(());
    }

    // ── on-add hook mode ───────────────────────────────────────────────────────
    if cfg.direction == config::Direction::OnAdd {
        let task_json =
            hookhandler::run_on_add(cfg.tasknotes_path.as_ref(), &cfg.tz).unwrap_or_else(|e| {
                eprintln!("sharptask on-add error: {}", e);
                String::new()
            });
        if !task_json.is_empty() {
            println!("{}", task_json);
        }
        return Ok(());
    }

    let mut errors = 0;

    // ── Obsidian Tasks (inline markdown) ──────────────────────────────────────
    let mut paths = Vec::new();
    if let Some(ref file_path) = cfg.file_path {
        paths.push(file_path.clone());
    } else if let Some(ref vault_path) = cfg.vault_path {
        let md_types = TypesBuilder::new()
            .add_defaults()
            .select("markdown")
            .build()
            .expect("Failed to build type matcher");
        let walk_paths = WalkBuilder::new(vault_path)
            .types(md_types)
            .build()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().map_or(false, |ft| ft.is_file()))
            .map(|x| x.into_path())
            .filter(|path| {
                !cfg.excluded_paths.iter().any(|excl| {
                    // Support both absolute excluded paths and vault-relative ones.
                    let abs_excl = if excl.is_absolute() {
                        excl.clone()
                    } else {
                        vault_path.join(excl)
                    };
                    path.starts_with(&abs_excl)
                })
            });
        paths.extend(walk_paths);
    }

    for path in paths {
        println!("{}", format!("Processing: {}", &path.display()).blue());
        let task_matcher = RegexMatcher::new_line_matcher(r"- \[(?: |-|x)\] .*")
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
            // When a UUID filter is set, skip tasks that don't match.
            if let Some(filter_uuid) = cfg.uuid {
                if line.task.uuid != Some(filter_uuid) {
                    continue;
                }
            }
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

    // ── TaskNotes (one .md file per task, YAML frontmatter) ───────────────────
    if let Some(ref tn_path) = cfg.tasknotes_path {
        // When --file is given, only process that file if it lives inside the
        // TaskNotes folder; skip the entire loop otherwise (the file is a vault
        // inline-task note, already handled above).
        let tn_files: Vec<_> = match &cfg.file_path {
            Some(fp) if fp.starts_with(tn_path) => vec![fp.clone()],
            Some(_) => vec![],
            None => {
                let md_types = TypesBuilder::new()
                    .add_defaults()
                    .select("markdown")
                    .build()
                    .expect("Failed to build type matcher");
                WalkBuilder::new(tn_path)
                    .types(md_types)
                    .build()
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_type().map_or(false, |ft| ft.is_file()))
                    .map(|x| x.into_path())
                    .collect()
            }
        };

        for path in tn_files {
            println!(
                "{}",
                format!("Processing TaskNote: {}", &path.display()).blue()
            );

            let task_result = tasknotes::parse_file(&path, &cfg.tz);
            let (mut task, mut extra) = match task_result {
                Ok(Some(t)) => t,
                Ok(None) => {
                    println!(
                        "  {}",
                        format!("Skipping (no title/frontmatter): {}", path.display()).yellow()
                    );
                    continue;
                }
                Err(e) => {
                    println!(
                        "  {}",
                        format!("Failed to parse {}: {}", path.display(), e).red()
                    );
                    errors += 1;
                    continue;
                }
            };

            // When a UUID filter is set, skip files whose tc_uuid doesn't match.
            if let Some(filter_uuid) = cfg.uuid {
                if task.uuid != Some(filter_uuid) {
                    continue;
                }
            }

            // Project TaskNotes (tagged 'project') are Obsidian-only — they
            // have no direct TW representation; their children reference them
            // via tc_project / project: field.
            if task.tags.iter().any(|t| t == "project") {
                continue;
            }

            let mut sync = TaskWarriorSync::new(&cfg.task_path, &cfg.tz)
                .context("Failed to open task database")
                .expect("Should be able to access task database");

            if cfg.direction == config::Direction::MdToTc {
                match sync.md_to_tc(&mut task, &path, cfg.vault_path.as_ref()) {
                    Ok(true) => {
                        // New task: UUID was just assigned — write it back.
                        // Also push any reviewed date from the frontmatter to TC.
                        if let (Some(uuid), Some(reviewed)) = (task.uuid, extra.reviewed) {
                            if let Err(e) = sync.sync_reviewed(uuid, reviewed) {
                                println!("  {}", format!("Failed to sync reviewed for {}: {}", path.display(), e).red());
                            }
                        }
                        if let Err(e) = tasknotes::write_file(&path, &task, &extra) {
                            println!("  {}", format!("Failed to write {}: {}", path.display(), e).red());
                            errors += 1;
                        }
                    }
                    Ok(false) => {
                        // Existing task: sync reviewed date if it differs.
                        if let Some(uuid) = task.uuid {
                            let tc_reviewed = sync.get_reviewed(uuid);
                            if extra.reviewed != tc_reviewed {
                                if let Some(reviewed) = extra.reviewed {
                                    if let Err(e) = sync.sync_reviewed(uuid, reviewed) {
                                        println!("  {}", format!("Failed to sync reviewed for {}: {}", path.display(), e).red());
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        println!("  {}", format!("Sync error for {}: {}", path.display(), e).red());
                        errors += 1;
                    }
                }
            } else {
                let updated_task_opt = sync.tc_to_md(&task, &cfg.tz);
                // Check if the reviewed date changed in TC independently of other fields.
                let tc_reviewed = task.uuid.and_then(|uuid| sync.get_reviewed(uuid));
                let reviewed_changed = tc_reviewed != extra.reviewed;

                if updated_task_opt.is_some() || reviewed_changed {
                    extra.reviewed = tc_reviewed;
                    let task_to_write = updated_task_opt.as_ref().unwrap_or(&task);
                    if let Err(e) = tasknotes::write_file(&path, task_to_write, &extra) {
                        println!("  {}", format!("Failed to write {}: {}", path.display(), e).red());
                        errors += 1;
                    }
                }
            }
        }
    }

    if errors > 0 {
        return Err(anyhow!("{errors} files failed to update"));
    } else {
        return Ok(());
    }
}
