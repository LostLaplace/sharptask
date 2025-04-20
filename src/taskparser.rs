use taskchampion::Uuid;
use chrono::DateTime;
use chrono_tz::Tz;
use regex::Regex;
use std::string::String;

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
    "📅",
    "⏳",
    "🛫",
    "➕",
    "✅",
    "❌",
    "🔺",
    "⏫",
    "🔼",
    "🔽",
    "⏬️",
    "🔁",
    "🆔",
    "⛔",
    "🔨",
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

    // Capture up to first significant emoji, this is our task description with tags
    let mut emoji_offsets = Vec::new();
    for emoji in SIGNIFICANT_EMOJI {
        emoji_offsets.push(task_with_metadata.find(emoji));
    }

    let min_element = emoji_offsets.iter().min_by(|a, b| {
        match (a, b) {
            (Some(a_val), Some(b_val)) => a_val.cmp(b_val),
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, None) => std::cmp::Ordering::Equal,
        }
    }).cloned()?;

    let (task_description, metadata) = match min_element {
        Some(min) => task_with_metadata.split_at(min),
        None => (task_with_metadata.as_str(), "")
    };

    None
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
