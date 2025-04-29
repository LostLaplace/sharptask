use anyhow::{Context, Result};
use std::path::PathBuf;
use taskchampion::storage::AccessMode;
use taskchampion::{Replica, StorageConfig};

use crate::taskparser::{self, ObsidianTask};

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
    pub fn md_to_tc(&mut self, task: &mut ObsidianTask) -> Result<bool> {
        // 1. If task has UUID, find it in TC DB
        let mut ops = taskchampion::Operations::new();

        if let Some(uuid) = task.uuid {
            if let Some(mut tc_task) = self.replica.get_task(uuid).ok().flatten() {
                // Status update
                if task.status != tc_task.get_status() {
                    tc_task.set_status(task.status.clone().into(), &mut ops)?;
                }

                // Description update
                if task.description != tc_task.get_description() {
                    tc_task.set_description(tc_task.get_description().to_string(), &mut ops)?;
                }

                // Due date update
                println!("Due date processing");
                let task_due_naive = task
                    .due
                    .map(|due| due.with_timezone(&chrono_tz::UTC).date_naive());
                let tc_task_due_naive = tc_task.get_due().map(|due| due.date_naive());

                if task_due_naive != tc_task_due_naive {
                    println!("Due dates not equal");
                    println!("task_due_naive: {:?}, tc_task_due_naive: {:?}", task_due_naive, tc_task_due_naive);
                    tc_task.set_due(task.due.map(|date| date.to_utc()), &mut ops)?;
                }

                // Wait date update
                let task_wait_naive = task
                    .start
                    .map(|wait| wait.with_timezone(&chrono_tz::UTC).date_naive());
                let tc_task_wait_naive = tc_task.get_wait().map(|wait| wait.date_naive());

                if task_wait_naive != tc_task_wait_naive {
                    tc_task.set_wait(task.start.map(|date| date.to_utc()), &mut ops)?;
                }

                // Update priority
                if task.status != tc_task.get_status() {
                    tc_task.set_status(task.status.clone().into(), &mut ops)?;
                }

                let mut task_data = tc_task.into_task_data();
                // Update tags
                let tc_tags: Vec<String> = task_data.iter().filter_map(|item| {
                    if item.0.starts_with("tag_") {
                        return Some(item.0.clone())
                    }
                    None
                }).collect();

                let mut update_tags = false;
                if task.tags.len() != tc_tags.len() {
                    update_tags = true;
                } else {
                    for tag in &task.tags {
                        let tag_string = format!("tag_{tag}");
                        if tc_tags.contains(&tag_string) {
                            continue;
                        } else {
                            update_tags = true;
                            break;
                        }
                    }
                }
                if update_tags {
                    // Clear out existing tags
                    for tag in &tc_tags {
                        let tag_string = format!("tag_{tag}");
                        task_data.update(tag_string, None, &mut ops);
                    }

                    // Add new tags
                    for tag in &task.tags {
                        let tag_string = format!("tag_{tag}");
                        task_data.update(tag_string, Some(String::new()), &mut ops);
                    }
                }
                
                // Update end date
                let end_date = task_data.get("end").map(|ts| chrono::DateTime::from_timestamp(ts.parse::<i64>().unwrap(), 0)).flatten();
                let complete_date = task.done.map(|dt| dt.to_utc());
                let canceled_date = task.canceled.map(|dt| dt.to_utc());

                if task.status == taskparser::Status::Complete && complete_date != end_date {
                    task_data.update("end", complete_date.map(|ed| ed.timestamp().to_string()), &mut ops);
                }

                if task.status == taskparser::Status::Canceled && canceled_date != end_date {
                    task_data.update("end", canceled_date.map(|ed| ed.timestamp().to_string()), &mut ops);
                }

                // Update scheduled
                let tc_scheduled_date = task_data.get("scheduled").map(|ts| chrono::DateTime::from_timestamp(ts.parse::<i64>().unwrap(), 0)).flatten();
                let scheduled_date = task.scheduled.map(|dt| dt.to_utc());

                if scheduled_date != tc_scheduled_date {
                    task_data.update("scheduled", scheduled_date.map(|ed| ed.timestamp().to_string()), &mut ops);
                }
                
                // Update priority
                // Normal priority results in no special item in the task data
                let tc_priority = task_data.get("priority").unwrap_or("");
                if task.priority.to_string() != tc_priority {
                    let pri = match task.priority {
                        taskparser::Priority::Normal => None,
                        _ => Some(task.priority.to_string())
                    };
                    task_data.update("priority", pri, &mut ops);
                }
                // If highest priority, also set the +next tag
                if task.priority == taskparser::Priority::Highest {
                    task_data.update("tag_next", Some(String::from("")), &mut ops);
                }

                // Update project
                let tc_project = task_data.get("project").map(|prj| prj.to_string());
                if task.project != tc_project {
                    task_data.update("project", task.project.clone(), &mut ops);
                }
            }

            if ops.is_empty() {
                return Ok(false);
            }
            return self.replica
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
    use taskchampion::Uuid;

    use super::*;

    const TZ: chrono_tz::Tz = America::Chicago;

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
        let mut task = ObsidianTask {
            uuid: Some(uuid.clone()),
            description: "Test task".to_string(),
            status: taskparser::Status::Pending,
            due: Some(due_date.with_timezone(&chrono_tz::America::Chicago)),
            project: Some(String::from("My project")),
            priority: taskparser::Priority::Normal,
            ..Default::default()
        };

        let result = ts.md_to_tc(&mut task).unwrap();
        assert!(!result);

        let mut rep2 = ts.get_replica();
        let task2 = rep2.get_task(uuid.clone()).unwrap().unwrap().into_task_data();
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
