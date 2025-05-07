use anyhow::{Context, Result};
use chrono::NaiveTime;
use std::path::PathBuf;
use taskchampion::storage::AccessMode;
use taskchampion::{Replica, StorageConfig};

use crate::taskparser::{self, ObsidianTask, ObsidianTaskBuilder};

pub struct TaskWarriorSync {
    replica: Replica,
    timezone: chrono_tz::Tz,
}

impl TaskWarriorSync {
    pub fn new(path: &PathBuf, timezone: chrono_tz::Tz) -> Result<Self> {
        let storage = StorageConfig::OnDisk {
            taskdb_dir: path.clone(),
            create_if_missing: false,
            access_mode: AccessMode::ReadWrite,
        }
        .into_storage()
        .context("Failed to build storage context")?;
        Ok(TaskWarriorSync {
            replica: Replica::new(storage),
            timezone,
        })
    }

    #[cfg(test)]
    fn from_replica(replica: Replica, timezone: chrono_tz::Tz) -> Self {
        TaskWarriorSync { replica, timezone }
    }

    #[cfg(test)]
    fn get_replica(&mut self) -> &mut Replica {
        &mut self.replica
    }

    // Updates any taskchampion copies of the task to match the markdown representation
    // Returns true if the markdown should be updated, false if no further changes needed
    pub fn md_to_tc(&mut self, task: &ObsidianTask) -> Result<bool> {
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
                    tc_task.set_due(task.due.map(|date| date.and_time(MIDNIGHT).and_utc()), &mut ops)?;
                }

