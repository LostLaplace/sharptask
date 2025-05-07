use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, offset::LocalResult};
use regex::Regex;
use std::fmt;
use std::iter::Peekable;
use std::{str::FromStr, string::String};
use taskchampion::{Task, Uuid};
use unicode_segmentation::{Graphemes, UnicodeSegmentation};

#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Status {
    Pending,
    Complete,
    Canceled,
}

impl PartialEq<taskchampion::Status> for Status {
    fn eq(&self, other: &taskchampion::Status) -> bool {
        return (*self == Status::Pending && *other == taskchampion::Status::Pending)
            || (*self == Status::Complete && *other == taskchampion::Status::Completed)
            || (*self == Status::Canceled && *other == taskchampion::Status::Deleted);
    }
}

impl PartialEq<Status> for taskchampion::Status {
    fn eq(&self, other: &Status) -> bool {
        return (*other == Status::Pending && *self == taskchampion::Status::Pending)
            || (*other == Status::Complete && *self == taskchampion::Status::Completed)
            || (*other == Status::Canceled && *self == taskchampion::Status::Deleted);
    }
}

impl From<Status> for taskchampion::Status {
    fn from(value: Status) -> Self {
        match value {
            Status::Pending => return taskchampion::Status::Pending,
            Status::Complete => return taskchampion::Status::Completed,
            Status::Canceled => return taskchampion::Status::Deleted,
        }
    }
}

impl Default for Status {
    fn default() -> Self {
        Status::Pending
    }
}

#[derive(Debug, PartialEq)]
pub enum Priority {
    Lowest,
    Low,
    Normal,
    Medium,
    High,
    Highest,
}

impl Default for Priority {
    fn default() -> Self {
        Priority::Normal
    }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lowest | Self::Low => return write!(f, "L"),
            Self::Normal => return write!(f, ""),
            Self::Medium => return write!(f, "M"),
            Self::High | Self::Highest => return write!(f, "H"),
        }
    }
}

const SIGNIFICANT_EMOJI: &[&str] = &[
    &"📅", &"⏳", &"🛫", &"➕", &"✅", &"❌", &"🔺", &"⏫", &"🔼", &"🔽", &"⏬", &"🔁", &"🆔",
    &"⛔", &"🔨",
];

#[derive(Default, Debug, PartialEq)]
pub struct ObsidianTask {
    pub uuid: Option<Uuid>,
    pub status: Status,
    pub description: String,
    pub tags: Vec<String>,
    pub due: Option<NaiveDate>,
    pub scheduled: Option<NaiveDate>,
    pub start: Option<NaiveDate>,
    pub created: Option<NaiveDate>,
    pub done: Option<NaiveDate>,
    pub canceled: Option<NaiveDate>,
    pub priority: Priority,
    pub project: Option<String>,
}

const MIDNIGHT: chrono::NaiveTime = chrono::NaiveTime::from_hms(0, 0, 0);

macro_rules! compare_date_fn {
    ($name:ident, $taskParam:tt, $tcData:tt) => {
        pub fn $name(&self, other: &taskchampion::Task) -> bool {
            let task_date = self.$taskParam.map(|ts| ts.and_time(MIDNIGHT).and_utc().timestamp());
            let tc_date = other
                .get_value($tcData)
                .map(|ts| ts.parse::<i64>().ok())
                .flatten();
            task_date == tc_date
        }
    };
}
//TODO: write helper functions for each field and then implement PartialEq
impl ObsidianTask {
    compare_date_fn!(compare_due, due, "due");
    compare_date_fn!(compare_schedule, scheduled, "scheduled");
    compare_date_fn!(compare_start, start, "wait");
    compare_date_fn!(compare_created, created, "created");
    compare_date_fn!(compare_done, done, "end");
    compare_date_fn!(compare_canceled, canceled, "end");

