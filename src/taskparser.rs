use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, offset::LocalResult};
use chrono_tz::Tz;
use regex::Regex;
use std::iter::Peekable;
use std::{str::FromStr, string::String};
use taskchampion::Uuid;
use unicode_segmentation::{Graphemes, UnicodeSegmentation};
use std::fmt;

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
    pub due: Option<DateTime<Tz>>,
    pub scheduled: Option<DateTime<Tz>>,
    pub start: Option<DateTime<Tz>>,
    pub created: Option<DateTime<Tz>>,
    pub done: Option<DateTime<Tz>>,
    pub canceled: Option<DateTime<Tz>>,
    pub priority: Priority,
    pub project: Option<String>,
}

#[derive(Debug, PartialEq)]
enum ObsidianMetadata {
    Due(DateTime<Tz>),
    Scheduled(DateTime<Tz>),
    Start(DateTime<Tz>),
    Created(DateTime<Tz>),
    Done(DateTime<Tz>),
    Canceled(DateTime<Tz>),
    Priority(Priority),
    Project(String),
}

struct MetadataParser<'a> {
    metadata: Peekable<Graphemes<'a>>,
    timezone: Tz,
}

impl MetadataParser<'_> {
    fn new(input: &'_ String, timezone: Tz) -> MetadataParser<'_> {
        MetadataParser {
            metadata: input.graphemes(true).peekable(),
            timezone,
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

        let local_nd = NaiveDateTime::from(nd.unwrap()).and_local_timezone($parser.timezone);

        match local_nd {
            LocalResult::Single(single_nd) => return Some(Ok($variant(single_nd))),
            LocalResult::Ambiguous(single_nd, _) => return Some(Ok($variant(single_nd))),
            LocalResult::None => {
                return Some(Err(anyhow!(
                    "Error converting to timezone: {}",
                    $parser.timezone
                )))
            }
        };
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

pub fn parse(mut task_string: String, tz: chrono_tz::Tz) -> Option<ObsidianTask> {
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
        let md = MetadataParser::new(&metadata_str, tz);
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
                            .unwrap(),
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
                            .unwrap()
                    ),
                    created: Some(
                        chrono_tz::America::Chicago
                            .with_ymd_and_hms(2025, 5, 19, 0, 0, 0)
                            .unwrap()
                    ),
                    ..Default::default()
                })
            ),
            (
                "- [ ] Task with existing uuid [[uuid: a80c42ce-dd29-4dc7-8582-34f36fcf8b80|⚔️]]",
                Some(ObsidianTask {
                    status: Status::Pending,
                    description: String::from("Task with existing uuid"),
                    uuid: Some(Uuid::from_str("a80c42ce-dd29-4dc7-8582-34f36fcf8b80").unwrap()),
                    ..Default::default()
                })
            ),
            (
                "- [ ] Task with invalid uuid [[uuid: uh-oh|⚔️]]",
                Some(ObsidianTask {
                    status: Status::Pending,
                    description: String::from("Task with invalid uuid"),
                    ..Default::default()
                })
            ),
            (
                "- [ ] Task with #some/tags",
                Some(ObsidianTask {
                    status: Status::Pending,
                    description: String::from("Task with #some/tags"),
                    tags: vec!(String::from("some"), String::from("tags")),
                    ..Default::default()
                })
            ),
            (
                " - [-] Task with a project 🔨 Project text 🙂",
                Some(ObsidianTask {
                    status: Status::Canceled,
                    description: String::from("Task with a project"),
                    project: Some(String::from("Project text 🙂")),
                    ..Default::default()
                })
            )
        ];

        for test in test_bank {
            let test_local = String::from(test.0);
            let task = parse(test_local, chrono_tz::America::Chicago);
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
        let mut metadata_iter = MetadataParser::new(&metadata_str, chrono_tz::America::Chicago);
        let reference = chrono_tz::America::Chicago
            .with_ymd_and_hms(2025, 5, 19, 0, 0, 0)
            .unwrap();
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
        let mut metadata_iter = MetadataParser::new(&metadata_str, chrono_tz::America::Chicago);
        let reference = chrono_tz::America::Chicago
            .with_ymd_and_hms(2025, 5, 19, 0, 0, 0)
            .unwrap();
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Due(reference)
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
        let mut metadata_iter = MetadataParser::new(&metadata_str, chrono_tz::America::Chicago);
        let reference = chrono_tz::America::Chicago
            .with_ymd_and_hms(2025, 5, 19, 0, 0, 0)
            .unwrap();
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Due(reference)
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
        let mut metadata_iter = MetadataParser::new(&metadata_str, chrono_tz::America::Chicago);
        let reference = chrono_tz::America::Chicago
            .with_ymd_and_hms(2025, 5, 19, 0, 0, 0)
            .unwrap();
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Due(reference)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Scheduled(reference)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Start(reference)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Created(reference)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Done(reference)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Canceled(reference)
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
        let mut metadata_iter = MetadataParser::new(&metadata_str, chrono_tz::America::Chicago);
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
        let mut metadata_iter = MetadataParser::new(&metadata_str, chrono_tz::America::Chicago);
        let reference = chrono_tz::America::Chicago
            .with_ymd_and_hms(2025, 5, 19, 0, 0, 0)
            .unwrap();
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Due(reference)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Scheduled(reference)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Start(reference)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Created(reference)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Done(reference)
        );
        assert_eq!(
            metadata_iter.next().unwrap().unwrap(),
            ObsidianMetadata::Canceled(reference)
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
        let mut metadata_iter = MetadataParser::new(&metadata_str, chrono_tz::America::Chicago);
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
        let mut metadata_iter = MetadataParser::new(&metadata_str, chrono_tz::America::Chicago);
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
        let task = parse(task_string, America::Chicago);
        let ref_task = ObsidianTask {
            status: Status::Pending,
            due: Some(
                chrono_tz::America::Chicago
                    .with_ymd_and_hms(2025, 5, 21, 0, 0, 0)
                    .unwrap(),
            ),
            tags: vec!["task".to_string()],
            description: String::from("This is a simple #task"),
            ..Default::default()
        };
        assert_eq!(task.unwrap(), ref_task);
    }
}
