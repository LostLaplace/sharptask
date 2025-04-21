use taskchampion::Uuid;
use chrono::DateTime;
use chrono_tz::Tz;
use regex::Regex;
use std::{str::FromStr, string::String};
use unicode_segmentation::UnicodeSegmentation;
use anyhow::{Result, Context};

#[derive(Debug, Eq, PartialEq)]
pub enum Status {
    Pending,
    Complete,
    Canceled,
}

pub enum Priority {
    Lowest,
    Low,
    Normal,
    Medium,
    High,
    Highest,
}

const SIGNIFICANT_EMOJI: &[&str] = &[
    &"📅",
    &"⏳",
    &"🛫",
    &"➕",
    &"✅",
    &"❌",
    &"🔺",
    &"⏫",
    &"🔼",
    &"🔽",
    &"⏬",
    &"🔁",
    &"🆔",
    &"⛔",
    &"🔨",
];

pub struct ObsidianTask {
	uuid: Option<Uuid>,
	status: Status,
	description: String,
	tags: Vec<String>,
	due: Option<DateTime<Tz>>,
	scheduled: Option<DateTime<Tz>>,
	start: Option<DateTime<Tz>>,
	created: Option<DateTime<Tz>>,
	done: Option<DateTime<Tz>>,
	canceled: Option<DateTime<Tz>>,
	priority: Priority,
	project: Option<String>,
}

pub fn parse<T: AsRef<str>>(task_string: T) -> Option<ObsidianTask> {
    let mut owned_task_string = String::from(task_string.as_ref());
    let status = parse_preamble(&mut owned_task_string);
    let mut task_with_metadata = owned_task_string;


    None
}

fn extract_task_parts(task: &mut String) -> (Option<String>, Option<Result<Uuid>>) {
    // Returns a tuple with the metadata string and UUID as options
    let mut uuid: Option<Result<Uuid>> = None;
    let uuid_re = Regex::new(r"(?<whole>\[\[uuid: (?<uuid>.*)\|\]\])").unwrap();
    if let Some(caps) = uuid_re.captures(task) {
        uuid = caps.name("uuid").map(|id| Uuid::parse_str(id.as_str()).with_context(|| format!("Failed to parse UUID: {}", id.as_str())));
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
        _ => return None
    };
    *task_string = caps.name("remaining")?.as_str().to_owned();
    Some(status)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(metadata.unwrap(), "📅 2025-05-19");
        assert!(uuid.is_none());
        assert_eq!(task, "Task data that is");
    }

    #[test]
    fn test_task_desc_only_metadata() {
        let mut task = String::from("📅 2025-05-19");
        let (metadata, uuid) = extract_task_parts(&mut task);
        assert_eq!(metadata.unwrap(), "📅 2025-05-19");
        assert!(uuid.is_none());
        assert_eq!(task, "");
    }

    #[test]
    fn test_task_desc_emojis() {
        let mut task = String::from("Make a  🥪 📅 2025-05-19");
        let (metadata, uuid) = extract_task_parts(&mut task);
        assert_eq!(metadata.unwrap(), "📅 2025-05-19");
        assert!(uuid.is_none());
        assert_eq!(task, "Make a  🥪");
    }

    #[test]
    fn test_uuid() {
        let mut task = String::from("Test task stuff [[uuid: 96bb3816-aedd-4033-8ff6-4746a700aac8|]]");
        let (metadata, uuid) = extract_task_parts(&mut task);
        assert!(metadata.is_none());
        assert_eq!(uuid.unwrap().unwrap(), Uuid::parse_str("96bb3816-aedd-4033-8ff6-4746a700aac8").unwrap());
        assert_eq!(task, "Test task stuff");
    }

    #[test]
    fn test_bad_uuid() {
        let mut task = String::from("Test task stuff [[uuid: abcd|]]");
        let (metadata, uuid) = extract_task_parts(&mut task);
        assert!(metadata.is_none());
        assert!(uuid.unwrap().is_err());
        assert_eq!(task, "Test task stuff");
    }

    #[test]
    fn test_metadata_and_uuid() {
        let mut task = String::from("Test task stuff 📅 2025-05-19 [[uuid: 96bb3816-aedd-4033-8ff6-4746a700aac8|]]");
        let (metadata, uuid) = extract_task_parts(&mut task);
        assert_eq!(metadata.unwrap(), "📅 2025-05-19");
        assert_eq!(uuid.unwrap().unwrap(), Uuid::parse_str("96bb3816-aedd-4033-8ff6-4746a700aac8").unwrap());
        assert_eq!(task, "Test task stuff");
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
}
