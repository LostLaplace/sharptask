use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use colored::Colorize;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use taskchampion::storage::AccessMode;
use taskchampion::{Replica, StorageConfig, Uuid};

use crate::taskparser::{self, ObsidianTask};

pub struct TaskWarriorSync {
    replica: Replica,
    tz: chrono_tz::Tz,
}

macro_rules! print_date_diff {
    ($tz:expr, $task:expr, $tcTask:expr, $($taskMember:tt, $tcValue:expr), *) => {
        $(
        println!(
            "{}",
            format!(
                "      {:?} -> {:?}",
                $task.$taskMember.map(|val| val
                    .and_local_timezone($task.tz)
                    .earliest()
                    .expect("Invalid timestamp")),
                $tcTask.get_value($tcValue).map(|val| {
                    chrono::DateTime::from_timestamp(
                        val.parse::<i64>().expect("Invalid timestamp"),
                        0,
                    )
                    .expect("Invalid timestamp")
                    .with_timezone($tz)
                })
            )
            .yellow()
        );
        )*
    };
}

impl TaskWarriorSync {
    pub fn new(path: &PathBuf, tz: &chrono_tz::Tz) -> Result<Self> {
        let storage = StorageConfig::OnDisk {
            taskdb_dir: path.clone(),
            create_if_missing: false,
            access_mode: AccessMode::ReadWrite,
        }
        .into_storage()
        .context("Failed to build storage context")?;
        Ok(TaskWarriorSync {
            replica: Replica::new(storage),
            tz: tz.clone(),
        })
    }

    #[cfg(test)]
    fn from_replica(replica: Replica, tz: &chrono_tz::Tz) -> Self {
        TaskWarriorSync {
            replica,
            tz: tz.clone(),
        }
    }

    #[cfg(test)]
    fn get_replica(&mut self) -> &mut Replica {
        &mut self.replica
    }

