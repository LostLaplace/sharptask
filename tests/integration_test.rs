#[allow(unused_must_use)]
#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use taskchampion::storage::AccessMode;
    use taskchampion::{self, Operations, Replica, Status};
    use test_bin;
    use test_bin::get_test_bin;
    use testdir;

    #[test]
    fn test_simple_md() {
        let mut sharptask = get_test_bin("sharptask");

        let simple_md = PathBuf::from("tests/simple.md");
        assert!(simple_md.exists());
        let path = testdir::testdir!();
        println!("test path: {:?}", path);
        fs::copy(simple_md, path.join("simple.md"));

        let empty_tn = path.join("empty_tasknotes");
        fs::create_dir(&empty_tn).unwrap();

        sharptask.args([
            "--task-db",
            path.join("taskData").to_str().unwrap(),
            "--file",
            path.join("simple.md").to_str().unwrap(),
            "--tasknotes",
            empty_tn.to_str().unwrap(),
            "md-to-tc",
        ]);

        let storage = taskchampion::StorageConfig::OnDisk {
            taskdb_dir: path.join("taskData"),
            create_if_missing: true,
            access_mode: AccessMode::ReadWrite,
        }
        .into_storage()
        .unwrap();

        let mut handle = sharptask.spawn().unwrap();
        assert!(handle.wait().unwrap().success());

        let mut replica = Replica::new(storage);
        assert_eq!(replica.all_task_uuids().unwrap().len(), 2);
        let mut tasks = replica.all_tasks().unwrap();
        let mut tasks_values = tasks.values_mut();
        let mut task1_opt = None;
        let mut task2_opt = None;
        for task in tasks_values {
            let desc = task.get_description();
            if desc.contains("Unsynced task") {
                task1_opt = Some(task);
            } else if desc.contains("Completed task") {
                task2_opt = Some(task);
            }
        }
        let task1 = task1_opt.unwrap();
        let task2 = task2_opt.unwrap();
        assert_eq!(task1.get_description(), "Unsynced task");
        assert_eq!(task1.get_status(), Status::Pending);
        assert_eq!(task2.get_description(), "Completed task");
        assert_eq!(task2.get_status(), Status::Completed);

        let contents = fs::read_to_string(path.join("simple.md")).unwrap();
        assert!(contents.contains("[[uuid: "));

        let mut ops = Operations::new();
        task1.set_status(Status::Completed, &mut ops);
        task1.set_value(
            "end",
            Some(
                chrono::DateTime::parse_from_rfc3339("2025-06-08T00:00:00Z")
                    .unwrap()
                    .timestamp()
                    .to_string(),
            ),
            &mut ops,
        );
        replica.commit_operations(ops);

        let mut tc_to_md = get_test_bin("sharptask");
        tc_to_md.args([
            "--task-db",
            path.join("taskData").to_str().unwrap(),
            "--file",
            path.join("simple.md").to_str().unwrap(),
            "--tasknotes",
            empty_tn.to_str().unwrap(),
            "tc-to-md",
        ]);

        let mut handle2 = tc_to_md.spawn().unwrap();
        assert!(handle2.wait().unwrap().success());

        let contents2 = fs::read_to_string(path.join("simple.md")).unwrap();
        assert!(contents2.contains("- [x] Unsynced task ✅ 2025-06-08 [[uuid:"));
    }

    #[test]
    fn test_vault() {
        let mut sharptask = get_test_bin("sharptask");

        let vault = PathBuf::from("tests/vault");
        assert!(vault.exists());
        let path = testdir::testdir!();
        println!("vault test path: {:?}", path);
        fs::create_dir(path.join("vault"));
        fs::copy(vault.join("a.md"), path.join("vault/a.md"));
        fs::copy(vault.join("b.md"), path.join("vault/b.md"));

        let storage = taskchampion::StorageConfig::OnDisk {
            taskdb_dir: path.join("taskData"),
            create_if_missing: true,
            access_mode: AccessMode::ReadWrite,
        }
        .into_storage()
        .unwrap();
        let replica = Replica::new(storage);

        let empty_tn = path.join("empty_tasknotes");
        fs::create_dir(&empty_tn).unwrap();

        sharptask.args([
            "--task-db",
            path.join("taskData").to_str().unwrap(),
            "--vault",
            path.join("vault").to_str().unwrap(),
            "--tasknotes",
            empty_tn.to_str().unwrap(),
            "md-to-tc",
        ]);

        let mut handle = sharptask.spawn().unwrap();
        assert!(handle.wait().unwrap().success());
    }

    #[test]
    fn test_tasknotes_md_to_tc() {
        let path = testdir::testdir!();
        println!("tasknotes test path: {:?}", path);

        let tn_dir = path.join("tasknotes");
        fs::create_dir(&tn_dir).unwrap();
        fs::copy(
            PathBuf::from("tests/tasknotes/pending-task.md"),
            tn_dir.join("pending-task.md"),
        )
        .unwrap();
        fs::copy(
            PathBuf::from("tests/tasknotes/completed-task.md"),
            tn_dir.join("completed-task.md"),
        )
        .unwrap();

        // Use an empty vault to avoid picking up tasks from the user's real vault
        // via ~/.sharptask/config.toml
        let empty_vault = path.join("empty_vault");
        fs::create_dir(&empty_vault).unwrap();

        let storage = taskchampion::StorageConfig::OnDisk {
            taskdb_dir: path.join("taskData"),
            create_if_missing: true,
            access_mode: AccessMode::ReadWrite,
        }
        .into_storage()
        .unwrap();

        let mut sharptask = get_test_bin("sharptask");
        sharptask.args([
            "--task-db",
            path.join("taskData").to_str().unwrap(),
            "--vault",
            empty_vault.to_str().unwrap(),
            "--tasknotes",
            tn_dir.to_str().unwrap(),
            "md-to-tc",
        ]);

        let mut handle = sharptask.spawn().unwrap();
        assert!(handle.wait().unwrap().success());

        // Both tasks should now be in TC
        let mut replica = Replica::new(storage);
        assert_eq!(replica.all_task_uuids().unwrap().len(), 2);

        let tasks = replica.all_tasks().unwrap();
        let pending = tasks
            .values()
            .find(|t| t.get_description() == "Unsynced tasknote")
            .expect("Pending task not found in TC");
        assert_eq!(pending.get_status(), Status::Pending);
        assert_eq!(pending.get_priority(), "H");

        let completed = tasks
            .values()
            .find(|t| t.get_description() == "Completed tasknote")
            .expect("Completed task not found in TC");
        assert_eq!(completed.get_status(), Status::Completed);

        // The pending task file should now have tc_uuid written back
        let pending_md = fs::read_to_string(tn_dir.join("pending-task.md")).unwrap();
        assert!(
            pending_md.contains("tc_uuid:"),
            "tc_uuid should have been written to pending-task.md"
        );

        // ── tc-to-md: update TC, verify write-back ────────────────────────────
        let pending_uuid = pending.get_uuid();
        drop(tasks); // release borrow on replica
        let mut pending_mutable = replica.get_task(pending_uuid).unwrap().unwrap();
        let mut ops = Operations::new();
        pending_mutable
            .set_status(Status::Completed, &mut ops)
            .expect("set_status");
        pending_mutable
            .set_value(
                "end",
                Some(
                    chrono::DateTime::parse_from_rfc3339("2025-07-01T00:00:00Z")
                        .unwrap()
                        .timestamp()
                        .to_string(),
                ),
                &mut ops,
            )
            .expect("set end");
        // Simulate tasksh review setting the reviewed UDA
        pending_mutable
            .set_value(
                "reviewed",
                Some(
                    chrono::DateTime::parse_from_rfc3339("2025-07-02T00:00:00Z")
                        .unwrap()
                        .timestamp()
                        .to_string(),
                ),
                &mut ops,
            )
            .expect("set reviewed");
        replica.commit_operations(ops).unwrap();

        let mut tc_to_md = get_test_bin("sharptask");
        tc_to_md.args([
            "--task-db",
            path.join("taskData").to_str().unwrap(),
            "--vault",
            empty_vault.to_str().unwrap(),
            "--tasknotes",
            tn_dir.to_str().unwrap(),
            "tc-to-md",
        ]);
        let mut handle2 = tc_to_md.spawn().unwrap();
        assert!(handle2.wait().unwrap().success());

        let updated_md = fs::read_to_string(tn_dir.join("pending-task.md")).unwrap();
        assert!(
            updated_md.contains("status: done"),
            "status should be 'done' after tc-to-md"
        );
        assert!(
            updated_md.contains("completed: 2025-07-01"),
            "completed date should be written back"
        );
        assert!(
            updated_md.contains("reviewed: 2025-07-02"),
            "reviewed date from tasksh should be written back to the frontmatter"
        );
        // Body text must be preserved
        assert!(
            updated_md.contains("This task has not yet been synced"),
            "note body should be preserved"
        );
    }
}