    pub fn compare_uuid(&self, other: &taskchampion::Task) -> bool {
        match self.uuid {
            Some(uuid) => return uuid == other.get_uuid(),
            None => return false,
        };
    }

    pub fn compare_status(&self, other: &taskchampion::Task) -> bool {
        self.status == other.get_status()
    }

    pub fn compare_description(&self, other: &taskchampion::Task) -> bool {
        self.description == other.get_description()
    }

    pub fn compare_tags(&self, other: &taskchampion::Task) -> bool {
        let tc_tags: Vec<String> = other
            .get_tags()
            .filter(|itm| itm.is_user())
            .map(|itm| itm.to_string())
            .collect();
        if self.tags.len() != tc_tags.len() {
            return false;
        }

        for tag in &self.tags {
            if !tc_tags.contains(&tag) {
                return false;
            }
        }

        return true;
    }

    pub fn compare_priority(&self, other: &taskchampion::Task) -> bool {
        if self.priority == Priority::Highest {
            return other
                .get_tags()
                .filter(|itm| itm.is_user())
                .map(|itm| itm.to_string())
                .collect::<Vec<String>>()
                .contains(&String::from("next"));
        }
        let tc_priority = other.get_value("priority").unwrap_or("");
        self.priority.to_string() == tc_priority
    }

    pub fn compare_project(&self, other: &taskchampion::Task) -> bool {
        let tc_project = other.get_value("project");
        self.project == tc_project.map(|prj| prj.to_string())
    }
}

pub struct ObsidianTaskBuilder {
    task: ObsidianTask
}

macro_rules! set_date_fn {
    ($taskMember:tt) => {
        pub fn $taskMember<T: AsRef<str>>(mut self, date: T) -> Self {
            let dt = chrono::NaiveDate::parse_from_str(date.as_ref(), "%Y-%m-%d").unwrap();
            self.task.$taskMember = Some(dt);
            self
        }
    };
}

impl ObsidianTaskBuilder {
    pub fn new() -> ObsidianTaskBuilder {
        let task = ObsidianTask::default();
        ObsidianTaskBuilder {
           task 
        }
    }

    pub fn uuid(mut self, uuid: Uuid) -> Self {
        self.task.uuid = Some(uuid);
        self
    }

    pub fn status(mut self, status: Status) -> Self {
        self.task.status = status;
        self
    }

    pub fn description<T: Into<String>>(mut self, desc: T) -> Self {
        self.task.description = desc.into();
        self
    }

    pub fn tags(mut self, tags: &[&str]) -> Self {
        let tag_strings = tags.iter().map(|itm| itm.to_string());
        let tag_vec: Vec<String> = Vec::from_iter(tag_strings);
        self.task.tags = tag_vec;
        self
    }

    pub fn priority(mut self, priority: Priority) -> Self {
        self.task.priority = priority;
        self
    }

    pub fn project<T: Into<String>>(mut self, project: T) -> Self {
        self.task.project = Some(project.into());
        self
    }

    set_date_fn!(due);
    set_date_fn!(scheduled);
    set_date_fn!(start);
    set_date_fn!(created);
    set_date_fn!(done);
    set_date_fn!(canceled);

    pub fn build(self) -> ObsidianTask {
        self.task
    }
}

impl PartialEq<Task> for ObsidianTask {
    fn eq(&self, other: &Task) -> bool {
        self.compare_due(other)
            && self.compare_schedule(other)
            && self.compare_start(other)
            && self.compare_created(other)
            && self.compare_done(other)
            && self.compare_canceled(other)
            && self.compare_uuid(other)
            && self.compare_status(other)
            && self.compare_description(other)
            && self.compare_tags(other)
            && self.compare_priority(other)
            && self.compare_project(other)
    }
}

#[derive(Debug, PartialEq)]
enum ObsidianMetadata {
    Due(NaiveDate),
    Scheduled(NaiveDate),
    Start(NaiveDate),
    Created(NaiveDate),
    Done(NaiveDate),
    Canceled(NaiveDate),
    Priority(Priority),
    Project(String),
}