    // Updates any taskchampion copies of the task to match the markdown representation
    // Returns true if the markdown should be updated, false if no further changes needed
    pub fn md_to_tc<T: AsRef<Path>>(
        &mut self,
        task: &mut ObsidianTask,
        file: T,
        vault_path: Option<T>,
    ) -> Result<bool> {
        // 1. If task has UUID, look it up in TC. A UUID can exist in the frontmatter
        //    but be absent from TC (e.g. synced against a different DB), in which case
        //    we fall through to the creation path and reuse the existing UUID.
        let mut ops = taskchampion::Operations::new();

        let existing_tc_task = task
            .uuid
            .and_then(|uuid| self.replica.get_task(uuid).ok().flatten());

        match existing_tc_task.is_some() {
            true => println!("  {}", format!("{}", task.to_string()).blue()),
            false => println!("  {}", format!("{}", task.to_string()).green()),
        }

        if let Some(mut tc_task) = existing_tc_task {
                // Equal task — fields are in sync. The only thing that may
                // still need updating is the obsidian:// annotation (e.g. legacy
                // stem-only format, or the source file moved). Refresh it if
                // needed and bail out either way.
                if *task == tc_task {
                    println!("{}", "      No changes".yellow());
                    if let Some(vault) = vault_path.as_ref() {
                        Self::set_obsidian_annotation(
                            &mut tc_task,
                            &mut ops,
                            file.as_ref(),
                            vault.as_ref(),
                        )?;
                    }
                    if ops.is_empty() {
                        return Ok(false);
                    }
                    return self
                        .replica
                        .commit_operations(ops)
                        .map(|_| false)
                        .context("Failed committing annotation refresh");
                }

                // Status update
                if !task.compare_status(&tc_task) {
                    println!(
                        "      {}",
                        format!("Status: {} -> {}", tc_task.get_status(), task.status).red()
                    );
                    tc_task.set_status(task.status.clone().into(), &mut ops)?;
                }

                // Description update
                if !task.compare_description(&tc_task) {
                    println!(
                        "      {}",
                        format!(
                            "Desc: {} -> {}",
                            tc_task.get_description(),
                            task.description
                        )
                        .red()
                    );
                    tc_task.set_description(task.description.clone(), &mut ops)?;
                }

                // Due date update
                if !task.compare_due(&tc_task) {
                    println!(
                        "      {}",
                        format!(
                            "Due: {:?} -> {:?}",
                            tc_task.get_due().map(|due| due.with_timezone(&self.tz)),
                            task.due.map(|due| due
                                .and_local_timezone(task.tz)
                                .earliest())
                        )
                        .yellow()
                    );
                    tc_task.set_due(
                        task.due.map(|date| {
                            date.and_local_timezone(self.tz)
                                .unwrap()
                                .to_utc()
                        }),
                        &mut ops,
                    )?;
                }

                // Wait date update
                if !task.compare_start(&tc_task) {
                    println!(
                        "      {}",
                        format!(
                            "Wait: {:?} -> {:?}",
                            tc_task.get_wait().map(|due| due.with_timezone(&self.tz)),
                            task.start.map(|start| start
                                .and_local_timezone(task.tz)
                                .earliest())
                        )
                        .red()
                    );
                    tc_task.set_wait(
                        task.start.map(|date| {
                            date.and_local_timezone(self.tz)
                                .unwrap()
                                .to_utc()
                        }),
                        &mut ops,
                    )?;
                }

                // Update priority
                if !task.compare_priority(&tc_task) {
                    tc_task.set_status(task.status.clone().into(), &mut ops)?;
                }

                // Update tags
                if !task.compare_tags(&tc_task) {
                    println!(
                        "      {}",
                        format!(
                            "Tags: {:?} -> {:?}",
                            tc_task
                                .get_tags()
                                .filter_map(|t| {
                                    if t.is_user() {
                                        return Some(t.to_string().replace("_tag", ""));
                                    }
                                    None
                                })
                                .collect::<Vec<String>>(),
                            task.tags,
                        )
                        .red()
                    );
                    // Clear out existing tags
                    for tag in tc_task
                        .get_tags()
                        .filter(|itm| itm.is_user())
                        .collect::<Vec<taskchampion::Tag>>()
                    {
                        let tag_string = format!("tag_{tag}");
                        tc_task.set_value(tag_string, None, &mut ops)?;
                    }

                    // Add new tags
                    for tag in &task.tags {
                        let tag_string = format!("tag_{tag}");
                        tc_task.set_value(tag_string, Some(String::new()), &mut ops)?;
                    }
                }

                // Update end date
                if task.status == taskparser::Status::Complete && !task.compare_done(&tc_task) {
                    println!(
                        "      {}",
                        format!(
                            "Complete Date: {:?} -> {:?}",
                            tc_task.get_value("end").map(|val| DateTime::from_timestamp(
                                val.parse().expect("Timestamp is not valid"),
                                0
                            )
                            .expect("Timestamp is not valid")
                            .with_timezone(&self.tz)),
                            task.done.map(|date| date
                                .and_local_timezone(task.tz)
                                .earliest())
                        )
                        .red()
                    );
                    tc_task.set_value(
                        "end",
                        task.done.map(|ed| {
                            ed.and_local_timezone(self.tz)
                                .unwrap()
                                .to_utc()
                                .timestamp()
                                .to_string()
                        }),
                        &mut ops,
                    )?;
                }

                if task.status == taskparser::Status::Canceled && !task.compare_canceled(&tc_task) {
                    println!(
                        "    {}",
                        format!(
                            "Canceled Date: {:?} -> {:?}",
                            tc_task.get_value("end").map(|val| DateTime::from_timestamp(
                                val.parse().expect("Timestamp is not valid"),
                                0
                            )
                            .expect("Timestamp is not valid")
                            .with_timezone(&self.tz)),
                            task.canceled.map(|date| date
                                .and_local_timezone(task.tz)
                                .earliest())
                        )
                        .red()
                    );
                    tc_task.set_value(
                        "end",
                        task.canceled.map(|ed| {
                            ed.and_local_timezone(self.tz)
                                .unwrap()
                                .to_utc()
                                .timestamp()
                                .to_string()
                        }),
                        &mut ops,
                    )?;
                }

                // Update scheduled
                if !task.compare_schedule(&tc_task) {
                    println!(
                        "    {}",
                        format!(
                            "Start Date: {:?} -> {:?}",
                            tc_task
                                .get_value("scheduled")
                                .map(|val| DateTime::from_timestamp(
                                    val.parse().expect("Timestamp is not valid"),
                                    0
                                )
                                .expect("Timestamp is not valid")
                                .with_timezone(&self.tz)),
                            task.start.map(|date| date
                                .and_local_timezone(task.tz)
                                .earliest())
                        )
                        .red()
                    );
                    tc_task.set_value(
                        "scheduled",
                        task.scheduled.map(|ed| {
                            ed.and_local_timezone(self.tz)
                                .unwrap()
                                .to_utc()
                                .timestamp()
                                .to_string()
                        }),
                        &mut ops,
                    )?;
                }

                // Update priority
                // Normal priority results in no special item in the task data
                if !task.compare_priority(&tc_task) {
                    println!(
                        "      {}",
                        format!("Priority: {} -> {}", tc_task.get_priority(), task.priority).red()
                    );
                    let pri = match task.priority {
                        taskparser::Priority::Normal => None,
                        taskparser::Priority::Lowest | taskparser::Priority::Low => Some("L"),
                        taskparser::Priority::Medium => Some("M"),
                        taskparser::Priority::High | taskparser::Priority::Highest => Some("H"),
                    };
                    tc_task.set_value("priority", pri.map(|x| x.to_string()), &mut ops)?;

                    // If highest priority, also set the +next tag
                    if task.priority == taskparser::Priority::Highest {
                        tc_task.set_value("tag_next", Some(String::from("")), &mut ops)?;
                    }
                }

                // Update project
                if !task.compare_project(&tc_task) {
                    println!(
                        "    {}",
                        format!(
                            "Project: {:?} -> {:?}",
                            tc_task.get_value("project"),
                            task.project
                        )
                        .red()
                    );
                    tc_task.set_value("project", task.project.clone(), &mut ops)?;
                }

            // Ensure the task carries an up-to-date obsidian:// annotation
            // pointing at its current source file. Tasks created before this
            // feature had no annotation, and tasks whose source file moved
            // need the annotation refreshed.
            if let Some(vault) = vault_path.as_ref() {
                Self::set_obsidian_annotation(&mut tc_task, &mut ops, file.as_ref(), vault.as_ref())?;
            }

            if ops.is_empty() {
                return Ok(false);
            }
            return self
                .replica
                .commit_operations(ops)
                .map(|_| false)
                .context("Failed committing operations");
        } else {
            // Create task, reusing UUID from frontmatter if present (e.g. synced against a
            // different DB), otherwise generate a fresh one.
            let uuid = task.uuid.unwrap_or_else(Uuid::new_v4);
            task.uuid = Some(uuid);
            let mut tc_task = self.replica.create_task(uuid, &mut ops)?;
            tc_task.set_status(task.status.clone().into(), &mut ops)?;
            tc_task.set_description(task.description.clone(), &mut ops)?;
            tc_task.set_value(
                "due",
                task.due.map(|x| {
                    x.and_local_timezone(self.tz)
                        .unwrap()
                        .to_utc()
                        .timestamp()
                        .to_string()
                }),
                &mut ops,
            )?;
            tc_task.set_value(
                "wait",
                task.start.map(|x| {
                    x.and_local_timezone(self.tz)
                        .unwrap()
                        .to_utc()
                        .timestamp()
                        .to_string()
                }),
                &mut ops,
            )?;
            tc_task.set_value(
                "scheduled",
                task.scheduled.map(|x| {
                    x.and_local_timezone(self.tz)
                        .unwrap()
                        .to_utc()
                        .timestamp()
                        .to_string()
                }),
                &mut ops,
            )?;
            tc_task.set_value(
                "created",
                task.created.map(|x| {
                    x.and_local_timezone(self.tz)
                        .unwrap()
                        .to_utc()
                        .timestamp()
                        .to_string()
                }),
                &mut ops,
            )?;
            // Pick whichever end date applies to this task's status.
            // Using two consecutive set_value("end", ...) would always overwrite
            // the first with the second, clearing a valid done date on completed tasks.
            let end_date = match task.status {
                taskparser::Status::Complete => task.done,
                taskparser::Status::Canceled => task.canceled,
                taskparser::Status::Pending => None,
            };
            tc_task.set_value(
                "end",
                end_date.map(|x| {
                    x.and_local_timezone(self.tz)
                        .unwrap()
                        .to_utc()
                        .timestamp()
                        .to_string()
                }),
                &mut ops,
            )?;

            let pri = match task.priority {
                taskparser::Priority::Lowest | taskparser::Priority::Low => Some("L"),
                taskparser::Priority::Normal => None,
                taskparser::Priority::Medium => Some("M"),
                taskparser::Priority::High | taskparser::Priority::Highest => Some("H"),
            };
            tc_task.set_value("priority", pri.map(|x| x.to_string()), &mut ops)?;
            if task.priority == taskparser::Priority::Highest {
                tc_task.set_value("tag_next", Some("".to_string()), &mut ops)?;
            }

            tc_task.set_value("project", task.project.clone(), &mut ops)?;

            for tag in &task.tags {
                let tag_str = format!("tag_{}", tag);
                tc_task.set_value(tag_str, Some("".to_string()), &mut ops)?;
            }

            if let Some(vault) = vault_path.as_ref() {
                Self::set_obsidian_annotation(&mut tc_task, &mut ops, file.as_ref(), vault.as_ref())?;
            }

            self.replica
                .commit_operations(ops)
                .context("Failed to commit operations")?;

            return Ok(true);
        }
    }

