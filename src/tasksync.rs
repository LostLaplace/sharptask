use anyhow::{Context, Result};
use std::path::PathBuf;
use taskchampion::storage::AccessMode;
use taskchampion::{Replica, StorageConfig};

use crate::taskparser::ObsidianTask;

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
                let task_due_naive = task
                    .due
                    .map(|due| due.with_timezone(&chrono_tz::UTC).date_naive());
                let tc_task_due_naive = tc_task.get_due().map(|due| due.date_naive());

                if task_due_naive != tc_task_due_naive {
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

                // Update tags
                
                // Deal with more annoying dates, creation and deletion
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

    use chrono_tz::America;
    use taskchampion::Uuid;

    use super::*;

    const TZ: chrono_tz::Tz = America::Chicago;

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