struct MetadataParser<'a> {
    metadata: Peekable<Graphemes<'a>>,
}

impl MetadataParser<'_> {
    fn new(input: &'_ String) -> MetadataParser<'_> {
        MetadataParser {
            metadata: input.graphemes(true).peekable(),
        }
    }
}

macro_rules! process_date {
    ($parser:ident, $variant:path) => {
        let mut date: String = $parser.metadata.by_ref().take(11).collect();
        date = date.trim().to_string();
        let nd = NaiveDate::parse_from_str(&date, "%Y-%m-%d");
        if let Err(err) = nd {
            return Some(Err(anyhow!(
                "Failed to parse date: {} with error: {}",
                date,
                err
            )));
        }

        return Some(Ok($variant(nd.unwrap())));
    };
}

impl Iterator for MetadataParser<'_> {
    type Item = Result<ObsidianMetadata>;

    fn next(&mut self) -> Option<Self::Item> {
        // Find next significant emoji
        let iter = self.metadata.by_ref();
        while let Some(grapheme) = iter.next() {
            match grapheme {
                "📅" => {
                    process_date!(self, ObsidianMetadata::Due);
                }
                "⏳" => {
                    process_date!(self, ObsidianMetadata::Scheduled);
                }
                "🛫" => {
                    process_date!(self, ObsidianMetadata::Start);
                }
                "➕" => {
                    process_date!(self, ObsidianMetadata::Created);
                }
                "✅" => {
                    process_date!(self, ObsidianMetadata::Done);
                }
                "❌" => {
                    process_date!(self, ObsidianMetadata::Canceled);
                }
                "🔺" => return Some(Ok(ObsidianMetadata::Priority(Priority::Highest))),
                "⏫" => return Some(Ok(ObsidianMetadata::Priority(Priority::High))),
                "🔼" => return Some(Ok(ObsidianMetadata::Priority(Priority::Medium))),
                "🔽" => return Some(Ok(ObsidianMetadata::Priority(Priority::Low))),
                "⏬️" => return Some(Ok(ObsidianMetadata::Priority(Priority::Lowest))),
                "🔨" => {
                    let mut project = String::new();
                    while let Some(item) = iter.peek() {
                        if SIGNIFICANT_EMOJI.contains(item) {
                            break;
                        }
                        project.push_str(iter.next().unwrap());
                    }
                    return Some(Ok(ObsidianMetadata::Project(project.trim().to_string())));
                }
                &_ => continue,
            };
        }
        None
    }
}

pub fn parse(mut task_string: String) -> Option<ObsidianTask> {
    let mut task = ObsidianTask::default();

    let status = parse_preamble(&mut task_string);
    task.status = status?;

    let (metadata, uuid) = extract_task_parts(&mut task_string);
    task.uuid = uuid.and_then(|id| id.ok());
    let tags = parse_tags(&task_string);
    task.tags = tags;

    if task_string.len() == 0 {
        return None;
    }
    task.description = task_string;

    if let Some(metadata_str) = metadata {
        let md = MetadataParser::new(&metadata_str);
        for data in md.filter_map(Result::ok) {
            match data {
                ObsidianMetadata::Due(date) => task.due = Some(date),
                ObsidianMetadata::Done(date) => task.done = Some(date),
                ObsidianMetadata::Start(date) => task.start = Some(date),
                ObsidianMetadata::Created(date) => task.created = Some(date),
                ObsidianMetadata::Canceled(date) => task.canceled = Some(date),
                ObsidianMetadata::Scheduled(date) => task.scheduled = Some(date),
                ObsidianMetadata::Priority(pri) => task.priority = pri,
                ObsidianMetadata::Project(prj) => task.project = Some(prj),
            }
        }
    }

    Some(task)
}

