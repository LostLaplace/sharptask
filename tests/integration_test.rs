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

        let storage = taskchampion::StorageConfig::OnDisk {
            taskdb_dir: path.join("taskData"),
            create_if_missing: true,
            access_mode: AccessMode::ReadWrite,
        }
        .into_storage()
        .unwrap();
        let mut replica = Replica::new(storage);

        sharptask.args([
            "--task-db",
            path.join("taskData").to_str().unwrap(),
            "--file",
            path.join("simple.md").to_str().unwrap(),
            "md-to-tc",
        ]);

        let mut handle = sharptask.spawn().unwrap();
        assert!(handle.wait().unwrap().success());

        assert_eq!(replica.all_task_uuids().unwrap().len(), 2);
        let mut tasks = replica.all_tasks().unwrap();
        let mut tasks_iterator = tasks.values_mut();
        println!("{:?}", tasks_iterator);
        let task1 = tasks_iterator
            .find(|x| x.get_description().contains("Unsynced task"))
            .unwrap();
        let task2 = tasks_iterator
            .find(|x| {
                println!("test: {}", x.get_description());
                x.get_description().contains("Completed task")
            })
            .unwrap();
        assert_eq!(task1.get_description(), "Unsynced task");
        assert_eq!(task1.get_status(), Status::Pending);
        assert_eq!(task2.get_description(), "Completed task");
        assert_eq!(task2.get_status(), Status::Completed);
        println!("Task2: {:?}", task2);

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

        sharptask.args([
            "--task-db",
            path.join("taskData").to_str().unwrap(),
            "--vault",
            path.join("vault").to_str().unwrap(),
            "md-to-tc",
        ]);

        let mut handle = sharptask.spawn().unwrap();
        assert!(handle.wait().unwrap().success());
    }
}
