use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, NaiveDateTime, NaiveTime, Utc};
use colored::Colorize;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use taskchampion::storage::AccessMode;
use taskchampion::{Replica, StorageConfig, Uuid};

use crate::taskparser::{self, ObsidianTask, ObsidianTaskBuilder};

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
                    .and_time(MIDNIGHT)
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
        // 1. If task has UUID, find it in TC DB
        let mut ops = taskchampion::Operations::new();

        match task.uuid {
            Some(_) => println!("  {}", format!("{}", task.to_string()).blue()),
            None => println!("  {}", format!("{}", task.to_string()).green()),
        }

        if let Some(uuid) = task.uuid {
            if let Some(mut tc_task) = self.replica.get_task(uuid).ok().flatten() {
                // If equal, skip processing
                if *task == tc_task {
                    println!("{}", "      No changes".yellow());
                    return Ok(false);
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

                const MIDNIGHT: NaiveTime = chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap();

                // Due date update
                if !task.compare_due(&tc_task) {
                    println!(
                        "      {}",
                        format!(
                            "Due: {:?} -> {:?}",
                            tc_task.get_due().map(|due| due.with_timezone(&self.tz)),
                            task.due.map(|due| due
                                .and_time(MIDNIGHT)
                                .and_local_timezone(task.tz)
                                .earliest())
                        )
                        .yellow()
                    );
                    tc_task.set_due(
                        task.due.map(|date| {
                            date.and_time(MIDNIGHT)
                                .and_local_timezone(self.tz)
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
                                .and_time(MIDNIGHT)
                                .and_local_timezone(task.tz)
                                .earliest())
                        )
                        .red()
                    );
                    tc_task.set_wait(
                        task.start.map(|date| {
                            date.and_time(MIDNIGHT)
                                .and_local_timezone(self.tz)
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
                                .and_time(MIDNIGHT)
                                .and_local_timezone(task.tz)
                                .earliest())
                        )
                        .red()
                    );
                    tc_task.set_value(
                        "end",
                        task.done.map(|ed| {
                            ed.and_time(MIDNIGHT)
                                .and_local_timezone(self.tz)
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
                                .and_time(MIDNIGHT)
                                .and_local_timezone(task.tz)
                                .earliest())
                        )
                        .red()
                    );
                    tc_task.set_value(
                        "end",
                        task.canceled.map(|ed| {
                            ed.and_time(MIDNIGHT)
                                .and_local_timezone(self.tz)
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
                                .and_time(MIDNIGHT)
                                .and_local_timezone(task.tz)
                                .earliest())
                        )
                        .red()
                    );
                    tc_task.set_value(
                        "scheduled",
                        task.scheduled.map(|ed| {
                            ed.and_time(MIDNIGHT)
                                .and_local_timezone(self.tz)
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
            // Generate UUID and create task
            const MIDNIGHT: NaiveTime = NaiveTime::from_hms(0, 0, 0);
            let uuid = Uuid::new_v4();
            task.uuid = Some(uuid);
            let mut tc_task = self.replica.create_task(uuid, &mut ops)?;
            tc_task.set_status(task.status.clone().into(), &mut ops)?;
            tc_task.set_description(task.description.clone(), &mut ops)?;
            tc_task.set_value(
                "due",
                task.due.map(|x| {
                    x.and_time(MIDNIGHT)
                        .and_local_timezone(self.tz)
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
                    x.and_time(MIDNIGHT)
                        .and_local_timezone(self.tz)
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
                    x.and_time(MIDNIGHT)
                        .and_local_timezone(self.tz)
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
                    x.and_time(MIDNIGHT)
                        .and_local_timezone(self.tz)
                        .unwrap()
                        .to_utc()
                        .timestamp()
                        .to_string()
                }),
                &mut ops,
            )?;
            tc_task.set_value(
                "end",
                task.done.map(|x| {
                    x.and_time(MIDNIGHT)
                        .and_local_timezone(self.tz)
                        .unwrap()
                        .to_utc()
                        .timestamp()
                        .to_string()
                }),
                &mut ops,
            )?;
            tc_task.set_value(
                "end",
                task.canceled.map(|x| {
                    x.and_time(MIDNIGHT)
                        .and_local_timezone(self.tz)
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

            if let Some(file_name) = file.as_ref().file_stem() {
                if let Some(vault) = vault_path {
                    if let Some(vault_name) = vault.as_ref().file_name() {
                        let timestamp = Utc::now().timestamp();
                        let annotation = String::from(format!("annotation_{timestamp}"));
                        let task_open = String::from(format!(
                            "obsidian://open?vault={}&file={}",
                            vault_name.to_str().unwrap(),
                            file_name.to_str().unwrap()
                        ));
                        tc_task.set_value(annotation, Some(task_open), &mut ops)?;
                    }
                }
            }

            self.replica
                .commit_operations(ops)
                .context("Failed to commit operations")?;

            return Ok(true);
        }
    }

    pub fn tc_to_md(&mut self, task: &ObsidianTask, tz: &chrono_tz::Tz) -> Option<ObsidianTask> {
        const MIDNIGHT: NaiveTime =
            chrono::NaiveTime::from_hms_opt(0, 0, 0).expect("Invalid timestamp");
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
}

#[derive(Debug, Clone)]
pub struct UpdateContext {
    pub line: usize,
    pub task: ObsidianTask,
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

    // Iterate through updates and replace those lines
    let mut tasks = Vec::with_capacity(updates.len());
    for update in updates {
        let trimmed = file_lines[update.line].trim_start();
        let whitespace_len = file_lines[update.line].len() - trimmed.len();
        let whitespace = &file_lines[update.line][0..whitespace_len];
        tasks.push(format!("{}{}", whitespace, update.task.to_string()));
    }
    for (index, update) in updates.iter().enumerate() {
        file_lines[update.line] = &tasks[index];
    }

    // Write to temp file
    let mut buf_writer = BufWriter::new(std::fs::File::create(&temp_path)?);
    for line in file_lines {
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
    use std::io::Read;
    use std::str::FromStr;

    use chrono_tz::UTC;
    use taskchampion::{Operations, Status, Task, Uuid};

    use crate::taskparser::Priority;
    use crate::testutil::{self, TaskBuilder, TestContext, create_mem_replica};
    use std::path::Path;
    use testfile::TestFile;

    use super::*;

    use pretty_assertions::{assert_eq, assert_ne};

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
            },
            UpdateContext {
                line: 3,
                task: obsidian_task.clone(),
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
        assert_eq!("obsidian://open?vault=test2&file=test1", annotation);
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