                // Wait date update
                if !task.compare_start(&tc_task) {
                    tc_task.set_wait(task.start.map(|date| date.and_time(MIDNIGHT).and_utc()), &mut ops)?;
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
                        task.done.map(|ed| ed.and_time(MIDNIGHT).and_utc().timestamp().to_string()),
                        &mut ops,
                    )?;
                }

                if task.status == taskparser::Status::Canceled && !task.compare_canceled(&tc_task) {
                    tc_task.set_value(
                        "end",
                        task.canceled.map(|ed| ed.and_time(MIDNIGHT).and_utc().timestamp().to_string()),
                        &mut ops,
                    )?;
                }

                // Update scheduled
                if !task.compare_schedule(&tc_task) {
                    tc_task.set_value(
                        "scheduled",
                        task.scheduled.map(|ed| ed.and_time(MIDNIGHT).and_utc().timestamp().to_string()),
                        &mut ops,
                    )?;
                }

                // Update priority
                // Normal priority results in no special item in the task data
                if !task.compare_priority(&tc_task) {
                    let pri = match task.priority {
                        taskparser::Priority::Normal => None,
                        _ => Some(task.priority.to_string()),
                    };
                    tc_task.set_value("priority", pri, &mut ops)?;

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

            return Ok(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use chrono::TimeZone;
    use chrono_tz::America;
    use taskchampion::{Operations, Status, Task, Uuid};

    use crate::taskparser::Priority;

    use super::*;

    const TZ: chrono_tz::Tz = America::Chicago;
    const TODAY: i64 = 1746248400;
    const MIDNIGHT: chrono::NaiveTime = chrono::NaiveTime::from_hms(0, 0, 0);

    struct TestContext {
        pub replica: Replica,
        pub ops: Operations,
    }

    struct TaskBuilder<'a> {
        context: &'a mut TestContext,
        task: Task,
    }

    macro_rules! tb_date_fn {
        ($tcData:tt) => {
            fn $tcData<T: AsRef<str>>(mut self, date: T) -> Self {
                let dt = chrono::NaiveDate::parse_from_str(date.as_ref(), "%Y-%m-%d").unwrap().and_time(MIDNIGHT).and_utc();

                let _ = self.task.set_value("$tcData", Some(dt.timestamp().to_string()), &mut self.context.ops);
                self
            }
        };
    }

    impl TaskBuilder<'_> {
        fn new(context: &mut TestContext) -> TaskBuilder {
            let uuid = Uuid::new_v4();
            let _ = context.replica.create_task(uuid, &mut context.ops);
            context.replica.commit_operations(context.ops.clone()).unwrap();
            context.ops = Operations::new();
            TaskBuilder {
                task: context.replica.get_task(uuid).unwrap().unwrap(),
                context,
            }
        }

        fn desc<T: Into<String>>(mut self, description: T) -> Self {
            let _ = self
                .task
                .set_description(description.into(), &mut self.context.ops);
            self
        }

        fn status(mut self, status: Status) -> Self {
            let _ = self.task.set_status(status, &mut self.context.ops);

            self
        }

        tb_date_fn!(due);
        tb_date_fn!(scheduled);
        tb_date_fn!(wait);
        tb_date_fn!(created);
        tb_date_fn!(end);

        fn priority<T: Into<String>>(mut self, priority: T) -> Self {
            let _ = self.task.set_priority(priority.into(), &mut self.context.ops);
            self
        }

        fn project<T: Into<String>>(mut self, project: T) -> Self {
            let _ = self.task.set_value("project", Some(project.into()), &mut self.context.ops);
            self
        }

        fn tags(mut self, tags: &[&str]) -> Self {
            for tag in tags {
                let tag_str = format!("tag_{tag}");
                let _ = self.task.set_value(tag_str, Some("".to_string()), &mut self.context.ops);
            }
            self
        }

        fn build(self) -> Task {
            let _ = self.context.replica.commit_operations(self.context.ops.clone());
            self.task
        }
    }

    impl TestContext {
        fn new() -> TestContext {
            TestContext {
                replica: Replica::new(StorageConfig::InMemory.into_storage().unwrap()),
                ops: Operations::new(),
            }
        }
    }

    #[test]
    fn test_basic() {
        let mut ctx = TestContext::new();
        let tb = TaskBuilder::new(&mut ctx)
            .desc("Test")
            .status(Status::Pending)
            .build();

        let mut ot = ObsidianTaskBuilder::new()
            .uuid(tb.get_uuid()) 
            .description("Test")
            .status(taskparser::Status::Pending)
            .due("2025-05-03")
            .build();

        let mut ts = TaskWarriorSync::from_replica(ctx.replica, TZ);
        assert!(!ts.md_to_tc(&mut ot).unwrap());
        let updated_task = ts.get_replica().get_task(tb.get_uuid()).unwrap().unwrap();
        assert_eq!(ot, updated_task);
    }

    #[test]
    fn test_update_due_date() {
        let mut replica = Replica::new(StorageConfig::InMemory.into_storage().unwrap());
        let mut ops = taskchampion::Operations::new();
        let uuid = Uuid::from_str("96bb3816-aedd-4033-8ff6-4746a700aac8").unwrap();
        let mut task = replica.create_task(uuid.clone(), &mut ops).unwrap();
        let _ = task.set_description("Test task".to_string(), &mut ops);
        let _ = task.set_status(taskchampion::Status::Pending, &mut ops);
        let _ = replica.commit_operations(ops);
        let mut ts = TaskWarriorSync::from_replica(replica, TZ);

        let due_date = chrono::Utc.with_ymd_and_hms(2025, 5, 28, 0, 0, 0).unwrap();
        let mut task = ObsidianTaskBuilder::new()
            .uuid(uuid.clone())
            .description("Test task")
            .status(taskparser::Status::Pending)
            .due("2025-05-28")
            .project("My project")
            .priority(Priority::Normal)
            .build();

        let result = ts.md_to_tc(&mut task).unwrap();
        assert!(!result);

        let mut rep2 = ts.get_replica();
        let task2 = rep2
            .get_task(uuid.clone())
            .unwrap()
            .unwrap()
            .into_task_data();
        let next_tag = task2.get("tag_next");
        assert!(next_tag.is_none());
        assert_eq!(task2.get("priority"), None);
    }

    //TODO: surely there's an easier way?
    #[test]
    fn test_create_task() {
        let mut replica = Replica::new(StorageConfig::InMemory.into_storage().unwrap());
        let mut ops = taskchampion::Operations::new();
        let uuid = Uuid::from_str("96bb3816-aedd-4033-8ff6-4746a700aac8").unwrap();
        let mut task = replica.create_task(uuid.clone(), &mut ops).unwrap();
        let _ = task.set_description("Test task".to_string(), &mut ops);
        let _ = task.set_status(taskchampion::Status::Pending, &mut ops);
        let _ = replica.commit_operations(ops);
        let mut ts = TaskWarriorSync::from_replica(replica, TZ);

        let mut task = ObsidianTask {
            uuid: Some(uuid.clone()),
            description: "Test task".to_string(),
            status: crate::taskparser::Status::Complete,
            ..Default::default()
        };
        let result = ts.md_to_tc(&mut task).unwrap();
        assert!(!result);

        let mut rep2 = ts.get_replica();
        let task2 = rep2.get_task(uuid.clone()).unwrap().unwrap();
        assert_eq!(task2.get_status(), taskchampion::Status::Completed);
    }
}