    /// Build an `obsidian://open?vault=…&file=…` URI for the given vault-relative
    /// file path. Returns `None` if either path lacks a usable name.
    fn build_obsidian_uri(file: &Path, vault: &Path) -> Option<String> {
        let vault_name = vault.file_name()?.to_str()?;
        let rel = file.strip_prefix(vault).unwrap_or(file);
        let rel_str = rel.to_str()?;
        Some(format!(
            "obsidian://open?vault={}&file={}",
            urlencoding::encode(vault_name),
            urlencoding::encode(rel_str),
        ))
    }

    /// Replace any existing `obsidian://` annotation on the task with one
    /// pointing at `file` (vault-relative). Other annotations are preserved.
    fn set_obsidian_annotation(
        tc_task: &mut taskchampion::Task,
        ops: &mut taskchampion::Operations,
        file: &Path,
        vault: &Path,
    ) -> Result<()> {
        let Some(new_uri) = Self::build_obsidian_uri(file, vault) else {
            return Ok(());
        };

        // Collect existing obsidian:// annotation keys; clear them.
        let stale: Vec<String> = tc_task
            .get_annotations()
            .filter(|a| a.description.starts_with("obsidian://"))
            .map(|a| format!("annotation_{}", a.entry.timestamp()))
            .collect();
        // Skip rewrite if the only existing obsidian:// annotation already matches.
        let already_correct = stale.len() == 1
            && tc_task
                .get_annotations()
                .find(|a| a.description.starts_with("obsidian://"))
                .map(|a| a.description == new_uri)
                .unwrap_or(false);
        if already_correct {
            return Ok(());
        }
        for key in stale {
            tc_task.set_value(key, None, ops)?;
        }

        // Pick a fresh timestamp; bump until it doesn't collide with an existing
        // annotation key (collision is rare but possible within the same second).
        let mut ts = Utc::now().timestamp();
        while tc_task.get_value(format!("annotation_{ts}")).is_some() {
            ts += 1;
        }
        tc_task.set_value(format!("annotation_{ts}"), Some(new_uri), ops)?;
        Ok(())
    }