fn extract_task_parts(task: &mut String) -> (Option<String>, Option<Result<Uuid>>) {
    // Returns a tuple with the metadata string and UUID as options
    let mut uuid: Option<Result<Uuid>> = None;
    let uuid_re = Regex::new(r"(?<whole>\[\[uuid: (?<uuid>.*)\|⚔️\]\])").unwrap();
    if let Some(caps) = uuid_re.captures(task) {
        uuid = caps.name("uuid").map(|id| {
            Uuid::parse_str(id.as_str())
                .with_context(|| format!("Failed to parse UUID: {}", id.as_str()))
        });
        if let Some(whole) = caps.name("whole") {
            *task = task.replace(whole.as_str(), "").trim().to_string();
        }
    }

    // Capture up to first significant emoji, this is our task description with tags
    let mut task_desc = String::with_capacity(task.len());
    let mut metadata = None;
    let graphemes = UnicodeSegmentation::graphemes(task.as_str(), true).collect::<Vec<&str>>();
    for grapheme in graphemes {
        if !SIGNIFICANT_EMOJI.contains(&grapheme) {
            task_desc.push_str(grapheme);
        } else {
            metadata = Some(task.replace(&task_desc, "").trim().to_string());
            break;
        }
    }

    *task = task_desc.trim().to_string();
    (metadata, uuid)
}

fn parse_preamble(task_string: &mut String) -> Option<Status> {
    // Remove the preamble: - [ ]
    let preamble_re = Regex::new(r"^\s*- \[(?<status>[x\- ])\] (?<remaining>.*)$").unwrap();
    let caps = preamble_re.captures(task_string)?;
    let status = match caps.name("status")?.as_str() {
        "x" => Status::Complete,
        "-" => Status::Canceled,
        " " => Status::Pending,
        _ => return None,
    };
    *task_string = caps.name("remaining")?.as_str().to_owned();
    Some(status)
}

