use taskchampion::{Uuid, Operations, Task, Replica, Status, StorageConfig};

const MIDNIGHT: chrono::NaiveTime = chrono::NaiveTime::from_hms(0, 0, 0);

pub struct TestContext<'b> {
    pub replica: &'b mut Replica,
    pub ops: Operations,
}

pub struct TaskBuilder<'a, 'b> {
    context: &'a mut TestContext<'b>,
    task: Task,
}

macro_rules! tb_date_fn {
    ($tcData:tt) => {
        pub fn $tcData<T: AsRef<str>>(mut self, date: T) -> Self {
            let dt = chrono::NaiveDate::parse_from_str(date.as_ref(), "%Y-%m-%d")
                .unwrap()
                .and_time(MIDNIGHT)
                .and_utc();

            let _ = self.task.set_value(
                stringify!($tcData),
                Some(dt.timestamp().to_string()),
                &mut self.context.ops,
            );
            self
        }
    };
}

impl<'a, 'b> TaskBuilder<'a, 'b> {
    pub fn new(context: &'a mut TestContext<'b>) -> TaskBuilder<'a, 'b> {
        let uuid = Uuid::new_v4();
        let _ = context.replica.create_task(uuid, &mut context.ops);
        context
            .replica
            .commit_operations(context.ops.clone())
            .unwrap();
        context.ops = Operations::new();
        TaskBuilder {
            task: context.replica.get_task(uuid).unwrap().unwrap(),
            context,
        }
    }

    pub fn desc<T: Into<String>>(mut self, description: T) -> Self {
        let _ = self
            .task
            .set_description(description.into(), &mut self.context.ops);
        self
    }

    pub fn status(mut self, status: Status) -> Self {
        let _ = self.task.set_status(status, &mut self.context.ops);

        self
    }

    tb_date_fn!(due);
    tb_date_fn!(scheduled);
    tb_date_fn!(wait);
    tb_date_fn!(created);
    tb_date_fn!(end);

    pub fn priority<T: Into<String>>(mut self, priority: T) -> Self {
        let _ = self
            .task
            .set_priority(priority.into(), &mut self.context.ops);
        self
    }

    pub fn project<T: Into<String>>(mut self, project: T) -> Self {
        let _ = self
            .task
            .set_value("project", Some(project.into()), &mut self.context.ops);
        self
    }

    pub fn tags(mut self, tags: &[&str]) -> Self {
        for tag in tags {
            let tag_str = format!("tag_{tag}");
            let _ = self
                .task
                .set_value(tag_str, Some("".to_string()), &mut self.context.ops);
        }
        self
    }

    pub fn build(self) -> Task {
        let _ = self
            .context
            .replica
            .commit_operations(self.context.ops.clone());
        self.task
    }
}

impl TestContext<'_> {
    pub fn new(replica: &'_ mut Replica) -> TestContext {
        TestContext {
            replica,
            ops: Operations::new(),
        }
    }
}

pub fn create_mem_replica() -> Replica {
    Replica::new(StorageConfig::InMemory.into_storage().unwrap())
}
