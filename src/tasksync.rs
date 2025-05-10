use anyhow::{anyhow, Context, Result};
use chrono::NaiveTime;
use std::path::{Path, PathBuf};
use taskchampion::storage::AccessMode;
use taskchampion::{Replica, StorageConfig, Uuid};

use crate::taskparser::{self, ObsidianTask, ObsidianTaskBuilder};

pub struct TaskWarriorSync {
    replica: Replica,
}

impl TaskWarriorSync {
    pub fn new(path: &PathBuf) -> Result<Self> {
        let storage = StorageConfig::OnDisk {
            taskdb_dir: path.clone(),
            create_if_missing: false,
            access_mode: AccessMode::ReadWrite,
        }
        .into_storage()
        .context("Failed to build storage context")?;
        Ok(TaskWarriorSync {
            replica: Replica::new(storage),
        })
    }

    #[cfg(test)]
    fn from_replica(replica: Replica) -> Self {
        TaskWarriorSync { replica }
    }

    #[cfg(test)]
    fn get_replica(&mut self) -> &mut Replica {
        &mut self.replica
    }

    // Updates any taskchampion copies of the task to match the markdown representation
    // Returns true if the markdown should be updated, false if no further changes needed
    pub fn md_to_tc<T: AsRef<Path>>(&mut self, task: &ObsidianTask, file: T) -> Result<bool> {
        // 1. If task has UUID, find it in TC DB
        let mut ops = taskchampion::Operations::new();

        if let Some(uuid) = task.uuid {
            if let Some(mut tc_task) = self.replica.get_task(uuid).ok().flatten() {
                // If equal, skip processing
                if *task == tc_task {
                    return Ok(false);
                }

                // Status update
                if !task.compare_status(&tc_task) {
                    tc_task.set_status(task.status.clone().into(), &mut ops)?;
                }

                // Description update
                if !task.compare_description(&tc_task) {
                    tc_task.set_description(tc_task.get_description().to_string(), &mut ops)?;
                }

                const MIDNIGHT: NaiveTime = chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap();

                // Due date update
                if !task.compare_due(&tc_task) {
                    tc_task.set_due(
                        task.due.map(|date| date.and_time(MIDNIGHT).and_utc()),
                        &mut ops,
                    )?;
                }

                // Wait date update
                if !task.compare_start(&tc_task) {
                    tc_task.set_wait(
                        task.start.map(|date| date.and_time(MIDNIGHT).and_utc()),
                        &mut ops,
                    )?;
                }

                // Update priority
                if !task.compare_priority(&tc_task) {
                    tc_task.set_status(task.status.clone().into(), &mut ops)?;
                }

                // Update tags
                if !task.compare_tags(&tc_task) {
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
                    tc_task.set_value(
                        "end",
                        task.done
                            .map(|ed| ed.and_time(MIDNIGHT).and_utc().timestamp().to_string()),
                        &mut ops,
                    )?;
                }

                if task.status == taskparser::Status::Canceled && !task.compare_canceled(&tc_task) {
                    tc_task.set_value(
                        "end",
                        task.canceled
                            .map(|ed| ed.and_time(MIDNIGHT).and_utc().timestamp().to_string()),
                        &mut ops,
                    )?;
                }

                // Update scheduled
                if !task.compare_schedule(&tc_task) {
                    tc_task.set_value(
                        "scheduled",
                        task.scheduled
                            .map(|ed| ed.and_time(MIDNIGHT).and_utc().timestamp().to_string()),
                        &mut ops,
                    )?;
                }

                // Update priority
                // Normal priority results in no special item in the task data
                if !task.compare_priority(&tc_task) {
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
            let mut tc_task = self.replica.create_task(uuid, &mut ops)?;
            tc_task.set_status(task.status.clone().into(), &mut ops)?;
            tc_task.set_description(task.description.clone(), &mut ops)?;
            tc_task.set_value(
                "due",
                task.due
                    .map(|x| x.and_time(MIDNIGHT).and_utc().timestamp().to_string()),
                &mut ops,
            )?;
            tc_task.set_value(
                "wait",
                task.start
                    .map(|x| x.and_time(MIDNIGHT).and_utc().timestamp().to_string()),
                &mut ops,
            )?;
            tc_task.set_value(
                "scheduled",
                task.scheduled
                    .map(|x| x.and_time(MIDNIGHT).and_utc().timestamp().to_string()),
                &mut ops,
            )?;
            tc_task.set_value(
                "created",
                task.created
                    .map(|x| x.and_time(MIDNIGHT).and_utc().timestamp().to_string()),
                &mut ops,
            )?;
            tc_task.set_value(
                "end",
                task.done
                    .map(|x| x.and_time(MIDNIGHT).and_utc().timestamp().to_string()),
                &mut ops,
            )?;
            tc_task.set_value(
                "end",
                task.canceled
                    .map(|x| x.and_time(MIDNIGHT).and_utc().timestamp().to_string()),
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

            self.replica.commit_operations(ops).context("Failed to commit operations")?;

            return Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use taskchampion::{Operations, Status, Task, Uuid};

    use crate::taskparser::Priority;
    use crate::testutil::{self, TaskBuilder, TestContext, create_mem_replica};
    use std::path::Path;

    use super::*;

    #[test]
    fn test_update_due_date() {
        let mut replica = create_mem_replica();
        let mut context = TestContext::new(&mut replica);
        let mut tc_task = TaskBuilder::new(&mut context)
            .desc("Test task")
            .status(taskchampion::Status::Pending)
            .build();

        let mut ts = TaskWarriorSync::from_replica(replica);

        let obs_task = ObsidianTaskBuilder::new()
            .uuid(tc_task.get_uuid())
            .description("Test task")
            .due("2025-05-28")
            .project("My project")
            .priority(Priority::Normal)
            .build();

        let result = ts.md_to_tc(&obs_task, "").unwrap();
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

        let mut ts = TaskWarriorSync::from_replica(replica);

        let result = ts.md_to_tc(&mut task, "").unwrap();
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

        let mut ts = TaskWarriorSync::from_replica(replica);

        let result = ts.md_to_tc(&mut task, "").unwrap();
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

        let mut ts = TaskWarriorSync::from_replica(replica);

        let result = ts.md_to_tc(&mut task, "").unwrap();
        assert!(!result);

        tc_task = ts.replica.get_task(tc_task.get_uuid()).unwrap().unwrap();
        assert_eq!(tc_task.get_priority(), "H");
        assert!(tc_task.get_value("tag_next").is_none());
        assert_eq!(task, tc_task);
    }

    #[test]
    fn test_new_task() {
        let replica = create_mem_replica();
        let task = ObsidianTaskBuilder::new()
            .description("Test")
            .priority(Priority::High)
            .build();

        let mut ts = TaskWarriorSync::from_replica(replica);
        let result = ts.md_to_tc(&task, "").unwrap();
        assert!(result);

        assert_eq!(ts.replica.all_task_uuids().unwrap().len(), 1);
        let uuid = ts.replica.all_task_uuids().unwrap().pop().unwrap();
        let reference_task = ObsidianTask {
            uuid: Some(uuid.clone()),
            ..task
        };
        let result_task = ts.replica.get_task(uuid).unwrap().unwrap();
        assert_eq!(reference_task, result_task);
    }
}
