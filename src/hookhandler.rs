//! Taskwarrior `on-modify` hook handler.
//!
//! Reads the two-line hook protocol from stdin, echoes the new task JSON to
//! stdout (required by TW), then finds the corresponding Obsidian TaskNote
//! and rewrites its frontmatter to match the new task state.
//!
//! Using the JSON passed directly from TW rather than re-reading the TC
//! database avoids the race condition where `tc-to-md` would see the
//! *pre-modification* state (because TW has not yet committed the new values
//! when the hook runs).

use anyhow::{Context, Result, anyhow};
use chrono::NaiveDateTime;
use grep::{regex::RegexMatcher, searcher::SearcherBuilder, searcher::sinks::UTF8};
use ignore::{WalkBuilder, types::TypesBuilder};
use serde_json::Value;
use std::path::{Path, PathBuf};
use taskchampion::Uuid;

use crate::taskparser::{ObsidianTask, ObsidianTaskBuilder, Priority, Status};
use crate::tasknotes::{self, TaskNotesExtra};
use crate::tasksync::{UpdateContext, update_obsidian_tasks};

/// Run the on-add hook: read stdin (one JSON line), echo it back, create a
/// TaskNote if one doesn't already exist for this task.
///
/// Returns the task JSON string (for the caller to `print!` to stdout).
pub fn run_on_add(tasknotes_path: Option<&PathBuf>, tz: &chrono_tz::Tz) -> Result<String> {
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .context("Failed to read task JSON from stdin")?;
    let task_json = input.trim_end().to_string();

    let task_value: Value =
        serde_json::from_str(&task_json).context("Failed to parse task JSON from TW")?;

    if let Some(tn_path) = tasknotes_path {
        let obsidian_task = task_from_tw_json(&task_value, tz)?;

        // Only create a TaskNote if there isn't one already (e.g. task was
        // imported or re-added after a sync).
        let uuid_str = task_value
            .get("uuid")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let uuid: Option<Uuid> = uuid_str.parse().ok();

        let already_exists = uuid
            .and_then(|u| find_tasknote_by_uuid(tn_path, u).ok().flatten())
            .is_some();

        if !already_exists {
            crate::tasknotes::create_file(tn_path, &obsidian_task, &TaskNotesExtra::default())
                .with_context(|| format!("Failed to create TaskNote in {}", tn_path.display()))?;
        }
    }

    Ok(task_json)
}

/// Run the on-modify hook: read stdin, echo the new task JSON, update the
/// matching TaskNote file.
///
/// Returns the new-task JSON string (for the caller to `print!` to stdout).
pub fn run(
    tasknotes_path: Option<&PathBuf>,
    vault_path: Option<&PathBuf>,
    tz: &chrono_tz::Tz,
) -> Result<String> {
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .context("Failed to read original task from stdin")?;
    let _old_task_json = input.trim_end().to_string();

    let mut new_line = String::new();
    std::io::stdin()
        .read_line(&mut new_line)
        .context("Failed to read modified task from stdin")?;
    let new_task_json = new_line.trim_end().to_string();

    if new_task_json.is_empty() {
        // on-add passes only one line; just return whatever we got
        return Ok(_old_task_json);
    }

    let new_task_value: Value =
        serde_json::from_str(&new_task_json).context("Failed to parse new task JSON from TW")?;

    let uuid_str = new_task_value
        .get("uuid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Task JSON has no uuid field"))?;

    let uuid: Uuid = uuid_str
        .parse()
        .map_err(|_| anyhow!("Invalid UUID: {}", uuid_str))?;

    let obsidian_task = task_from_tw_json(&new_task_value, tz)?;

    if let Some(tn_path) = tasknotes_path {
        if let Some(file_path) = find_tasknote_by_uuid(tn_path, uuid)? {
            // Re-parse the existing file to get the Extra fields we want to preserve
            // (reviewed, dateCreated, dateModified, projects, etc.) then overwrite only
            // the managed fields.
            let extra = match tasknotes::parse_file(&file_path, tz)? {
                Some((_task, extra)) => extra,
                None => TaskNotesExtra::default(),
            };

            tasknotes::write_file(&file_path, &obsidian_task, &extra)
                .with_context(|| format!("Failed to write {}", file_path.display()))?;
        }
        // If no matching file is found the task has no TaskNote yet — that's fine.
    }

    // Update inline task in vault note (if any).
    if let Some(vpath) = vault_path {
        if let Some((file_path, line_num)) = find_inline_task_by_uuid(vpath, uuid)? {
            let update = UpdateContext {
                line: line_num,
                task: obsidian_task.clone(),
            };
            update_obsidian_tasks(&file_path, &[update])
                .with_context(|| format!("Failed to update inline task in {}", file_path.display()))?;
        }
    }

    Ok(new_task_json)
}

