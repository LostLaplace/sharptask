#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::process::ExitStatus;
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

        assert_eq!(replica.all_task_uuids().unwrap().len(), 1);
        let mut task = replica.all_tasks().unwrap().into_iter().next().unwrap().1;
        assert_eq!(task.get_description(), "Unsynced task");
        assert_eq!(task.get_status(), Status::Pending);

        let contents = fs::read_to_string(path.join("simple.md")).unwrap();
        assert!(contents.contains("[[uuid: "));

        let mut ops = Operations::new();
        task.set_status(Status::Completed, &mut ops);
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
        assert!(contents2.contains("- [x] Unsynced task ✅ 2025-06-07 [[uuid:"));
    }
}