    /// Parse a previously written annotation back into a vault-relative path.
    /// Returns `None` for annotations that aren't ours or that we can't parse.
    fn parse_obsidian_annotation(annotation: &str) -> Option<PathBuf> {
        let rest = annotation.strip_prefix("obsidian://open?")?;
        for pair in rest.split('&') {
            if let Some(val) = pair.strip_prefix("file=") {
                let decoded = urlencoding::decode(val).ok()?.into_owned();
                return Some(PathBuf::from(decoded));
            }
        }
        None
    }

    /// Returns the `reviewed` UDA date set by `tasksh review`, if present.
    /// The date is expressed in UTC so that comparisons are stable across timezones.
    pub fn get_reviewed(&mut self, uuid: Uuid) -> Option<NaiveDate> {
        self.replica
            .get_task(uuid)
            .ok()
            .flatten()
            .and_then(|task| {
                task.get_value("reviewed").and_then(|val| {
                    val.parse::<i64>().ok().and_then(|ts| {
                        chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.date_naive())
                    })
                })
            })
    }

    /// Writes a `reviewed` date back to a TC task (sets the `reviewed` UDA).
    /// Stored as midnight UTC so it round-trips cleanly with `get_reviewed`.
    pub fn sync_reviewed(&mut self, uuid: Uuid, reviewed: NaiveDate) -> Result<()> {
        const MIDNIGHT: NaiveTime = chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap();
        if let Some(mut tc_task) = self.replica.get_task(uuid).ok().flatten() {
            let mut ops = taskchampion::Operations::new();
            let ts = reviewed
                .and_time(MIDNIGHT)
                .and_utc()
                .timestamp()
                .to_string();
            tc_task.set_value("reviewed", Some(ts), &mut ops)?;
            self.replica
                .commit_operations(ops)
                .context("Failed to commit reviewed update")?;
        }
        Ok(())
    }

    pub fn tc_to_md(&mut self, task: &ObsidianTask, tz: &chrono_tz::Tz) -> Option<ObsidianTask> {
        // Compare the task with its taskchampion version,
        // if taskchampion exists and they don't match, return
        // the new string to put in the markdown
        if let Some(uuid) = task.uuid {
            let tc_task_opt = self.replica.get_task(uuid).ok().flatten();
            if let Some(tc_task) = tc_task_opt {
                if *task != tc_task {
                    if !task.compare_due(&tc_task) {
                        print_date_diff!(tz, task, tc_task, due, "due");
                    }
                    if !task.compare_schedule(&tc_task) {
                        print_date_diff!(tz, task, tc_task, scheduled, "scheduled");
                    }
                    if !task.compare_start(&tc_task) {
                        print_date_diff!(tz, task, tc_task, start, "wait");
                    }
                    if !task.compare_created(&tc_task) {
                        print_date_diff!(tz, task, tc_task, created, "created");
                    }
                    if !task.compare_done(&tc_task) {
                        print_date_diff!(tz, task, tc_task, done, "end");
                    }
                    if !task.compare_canceled(&tc_task) {
                        print_date_diff!(tz, task, tc_task, canceled, "end");
                    }
                    if !task.compare_status(&tc_task) {
                        println!(
                            "{}",
                            format!("      {} -> {}", task.status, tc_task.get_status()).yellow()
                        );
                    }
                    if !task.compare_description(&tc_task) {
                        println!(
                            "{}",
                            format!(
                                "      {} -> {}",
                                task.description,
                                tc_task.get_description()
                            )
                            .yellow()
                        );
                    }
                    if !task.compare_priority(&tc_task) {
                        println!(
                            "{}",
                            format!("      {} -> {}", task.priority, tc_task.get_priority())
                                .yellow()
                        );
                    }
                    if !task.compare_project(&tc_task) {
                        println!(
                            "{}",
                            format!(
                                "      {:?} -> {:?}",
                                task.project,
                                tc_task.get_value("project")
                            )
                            .yellow()
                        );
                    }
                    let obsidian_task = ObsidianTask::from(tc_task);
                    return Some(obsidian_task.with_tz(&self.tz));
                }
            }
        }
        None
    }

    /// Return all non-deleted tasks from TC as `(uuid, ObsidianTask)` pairs.
    /// Used by `tc-to-md` to find tasks that have no corresponding TaskNote yet.
    pub fn all_tasks(&mut self) -> Vec<(Uuid, ObsidianTask)> {
        self.replica
            .all_tasks()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(uuid, tc_task)| {
                use taskchampion::Status as TcStatus;
                // Skip deleted tasks — they don't need a TaskNote.
                if tc_task.get_status() == TcStatus::Deleted {
                    return None;
                }
                let task = ObsidianTask::from(tc_task).with_tz(&self.tz);
                Some((uuid, task))
            })
            .collect()
    }

    /// Return true if the task with this UUID exists in TC and is in the
    /// Deleted state. Returns false for missing tasks or any other status.
    pub fn is_deleted_in_tc(&mut self, uuid: Uuid) -> bool {
        use taskchampion::Status as TcStatus;
        self.replica
            .get_task(uuid)
            .ok()
            .flatten()
            .map(|t| t.get_status() == TcStatus::Deleted)
            .unwrap_or(false)
    }

    /// Mark the task as completed, stamping the end date to now.
    pub fn mark_done(&mut self, uuid: Uuid) -> Result<()> {
        use taskchampion::Status as TcStatus;
        let mut ops = taskchampion::Operations::new();
        let mut tc_task = self
            .replica
            .get_task(uuid)
            .context("Failed to look up task")?
            .ok_or_else(|| anyhow::anyhow!("Task {} not found in TC", uuid))?;
        tc_task.set_status(TcStatus::Completed, &mut ops)?;
        tc_task.set_value("end", Some(Utc::now().timestamp().to_string()), &mut ops)?;
        self.replica
            .commit_operations(ops)
            .context("Failed to commit done status")
    }

    /// Mark the task as deleted in TC (taskchampion's tombstone state).
    pub fn delete_task(&mut self, uuid: Uuid) -> Result<()> {
        use taskchampion::Status as TcStatus;
        let mut ops = taskchampion::Operations::new();
        let mut tc_task = self
            .replica
            .get_task(uuid)
            .context("Failed to look up task")?
            .ok_or_else(|| anyhow::anyhow!("Task {} not found in TC", uuid))?;
        tc_task.set_status(TcStatus::Deleted, &mut ops)?;
        self.replica
            .commit_operations(ops)
            .context("Failed to commit delete status")
    }

    /// Walk pending TC tasks and return those that have an `obsidian://`
    /// annotation but no longer have a live representation in the vault.
    ///
    /// A task is considered orphaned when its UUID appears nowhere:
    ///   - not in any inline `[[uuid: <uuid>|⚔️]]` reference in the vault,
    ///   - not in the `tc_uuid:` frontmatter of any TaskNote (when
    ///     `tasknotes_path` is set).
    ///
    /// Uses a "scan the recorded file first, then fall back to vault-wide"
    /// strategy for inline tasks; cheap when the task is still where the
    /// annotation says it is, correct when the user has moved it.
    pub fn find_orphaned_obsidian_tasks(
        &mut self,
        vault_path: &Path,
        tasknotes_path: Option<&Path>,
    ) -> Result<Vec<OrphanedTask>> {
        use taskchampion::Status as TcStatus;

        let all = self
            .replica
            .all_tasks()
            .context("Failed to load all tasks")?;

        // UUID sets are built lazily — most runs find no orphans and never
        // need to walk the vault.
        let mut vault_inline: Option<std::collections::HashSet<Uuid>> = None;
        let mut tasknote_uuids: Option<std::collections::HashSet<Uuid>> = None;

        let mut orphans = Vec::new();

        for (uuid, tc_task) in all {
            if tc_task.get_status() != TcStatus::Pending {
                continue;
            }

            let Some(rel_path) = tc_task
                .get_annotations()
                .find_map(|a| Self::parse_obsidian_annotation(&a.description))
            else {
                continue;
            };

            // Fast path for inline tasks: read just the file the annotation
            // names and look for the UUID. Skip when the path is a bare stem
            // (legacy annotation format) or when it resolves into the
            // TaskNotes folder (different check).
            let abs_path = vault_path.join(&rel_path);
            let looks_like_tasknote = tasknotes_path
                .map(|tn| abs_path.starts_with(tn))
                .unwrap_or(false);
            let path_looks_resolvable = rel_path.extension().is_some()
                || rel_path.components().count() > 1;

            if !looks_like_tasknote && path_looks_resolvable {
                let needle = format!("[[uuid: {}|", uuid);
                if std::fs::read_to_string(&abs_path)
                    .map(|c| c.contains(&needle))
                    .unwrap_or(false)
                {
                    continue;
                }
            }

            // Slow path: walk the vault (and TaskNotes folder) once, cache.
            let inline_set = match &vault_inline {
                Some(s) => s,
                None => {
                    let s = crate::hookhandler::find_all_inline_task_uuids(vault_path)
                        .unwrap_or_default();
                    vault_inline = Some(s);
                    vault_inline.as_ref().unwrap()
                }
            };
            if inline_set.contains(&uuid) {
                continue;
            }

            if let Some(tn_path) = tasknotes_path {
                let tn_set = match &tasknote_uuids {
                    Some(s) => s,
                    None => {
                        let s = collect_tasknote_uuids(tn_path, &self.tz);
                        tasknote_uuids = Some(s);
                        tasknote_uuids.as_ref().unwrap()
                    }
                };
                if tn_set.contains(&uuid) {
                    continue;
                }
            }

            orphans.push(OrphanedTask {
                uuid,
                description: tc_task.get_description().to_string(),
                expected_path: rel_path,
                is_tasknote: looks_like_tasknote,
            });
        }

        Ok(orphans)
    }
}