/// Scan `tasknotes_path` for the first file whose `tc_uuid` frontmatter field
/// matches `uuid`.  Returns `None` if no match is found.
fn find_tasknote_by_uuid(tasknotes_path: &Path, uuid: Uuid) -> Result<Option<PathBuf>> {
    let needle = uuid.to_string();

    let md_types = TypesBuilder::new()
        .add_defaults()
        .select("markdown")
        .build()
        .expect("Failed to build type matcher");

    let result = WalkBuilder::new(tasknotes_path)
        .types(md_types)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map_or(false, |ft| ft.is_file()))
        .map(|e| e.into_path())
        .find(|path| {
            // Fast path: check file contents for the UUID string before parsing YAML.
            let content = std::fs::read_to_string(path).unwrap_or_default();
            if !content.contains(&needle) {
                return false;
            }
            // Confirm via proper parse to avoid false positives.
            if let Ok(Some((task, _))) =
                tasknotes::parse_file(path, &chrono_tz::UTC)
            {
                task.uuid == Some(uuid)
            } else {
                false
            }
        });

    Ok(result)
}

/// Scan `vault_path` for an inline task line containing `[[uuid: <uuid>|⚔️]]`.
/// Returns the file path and 0-based line number if found.
fn find_inline_task_by_uuid(vault_path: &Path, uuid: Uuid) -> Result<Option<(PathBuf, usize)>> {
    let needle = format!("[[uuid: {}|", uuid);
    let pattern = format!(r"- \[.\].*\[\[uuid: {}", regex::escape(&uuid.to_string()));

    let md_types = TypesBuilder::new()
        .add_defaults()
        .select("markdown")
        .build()
        .expect("Failed to build type matcher");

    let matcher = RegexMatcher::new_line_matcher(&pattern)
        .context("Failed to build UUID regex matcher")?;

    let mut found: Option<(PathBuf, usize)> = None;

    for entry in WalkBuilder::new(vault_path)
        .types(md_types)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map_or(false, |ft| ft.is_file()))
    {
        let path = entry.into_path();
        // Fast path: skip files that don't mention the UUID at all.
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        if !content.contains(&needle) {
            continue;
        }

        let mut line_result: Option<usize> = None;
        let sink = UTF8(|lnum, _text| {
            line_result = Some(usize::try_from(lnum - 1).unwrap_or(0));
            Ok(false) // stop after first match
        });
        let _ = SearcherBuilder::new()
            .line_number(true)
            .build()
            .search_path(&matcher, &path, sink);

        if let Some(line_num) = line_result {
            found = Some((path, line_num));
            break;
        }
    }

    Ok(found)
}