fn parse_tags(task_string: &String) -> Vec<String> {
    let mut tags = Vec::new();
    let mut graphemes = task_string.graphemes(true);
    while let Some(grapheme) = graphemes.next() {
        if grapheme == "#" {
            let some_tags: String = graphemes.clone().take_while(|item| *item != " ").collect();
            for a_tag in some_tags.split(|tag| tag == '/') {
                tags.push(a_tag.to_string());
            }
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use std::fs::Metadata;

    use chrono_tz::America;

    use super::*;

    #[test]
    fn test_task_bank() {
        let test_bank = vec![
            (
                "- [ ] This is some simple text",
                Some(ObsidianTask {
                    status: Status::Pending,
                    description: String::from("This is some simple text"),
                    ..Default::default()
                }),
            ),
            (
                "- [ ] Task with due date 📅 2025-05-19",
                Some(ObsidianTask {
                    status: Status::Pending,
                    description: String::from("Task with due date"),
                    due: Some(
                        chrono_tz::America::Chicago
                            .with_ymd_and_hms(2025, 5, 19, 0, 0, 0)
                            .unwrap().date_naive()
                    ),
                    ..Default::default()
                }),
            ),
            (
                "- [x] Task with due date and creation date 📅 2025-05-27 ➕ 2025-05-19",
                Some(ObsidianTask {
                    status: Status::Complete,
                    description: String::from("Task with due date and creation date"),
                    due: Some(
                        chrono_tz::America::Chicago
                            .with_ymd_and_hms(2025, 5, 27, 0, 0, 0)
                            .unwrap().date_naive(),
                    ),
                    created: Some(
                        chrono_tz::America::Chicago
                            .with_ymd_and_hms(2025, 5, 19, 0, 0, 0)
                            .unwrap().date_naive(),
                    ),
                    ..Default::default()
                }),
            ),
            (
                "- [ ] Task with existing uuid [[uuid: a80c42ce-dd29-4dc7-8582-34f36fcf8b80|⚔️]]",
                Some(ObsidianTask {
                    status: Status::Pending,
                    description: String::from("Task with existing uuid"),
                    uuid: Some(Uuid::from_str("a80c42ce-dd29-4dc7-8582-34f36fcf8b80").unwrap()),
                    ..Default::default()
                }),
            ),
            (
                "- [ ] Task with invalid uuid [[uuid: uh-oh|⚔️]]",
                Some(ObsidianTask {
                    status: Status::Pending,
                    description: String::from("Task with invalid uuid"),
                    ..Default::default()
                }),
            ),
            (
                "- [ ] Task with #some/tags",
                Some(ObsidianTask {
                    status: Status::Pending,
                    description: String::from("Task with #some/tags"),
                    tags: vec![String::from("some"), String::from("tags")],
                    ..Default::default()
                }),
            ),
            (
                " - [-] Task with a project 🔨 Project text 🙂",
                Some(ObsidianTask {
                    status: Status::Canceled,
                    description: String::from("Task with a project"),
                    project: Some(String::from("Project text 🙂")),
                    ..Default::default()
                }),
            ),
        ];

        for test in test_bank {
            let test_local = String::from(test.0);
            let task = parse(test_local);
            assert_eq!(task, test.1);
        }
    }

    #[test]
    fn test_task_desc_trivial() {
        let mut trivial_case = String::from("");
        let (metadata, uuid) = extract_task_parts(&mut trivial_case);
        assert!(metadata.is_none());
        assert!(uuid.is_none());
        assert_eq!(trivial_case, "");
    }

    #[test]
    fn test_task_desc_simple() {
        let mut simple_task = String::from("This is some simple text");
        let (metadata, uuid) = extract_task_parts(&mut simple_task);
        assert!(metadata.is_none());
        assert!(uuid.is_none());
        assert_eq!(simple_task, "This is some simple text");
    }

    #[test]
    fn test_task_desc_with_metadata() {
        let mut task = String::from("Task data that is 📅 2025-05-19");
        let (metadata, uuid) = extract_task_parts(&mut task);
        assert_eq!(metadata.clone().unwrap(), "📅 2025-05-19");
        assert!(uuid.is_none());
        assert_eq!(task, "Task data that is");

        let metadata_str = metadata.clone().unwrap();
        let mut metadata_iter = MetadataParser::new(&metadata_str);
        let reference = chrono_tz::America::Chicago
            .with_ymd_and_hms(2025, 5, 19, 0, 0, 0)
            .unwrap().date_naive();
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Due(reference)
        );
    }

    #[test]
    fn test_task_desc_only_metadata() {
        let mut task = String::from("📅 2025-05-19");
        let (metadata, uuid) = extract_task_parts(&mut task);
        assert_eq!(metadata.clone().unwrap(), "📅 2025-05-19");
        assert!(uuid.is_none());
        assert_eq!(task, "");

        let metadata_str = metadata.clone().unwrap();
        let mut metadata_iter = MetadataParser::new(&metadata_str);
        let reference = chrono_tz::America::Chicago
            .with_ymd_and_hms(2025, 5, 19, 0, 0, 0)
            .unwrap();
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Due(reference.date_naive())
        );
    }

    #[test]
    fn test_task_desc_emojis() {
        let mut task = String::from("Make a  🥪 📅 2025-05-19");
        let (metadata, uuid) = extract_task_parts(&mut task);
        assert_eq!(metadata.clone().unwrap(), "📅 2025-05-19");
        assert!(uuid.is_none());
        assert_eq!(task, "Make a  🥪");

        let metadata_str = metadata.clone().unwrap();
        let mut metadata_iter = MetadataParser::new(&metadata_str);
        let reference = chrono_tz::America::Chicago
            .with_ymd_and_hms(2025, 5, 19, 0, 0, 0)
            .unwrap();
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Due(reference.date_naive())
        );
    }

    #[test]
    fn test_uuid() {
        let mut task =
            String::from("Test task stuff [[uuid: 96bb3816-aedd-4033-8ff6-4746a700aac8|⚔️]]");
        let (metadata, uuid) = extract_task_parts(&mut task);
        assert!(metadata.is_none());
        assert_eq!(
            uuid.unwrap().unwrap(),
            Uuid::parse_str("96bb3816-aedd-4033-8ff6-4746a700aac8").unwrap()
        );
        assert_eq!(task, "Test task stuff");
    }

    #[test]
    fn test_bad_uuid() {
        let mut task = String::from("Test task stuff [[uuid: abcd|⚔️]]");
        let (metadata, uuid) = extract_task_parts(&mut task);
        assert!(metadata.is_none());
        assert!(uuid.unwrap().is_err());
        assert_eq!(task, "Test task stuff");
    }

    #[test]
    fn test_metadata_and_uuid() {
        let mut task = String::from(
            "Test task stuff 📅 2025-05-19 [[uuid: 96bb3816-aedd-4033-8ff6-4746a700aac8|⚔️]]",
        );
        let (metadata, uuid) = extract_task_parts(&mut task);
        assert_eq!(metadata.unwrap(), "📅 2025-05-19");
        assert_eq!(
            uuid.unwrap().unwrap(),
            Uuid::parse_str("96bb3816-aedd-4033-8ff6-4746a700aac8").unwrap()
        );
        assert_eq!(task, "Test task stuff");
    }

    #[test]
    fn test_all_date_types() {
        let mut task = String::from(
            "Test task stuff 📅 2025-05-19 ⏳ 2025-05-19 🛫 2025-05-19 ➕ 2025-05-19 ✅ 2025-05-19 ❌ 2025-05-19 [[uuid: 96bb3816-aedd-4033-8ff6-4746a700aac8|⚔️]]",
        );
        let (metadata, uuid) = extract_task_parts(&mut task);
        assert_eq!(
            metadata.clone().unwrap(),
            "📅 2025-05-19 ⏳ 2025-05-19 🛫 2025-05-19 ➕ 2025-05-19 ✅ 2025-05-19 ❌ 2025-05-19"
        );
        assert_eq!(
            uuid.unwrap().unwrap(),
            Uuid::parse_str("96bb3816-aedd-4033-8ff6-4746a700aac8").unwrap()
        );
        assert_eq!(task, "Test task stuff");

        let metadata_str = metadata.clone().unwrap();
        let mut metadata_iter = MetadataParser::new(&metadata_str);
        let reference = chrono_tz::America::Chicago
            .with_ymd_and_hms(2025, 5, 19, 0, 0, 0)
            .unwrap();
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Due(reference.date_naive())
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Scheduled(reference.date_naive())
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Start(reference.date_naive())
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Created(reference.date_naive())
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Done(reference.date_naive())
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Canceled(reference.date_naive())
        );
    }

    #[test]
    fn test_priority() {
        let mut task = String::from(
            "Test task stuff 🔺⏫🔼🔽⏬️ [[uuid: 96bb3816-aedd-4033-8ff6-4746a700aac8|⚔️]]",
        );
        let (metadata, uuid) = extract_task_parts(&mut task);
        assert_eq!(metadata.clone().unwrap(), "🔺⏫🔼🔽⏬️");
        assert_eq!(
            uuid.unwrap().unwrap(),
            Uuid::parse_str("96bb3816-aedd-4033-8ff6-4746a700aac8").unwrap()
        );
        assert_eq!(task, "Test task stuff");

        let metadata_str = metadata.clone().unwrap();
        let mut metadata_iter = MetadataParser::new(&metadata_str);
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Priority(Priority::Highest)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Priority(Priority::High)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Priority(Priority::Medium)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Priority(Priority::Low)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Priority(Priority::Lowest)
        );
    }

    #[test]
    fn test_all() {
        let mut task = String::from(
            "Test #task stuff #project/tag 📅 2025-05-19 ⏳ 2025-05-19 🛫 2025-05-19 ➕ 2025-05-19 ✅ 2025-05-19 ❌ 2025-05-19 🔨 This is a project 🔺⏫🔼🔽⏬️ [[uuid: 96bb3816-aedd-4033-8ff6-4746a700aac8|⚔️]]",
        );
        let (metadata, uuid) = extract_task_parts(&mut task);
        assert_eq!(
            metadata.clone().unwrap(),
            "📅 2025-05-19 ⏳ 2025-05-19 🛫 2025-05-19 ➕ 2025-05-19 ✅ 2025-05-19 ❌ 2025-05-19 🔨 This is a project 🔺⏫🔼🔽⏬️"
        );
        assert_eq!(
            uuid.unwrap().unwrap(),
            Uuid::parse_str("96bb3816-aedd-4033-8ff6-4746a700aac8").unwrap()
        );
        assert_eq!(task, "Test #task stuff #project/tag");

        let metadata_str = metadata.clone().unwrap();
        let mut metadata_iter = MetadataParser::new(&metadata_str);
        let reference = chrono_tz::America::Chicago
            .with_ymd_and_hms(2025, 5, 19, 0, 0, 0)
            .unwrap();
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Due(reference.date_naive())
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Scheduled(reference.date_naive())
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Start(reference.date_naive())
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Created(reference.date_naive())
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Done(reference.date_naive())
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Canceled(reference.date_naive())
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Project("This is a project".to_string())
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Priority(Priority::Highest)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Priority(Priority::High)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Priority(Priority::Medium)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Priority(Priority::Low)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Priority(Priority::Lowest)
        );

        let tags = parse_tags(&task);
        assert_eq!(tags, ["task", "project", "tag"]);
    }

    #[test]
    fn test_date_parse_fail() {
        let mut task =
            String::from("Test task stuff 📅25 [[uuid: 96bb3816-aedd-4033-8ff6-4746a700aac8|⚔️]]");
        let (metadata, _) = extract_task_parts(&mut task);
        let metadata_str = metadata.clone().unwrap();
        let mut metadata_iter = MetadataParser::new(&metadata_str);
        assert!(metadata_iter.next().unwrap().is_err());
    }

    #[test]
    fn test_project() {
        let mut task = String::from(
            "Test task stuff 🔨 This is a test project [[uuid: 96bb3816-aedd-4033-8ff6-4746a700aac8|⚔️]]",
        );
        let (metadata, uuid) = extract_task_parts(&mut task);
        assert_eq!(metadata.clone().unwrap(), "🔨 This is a test project");
        assert_eq!(
            uuid.unwrap().unwrap(),
            Uuid::parse_str("96bb3816-aedd-4033-8ff6-4746a700aac8").unwrap()
        );
        assert_eq!(task, "Test task stuff");

        let metadata_str = metadata.clone().unwrap();
        let mut metadata_iter = MetadataParser::new(&metadata_str);
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Project("This is a test project".to_string())
        );
    }

    #[test]
    fn test_parse_preamble() {
        let mut trivial_case = String::from("");
        let status = parse_preamble(&mut trivial_case);
        assert!(status.is_none());
        assert_eq!(trivial_case, "");
    }

    #[test]
    fn test_no_task() {
        let mut no_task = String::from("This contains no task");
        let status = parse_preamble(&mut no_task);
        assert!(status.is_none());
        assert_eq!(no_task, "This contains no task");
    }

    #[test]
    fn test_simple_task() {
        let mut simple_task = String::from("- [ ] Complete this test");
        let status = parse_preamble(&mut simple_task);
        assert_eq!(status.unwrap(), Status::Pending);
        assert_eq!(simple_task, "Complete this test");
    }

    #[test]
    fn test_whitespace_task() {
        let mut whitespace_task = String::from("    - [ ] Complete this test");
        let status = parse_preamble(&mut whitespace_task);
        assert_eq!(status.unwrap(), Status::Pending);
        assert_eq!(whitespace_task, "Complete this test");
    }

    #[test]
    fn test_all_status_types() {
        let pending = "- [ ] Pending task";
        let canceled = "- [-] Canceled task";
        let completed = "- [x] Completed task";
        let mut pending_string = String::from(pending);
        let mut canceled_string = String::from(canceled);
        let mut completed_string = String::from(completed);
        let pending_status = parse_preamble(&mut pending_string);
        let canceled_status = parse_preamble(&mut canceled_string);
        let completed_status = parse_preamble(&mut completed_string);
        assert_eq!(pending_status.unwrap(), Status::Pending);
        assert_eq!(canceled_status.unwrap(), Status::Canceled);
        assert_eq!(completed_status.unwrap(), Status::Complete);
    }

    #[test]
    fn test_parse_tags() {
        let tag_string = String::from("#These/are/some_tags and #tags");
        let tags = parse_tags(&tag_string);
        assert_eq!(tags, ["These", "are", "some_tags", "tags"]);
    }

    #[test]
    fn test_full_parse() {
        let task_string = String::from("- [ ] This is a simple #task 📅 2025-05-21");
        let task = parse(task_string);
        let ref_task = ObsidianTask {
            status: Status::Pending,
            due: Some(
                chrono_tz::America::Chicago
                    .with_ymd_and_hms(2025, 5, 21, 0, 0, 0)
                    .unwrap().date_naive(),
            ),
            tags: vec!["task".to_string()],
            description: String::from("This is a simple #task"),
            ..Default::default()
        };
        assert_eq!(task.unwrap(), ref_task);
    }

    #[test]
    fn test_compare_functions() {
        let uuid = Uuid::new_v4();
        let task = ObsidianTask {
            uuid: Some(uuid),
            description: String::from("This is a test"),
            status: Status::Pending,
            due: Some(
                chrono::NaiveDate::parse_from_str("2025-06-02", "%Y-%m-%d").unwrap()
            ),
            scheduled: Some(
                chrono::NaiveDate::from_ymd_opt(2025, 6, 1).unwrap()
            ),
            tags: vec![
                String::from("test"),
                String::from("test2"),
                String::from("next"),
            ],
            project: Some(String::from("Test project")),
            priority: Priority::Highest,
            ..Default::default()
        };
        let mut ops = taskchampion::Operations::new();
        let mut tc_data = taskchampion::TaskData::create(uuid, &mut ops);
        tc_data.update(
            "scheduled",
            Some(
                chrono::NaiveDate::from_ymd_opt(2025, 6, 1).unwrap().and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp().to_string()
            ),
            &mut ops,
        );
        tc_data.update(
            "due",
            Some(
                chrono::NaiveDate::from_ymd_opt(2025, 6, 2).unwrap().and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp().to_string()
            ),
            &mut ops,
        );
        tc_data.update("tag_test", Some(String::from("")), &mut ops);
        tc_data.update("tag_test2", Some(String::from("")), &mut ops);
        tc_data.update("tag_next", Some(String::from("")), &mut ops);
        tc_data.update("priority", Some(String::from("H")), &mut ops);
        tc_data.update("project", Some(String::from("Test project")), &mut ops);
        let mut replica = taskchampion::Replica::new(
            taskchampion::StorageConfig::InMemory
                .into_storage()
                .unwrap(),
        );
        let _ = replica.commit_operations(ops);
        let mut tc_task = replica.get_task(uuid).unwrap().unwrap();
        ops = taskchampion::Operations::new();
        tc_task.set_status(taskchampion::Status::Pending, &mut ops);
        tc_task.set_description("This is a test".to_string(), &mut ops);

        assert!(task.compare_schedule(&tc_task));
        assert!(task.compare_due(&tc_task));
        assert!(task.compare_tags(&tc_task));
        assert!(task.compare_project(&tc_task));
        assert!(task.compare_priority(&tc_task));
        assert_eq!(task, tc_task);
    }
}