/// Walk a TaskNotes folder and collect every `tc_uuid` found in frontmatter.
fn collect_tasknote_uuids(
    tn_path: &Path,
    tz: &chrono_tz::Tz,
) -> std::collections::HashSet<Uuid> {
    use ignore::{WalkBuilder, types::TypesBuilder};
    let md_types = TypesBuilder::new()
        .add_defaults()
        .select("markdown")
        .build()
        .expect("Failed to build type matcher");
    WalkBuilder::new(tn_path)
        .types(md_types)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map_or(false, |ft| ft.is_file()))
        .filter_map(|e| {
            crate::tasknotes::parse_file(e.path(), tz)
                .ok()
                .flatten()
                .and_then(|(t, _)| t.uuid)
        })
        .collect()
}

/// A task that is still pending in TC but whose obsidian source we couldn't
/// find. Returned by `find_orphaned_obsidian_tasks`.
#[derive(Debug, Clone)]
pub struct OrphanedTask {
    pub uuid: Uuid,
    pub description: String,
    pub expected_path: PathBuf,
    pub is_tasknote: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateContext {
    pub line: usize,
    pub task: ObsidianTask,
    /// When true, the line is removed from the file entirely instead of
    /// being replaced with the rendered task. Used when TC reports the task
    /// as deleted so the inline checkbox vanishes from the vault note.
    pub delete: bool,
}

pub fn update_obsidian_tasks<T: AsRef<Path>>(path: T, updates: &[UpdateContext]) -> Result<()> {
    // If temp file already exists, delete it
    let temp_path = path.as_ref().with_extension(".temp");
    if temp_path.exists() {
        fs::remove_file(&temp_path)?;
    }

    // Read file into memory
    let file_string = fs::read_to_string(&path)?;
    let mut file_lines: Vec<&str> = file_string.lines().collect();

    // Build replacement strings for non-delete updates and collect the set of
    // line indexes to drop entirely.
    let mut tasks = Vec::with_capacity(updates.len());
    let mut drops: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for update in updates {
        if update.delete {
            drops.insert(update.line);
            tasks.push(String::new()); // placeholder so indexes line up
            continue;
        }
        let trimmed = file_lines[update.line].trim_start();
        let whitespace_len = file_lines[update.line].len() - trimmed.len();
        let whitespace = &file_lines[update.line][0..whitespace_len];
        tasks.push(format!("{}{}", whitespace, update.task.to_string()));
    }
    for (index, update) in updates.iter().enumerate() {
        if !update.delete {
            file_lines[update.line] = &tasks[index];
        }
    }

    // Write to temp file, skipping dropped lines
    let mut buf_writer = BufWriter::new(std::fs::File::create(&temp_path)?);
    for (idx, line) in file_lines.iter().enumerate() {
        if drops.contains(&idx) {
            continue;
        }
        buf_writer.write(line.as_bytes())?;
        write!(buf_writer, "\n")?;
    }

    // Delete original, rename temp
    fs::remove_file(&path)?;
    fs::rename(&temp_path, &path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    
    

    use crate::taskparser::ObsidianTaskBuilder;
    use crate::taskparser::Priority;
    use crate::testutil::{TaskBuilder, TestContext, create_mem_replica};
    use chrono_tz::UTC;
    
    
    

    use super::*;

    use pretty_assertions::assert_eq;

    #[test]
    fn test_file_update() {
        std::fs::remove_file("test.md");
        let mut test_file = std::fs::File::create_new("test.md").unwrap();
        writeln!(test_file, "This is a normal line");
        writeln!(test_file, "- [ ] This is a test");
        writeln!(test_file, "Another normal line");
        writeln!(test_file, "    - [ ] This is a second test");

        let obsidian_task = ObsidianTaskBuilder::new()
            .description("This is a passed test")
            .status(taskparser::Status::Complete)
            .build();

        let context = vec![
            UpdateContext {
                line: 1,
                task: obsidian_task.clone(),
                delete: false,
            },
            UpdateContext {
                line: 3,
                task: obsidian_task.clone(),
                delete: false,
            },
        ];

        assert!(update_obsidian_tasks("test.md", &context).is_ok());

        let updated_content = std::fs::read_to_string("test.md").unwrap();
        assert_eq!(
            updated_content,
            "This is a normal line\n- [x] This is a passed test\nAnother normal line\n    - [x] This is a passed test\n"
        );
        std::fs::remove_file("test.md");
    }

    #[test]
    fn test_file_delete_line() {
        let path = "test_delete.md";
        let _ = std::fs::remove_file(path);
        let mut test_file = std::fs::File::create_new(path).unwrap();
        writeln!(test_file, "Header").unwrap();
        writeln!(test_file, "- [ ] Keep me").unwrap();
        writeln!(test_file, "- [ ] Drop me").unwrap();
        writeln!(test_file, "Footer").unwrap();

        let keep = ObsidianTaskBuilder::new()
            .description("Keep me")
            .status(taskparser::Status::Pending)
            .build();
        let drop = ObsidianTaskBuilder::new()
            .description("Drop me")
            .status(taskparser::Status::Pending)
            .build();

        let context = vec![
            UpdateContext { line: 1, task: keep, delete: false },
            UpdateContext { line: 2, task: drop, delete: true },
        ];

        assert!(update_obsidian_tasks(path, &context).is_ok());
        let content = std::fs::read_to_string(path).unwrap();
        assert_eq!(content, "Header\n- [ ] Keep me\nFooter\n");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_update_due_date() {
        let mut replica = create_mem_replica();
        let mut context = TestContext::new(&mut replica);
        let mut tc_task = TaskBuilder::new(&mut context)
            .desc("Test task")
            .status(taskchampion::Status::Pending)
            .build();

        let mut ts = TaskWarriorSync::from_replica(replica, &UTC);

        let mut obs_task = ObsidianTaskBuilder::new()
            .uuid(tc_task.get_uuid())
            .description("Test task")
            .due_str("2025-05-28")
            .project(Some("My project"))
            .priority(Priority::Normal)
            .build();

        let result = ts.md_to_tc(&mut obs_task, "", None).unwrap();
        assert!(!result);

        tc_task = ts.replica.get_task(tc_task.get_uuid()).unwrap().unwrap();
        assert_eq!(obs_task, tc_task);
    }

    #[test]
    fn test_create_task() {
        let mut replica = create_mem_replica();
        let mut context = TestContext::new(&mut replica);
        let mut tc_task = TaskBuilder::new(&mut context)
            .desc("Test task")
            .status(taskchampion::Status::Pending)
            .build();

        let mut task = ObsidianTaskBuilder::new()
            .uuid(tc_task.get_uuid())
            .description("Test task")
            .status(taskparser::Status::Complete)
            .build();

        let mut ts = TaskWarriorSync::from_replica(replica, &UTC);

        let result = ts.md_to_tc(&mut task, "", None).unwrap();
        assert!(!result);

        tc_task = ts.replica.get_task(tc_task.get_uuid()).unwrap().unwrap();
        assert_eq!(tc_task.get_status(), taskchampion::Status::Completed);
    }

    #[test]
    fn test_highest_pri() {
        let mut replica = create_mem_replica();
        let mut context = TestContext::new(&mut replica);
        let mut tc_task = TaskBuilder::new(&mut context)
            .desc("Test task")
            .status(taskchampion::Status::Pending)
            .priority("")
            .build();

        let mut task = ObsidianTaskBuilder::new()
            .uuid(tc_task.get_uuid())
            .description("Test task")
            .priority(Priority::Highest)
            .build();

        let mut ts = TaskWarriorSync::from_replica(replica, &UTC);

        let result = ts.md_to_tc(&mut task, "", None).unwrap();
        assert!(!result);

        tc_task = ts.replica.get_task(tc_task.get_uuid()).unwrap().unwrap();
        assert_eq!(tc_task.get_priority(), "H");
        assert!(tc_task.get_value("tag_next").is_some());
    }

    #[test]
    fn test_pri_demote() {
        let mut replica = create_mem_replica();
        let mut context = TestContext::new(&mut replica);
        let mut tc_task = TaskBuilder::new(&mut context)
            .desc("Test task")
            .status(taskchampion::Status::Pending)
            .priority("H")
            .tags(&["next"])
            .build();

        let mut task = ObsidianTaskBuilder::new()
            .uuid(tc_task.get_uuid())
            .description("Test task")
            .priority(Priority::High)
            .build();

        let mut ts = TaskWarriorSync::from_replica(replica, &UTC);

        let result = ts.md_to_tc(&mut task, "", None).unwrap();
        assert!(!result);

        tc_task = ts.replica.get_task(tc_task.get_uuid()).unwrap().unwrap();
        assert_eq!(tc_task.get_priority(), "H");
        assert!(tc_task.get_value("tag_next").is_none());
        assert_eq!(task, tc_task);
    }

    #[test]
    fn test_new_task() {
        let replica = create_mem_replica();
        let mut task = ObsidianTaskBuilder::new()
            .description("Test")
            .priority(Priority::High)
            .build();

        let mut ts = TaskWarriorSync::from_replica(replica, &UTC);
        let result = ts.md_to_tc(&mut task, "test1.md", Some("/test2")).unwrap();
        assert!(result);

        assert_eq!(ts.replica.all_task_uuids().unwrap().len(), 1);
        let uuid = ts.replica.all_task_uuids().unwrap().pop().unwrap();
        let mut reference_task = task.clone();
        reference_task.uuid = Some(uuid.clone());

        let result_task = ts.replica.get_task(uuid).unwrap().unwrap();
        assert_eq!(reference_task, result_task);
        let task_data = result_task.into_task_data();
        let annotation = task_data
            .iter()
            .find_map(|x| {
                if x.0.starts_with("annotation") {
                    return Some(x.1);
                }
                None
            })
            .unwrap();
        assert_eq!("obsidian://open?vault=test2&file=test1.md", annotation);
    }

    #[test]
    fn test_timezone() {
        let mut o_task = ObsidianTaskBuilder::new()
            .tz(chrono_tz::America::Chicago)
            .description("Test")
            .due_str("2025-06-07")
            .build();

        let replica = create_mem_replica();
        let mut ts = TaskWarriorSync::from_replica(replica, &chrono_tz::America::Chicago);

        ts.md_to_tc(&mut o_task, "", None).unwrap();

        let uuid = ts.replica.all_task_uuids().unwrap()[0];
        let task = ts.replica.get_task(uuid).unwrap().unwrap();
        assert_eq!(
            task.get_due().unwrap().timestamp(),
            chrono::DateTime::parse_from_rfc3339("2025-06-07T05:00:00Z")
                .unwrap()
                .timestamp()
        );
    }
}