/// Scan `vault_path` for all inline task UUIDs (`[[uuid: <uuid>|`).
/// Returns the set of UUIDs found across all vault markdown files.
pub fn find_all_inline_task_uuids(vault_path: &Path) -> Result<std::collections::HashSet<Uuid>> {
    let pattern = r"\[\[uuid: ([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\|";

    let md_types = TypesBuilder::new()
        .add_defaults()
        .select("markdown")
        .build()
        .expect("Failed to build type matcher");

    let matcher = RegexMatcher::new_line_matcher(pattern)
        .context("Failed to build inline UUID regex matcher")?;

    let mut uuids = std::collections::HashSet::new();
    let re = regex::Regex::new(pattern).unwrap();

    for entry in WalkBuilder::new(vault_path)
        .types(md_types)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map_or(false, |ft| ft.is_file()))
    {
        let path = entry.into_path();
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        if !content.contains("[[uuid:") {
            continue;
        }

        let mut line_texts: Vec<String> = Vec::new();
        let sink = UTF8(|_lnum, text| {
            line_texts.push(text.to_owned());
            Ok(true)
        });
        let _ = SearcherBuilder::new()
            .line_number(false)
            .build()
            .search_path(&matcher, &path, sink);

        for line in &line_texts {
            for cap in re.captures_iter(line) {
                if let Ok(uuid) = cap[1].parse::<Uuid>() {
                    uuids.insert(uuid);
                }
            }
        }
    }

    Ok(uuids)
}

fn task_from_tw_json(v: &Value, tz: &chrono_tz::Tz) -> Result<ObsidianTask> {
    let uuid: Option<Uuid> = v
        .get("uuid")
        .and_then(|x| x.as_str())
        .and_then(|s| s.parse().ok());

    let description = v
        .get("description")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();

    let status = match v.get("status").and_then(|x| x.as_str()).unwrap_or("pending") {
        "completed" => Status::Complete,
        "deleted" => Status::Canceled,
        _ => Status::Pending,
    };

    let priority = match v.get("priority").and_then(|x| x.as_str()).unwrap_or("") {
        "H" => {
            let is_next = v
                .get("tags")
                .and_then(|t| t.as_array())
                .map(|arr| arr.iter().any(|tag| tag.as_str() == Some("next")))
                .unwrap_or(false);
            if is_next { Priority::Highest } else { Priority::High }
        }
        "M" => Priority::Medium,
        "L" => Priority::Low,
        _ => Priority::Normal,
    };

    let due = parse_tw_date(v.get("due").and_then(|x| x.as_str()), tz);
    let start = parse_tw_date(v.get("wait").and_then(|x| x.as_str()), tz);
    let scheduled = parse_tw_date(v.get("scheduled").and_then(|x| x.as_str()), tz);
    let created = parse_tw_date(v.get("entry").and_then(|x| x.as_str()), tz);

    let (done, canceled) = match status {
        Status::Complete => (parse_tw_date(v.get("end").and_then(|x| x.as_str()), tz), None),
        Status::Canceled => (None, parse_tw_date(v.get("end").and_then(|x| x.as_str()), tz)),
        Status::Pending => (None, None),
    };

    let tags: Vec<String> = v
        .get("tags")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|tag| tag.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();

    let project = v
        .get("project")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());

    let mut builder = ObsidianTaskBuilder::new().tz(tz.clone());
    builder = builder
        .description(description)
        .status(status)
        .priority(priority)
        .tags(&tags)
        .project(project);

    if let Some(id) = uuid {
        builder = builder.uuid(id);
    }
    if let Some(d) = due {
        builder = builder.due(Some(d));
    }
    if let Some(d) = start {
        builder = builder.start(Some(d));
    }
    if let Some(d) = scheduled {
        builder = builder.scheduled(Some(d));
    }
    if let Some(d) = created {
        builder = builder.created(Some(d));
    }
    if let Some(d) = done {
        builder = builder.done(Some(d));
    }
    if let Some(d) = canceled {
        builder = builder.canceled(Some(d));
    }

    Ok(builder.build())
}

/// Parse a Taskwarrior date string (`20260101T000000Z`) into a `NaiveDateTime` in local time.
fn parse_tw_date(s: Option<&str>, tz: &chrono_tz::Tz) -> Option<NaiveDateTime> {
    let s = s?;
    // TW format: YYYYMMDDTHHMMSSz — always UTC (Z suffix). Strip the trailing Z
    // and parse as NaiveDateTime, then treat as UTC before converting to local.
    let s_trimmed = s.trim_end_matches('Z');
    chrono::NaiveDateTime::parse_from_str(s_trimmed, "%Y%m%dT%H%M%S")
        .ok()
        .map(|dt| dt.and_utc().with_timezone(tz).naive_local())
}
