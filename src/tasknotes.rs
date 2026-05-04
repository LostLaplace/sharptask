use anyhow::{Context, Result};
use chrono::NaiveDate;
use regex::Regex;
use serde::Deserialize;
use std::path::Path;
use taskchampion::Uuid;

use crate::taskparser::{ObsidianTask, ObsidianTaskBuilder, Priority, Status};

/// Extra TC fields not modelled in `ObsidianTask` that are specific to
/// the TaskNotes format (or TC UDAs set by external tools like `tasksh`).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TaskNotesExtra {
    /// Date the task was last reviewed via `tasksh review`.
    pub reviewed: Option<NaiveDate>,
}

/// YAML frontmatter fields recognized by the TaskNotes Obsidian plugin.
/// Unknown fields in the file are ignored during deserialization.
/// Serialization is done via a generic `serde_yaml::Value` merge in `write_file`
/// so that unrecognized fields are never dropped.
#[derive(Debug, Deserialize, Default)]
struct Frontmatter {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    due: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scheduled: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cancelled: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    /// Taskwarrior UUID — written by sharptask to track the TC representation.
    #[serde(skip_serializing_if = "Option::is_none")]
    tc_uuid: Option<String>,
    /// Last review date set by `tasksh review` (UDA `reviewed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    reviewed: Option<String>,
}

/// Split file content into (frontmatter_yaml, body).
/// Returns `None` if the file does not start with a `---` fence.
fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let rest = content.strip_prefix("---\n")?;
    // frontmatter ends at the first `\n---` followed by either `\n` or end-of-string
    if let Some(end) = rest.find("\n---\n") {
        Some((&rest[..end], &rest[end + 5..]))
    } else if let Some(end) = rest.find("\n---") {
        // closing fence at very end of file with no trailing newline
        Some((&rest[..end], &rest[end + 4..]))
    } else {
        None
    }
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

fn parse_status(s: Option<&str>) -> Status {
    match s.map(|v| v.to_lowercase()).as_deref() {
        Some("done") | Some("complete") | Some("completed") => Status::Complete,
        Some("cancelled") | Some("canceled") => Status::Canceled,
        _ => Status::Pending,
    }
}

fn parse_priority(s: Option<&str>) -> Priority {
    match s.map(|v| v.to_lowercase()).as_deref() {
        Some("highest") => Priority::Highest,
        Some("high") => Priority::High,
        Some("medium") => Priority::Medium,
        Some("low") => Priority::Low,
        Some("lowest") => Priority::Lowest,
        _ => Priority::Normal,
    }
}

fn status_to_str(s: &Status) -> &'static str {
    match s {
        Status::Pending => "todo",
        Status::Complete => "done",
        Status::Canceled => "cancelled",
    }
}

fn priority_to_str(p: &Priority) -> Option<&'static str> {
    match p {
        Priority::Normal => None,
        Priority::Lowest => Some("lowest"),
        Priority::Low => Some("low"),
        Priority::Medium => Some("medium"),
        Priority::High => Some("high"),
        Priority::Highest => Some("highest"),
    }
}

/// Strip ` #tagname` inline-tag patterns appended to the description by the
/// `From<taskchampion::Task>` conversion. TaskNotes stores tags separately in
/// the YAML frontmatter, so the title should be clean.
fn strip_inline_tags(description: &str) -> String {
    let re = Regex::new(r" #\S+").unwrap();
    re.replace_all(description, "").trim().to_string()
}

/// Parse a TaskNotes `.md` file into an `ObsidianTask` and `TaskNotesExtra`.
/// Returns `Ok(None)` if the file has no YAML frontmatter or no `title` field.
pub fn parse_file(
    path: &Path,
    tz: &chrono_tz::Tz,
) -> Result<Option<(ObsidianTask, TaskNotesExtra)>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {:?}", path))?;

    let (fm_str, _body) = match split_frontmatter(&content) {
        Some(parts) => parts,
        None => return Ok(None),
    };

    let fm: Frontmatter = serde_yaml::from_str(fm_str)
        .with_context(|| format!("Failed to parse YAML frontmatter in {:?}", path))?;

    // Real TaskNotes files use the filename as the task title rather than a
    // `title:` frontmatter field.  Fall back to the file stem so those files
    // are not silently skipped.
    let title = fm
        .title
        .filter(|t| !t.trim().is_empty())
        .or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();
    if title.is_empty() {
        return Ok(None);
    }

    let uuid = fm
        .tc_uuid
        .as_deref()
        .and_then(|u| Uuid::parse_str(u).ok());

    let task = ObsidianTaskBuilder::new()
        .description(title)
        .status(parse_status(fm.status.as_deref()))
        .priority(parse_priority(fm.priority.as_deref()))
        .uuid_opt(uuid)
        .due(fm.due.as_deref().and_then(parse_date))
        .scheduled(fm.scheduled.as_deref().and_then(parse_date))
        .start(fm.start.as_deref().and_then(parse_date))
        .created(fm.created.as_deref().and_then(parse_date))
        .done(fm.completed.as_deref().and_then(parse_date))
        .canceled(fm.cancelled.as_deref().and_then(parse_date))
        .tags(&fm.tags.unwrap_or_default())
        .project(fm.project)
        .build()
        .with_tz(tz);

    let extra = TaskNotesExtra {
        reviewed: fm.reviewed.as_deref().and_then(parse_date),
    };

    Ok(Some((task, extra)))
}

/// Write an `ObsidianTask` back to a TaskNotes `.md` file.
///
/// Only the fields sharptask owns are updated; all other frontmatter keys
/// (e.g. `dateCreated`, `dateModified`, `projects`) are preserved verbatim.
/// The note body below the closing `---` fence is also left untouched.
pub fn write_file(path: &Path, task: &ObsidianTask, extra: &TaskNotesExtra) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {:?}", path))?;

    let (fm_str, body) = split_frontmatter(&content)
        .context("Cannot write to a file that has no YAML frontmatter")?;

    // Parse into a generic Value so unknown fields survive the round-trip.
    let mut fm_val: serde_yaml::Value = serde_yaml::from_str(fm_str)
        .with_context(|| format!("Failed to parse YAML frontmatter in {:?}", path))?;
    let map = fm_val
        .as_mapping_mut()
        .context("Frontmatter is not a YAML mapping")?;

    macro_rules! set_str {
        ($key:expr, $val:expr) => {{
            map.insert(
                serde_yaml::Value::String($key.to_string()),
                serde_yaml::Value::String($val),
            );
        }};
    }
    macro_rules! remove_key {
        ($key:expr) => {{
            map.remove(&serde_yaml::Value::String($key.to_string()));
        }};
    }

    // Only write `title` back if the file already carried one (real TaskNotes
    // files use the filename as the title and have no `title:` field).
    if map.contains_key(&serde_yaml::Value::String("title".to_string())) {
        set_str!("title", strip_inline_tags(&task.description));
    }

    set_str!("status", status_to_str(&task.status).to_string());

    match priority_to_str(&task.priority) {
        Some(p) => set_str!("priority", p.to_string()),
        None => remove_key!("priority"),
    }

    match task.due {
        Some(d) => set_str!("due", d.format("%Y-%m-%d").to_string()),
        None => remove_key!("due"),
    }
    match task.scheduled {
        Some(d) => set_str!("scheduled", d.format("%Y-%m-%d").to_string()),
        None => remove_key!("scheduled"),
    }
    match task.start {
        Some(d) => set_str!("start", d.format("%Y-%m-%d").to_string()),
        None => remove_key!("start"),
    }
    match task.done {
        Some(d) => set_str!("completed", d.format("%Y-%m-%d").to_string()),
        None => remove_key!("completed"),
    }
    match task.canceled {
        Some(d) => set_str!("cancelled", d.format("%Y-%m-%d").to_string()),
        None => remove_key!("cancelled"),
    }

    // Only overwrite tags when TC has tags to contribute; otherwise leave the
    // existing frontmatter tags (e.g. the plugin's own "task" tag) in place.
    if !task.tags.is_empty() {
        let tag_vals: Vec<serde_yaml::Value> = task
            .tags
            .iter()
            .map(|t| serde_yaml::Value::String(t.clone()))
            .collect();
        map.insert(
            serde_yaml::Value::String("tags".to_string()),
            serde_yaml::Value::Sequence(tag_vals),
        );
    }

    if let Some(ref proj) = task.project {
        set_str!("project", proj.clone());
    }

    if let Some(uuid) = task.uuid {
        set_str!("tc_uuid", uuid.to_string());
    }

    match extra.reviewed {
        Some(d) => set_str!("reviewed", d.format("%Y-%m-%d").to_string()),
        None => remove_key!("reviewed"),
    }

    let fm_yaml =
        serde_yaml::to_string(&fm_val).context("Failed to serialize task frontmatter to YAML")?;
    let new_content = format!("---\n{}---\n{}", fm_yaml, body);
    std::fs::write(path, new_content)
        .with_context(|| format!("Failed to write {:?}", path))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use taskchampion::Uuid;

    fn write_temp_file(content: &str) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn test_parse_basic_task() {
        let content = "---\ntitle: Buy groceries\nstatus: todo\npriority: high\ndue: 2026-05-05\n---\nSome notes here.\n";
        let f = write_temp_file(content);
        let (task, extra) = parse_file(f.path(), &chrono_tz::UTC).unwrap().unwrap();
        assert_eq!(task.description, "Buy groceries");
        assert_eq!(task.status, Status::Pending);
        assert_eq!(task.priority, Priority::High);
        assert_eq!(
            task.due,
            Some(NaiveDate::parse_from_str("2026-05-05", "%Y-%m-%d").unwrap())
        );
        assert_eq!(extra.reviewed, None);
    }

    #[test]
    fn test_parse_with_uuid() {
        let uuid_str = "a80c42ce-dd29-4dc7-8582-34f36fcf8b80";
        let content = format!(
            "---\ntitle: Tracked task\nstatus: done\ntc_uuid: {}\n---\n",
            uuid_str
        );
        let f = write_temp_file(&content);
        let (task, _) = parse_file(f.path(), &chrono_tz::UTC).unwrap().unwrap();
        assert_eq!(task.uuid, Some(Uuid::parse_str(uuid_str).unwrap()));
        assert_eq!(task.status, Status::Complete);
    }

    #[test]
    fn test_parse_no_title_uses_filename() {
        // Files without a `title:` field fall back to the filename stem.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("My Task Title.md");
        std::fs::write(&path, "---\nstatus: todo\n---\n").unwrap();
        let (task, _) = parse_file(&path, &chrono_tz::UTC).unwrap().unwrap();
        assert_eq!(task.description, "My Task Title");
    }

    #[test]
    fn test_parse_no_frontmatter_returns_none() {
        let f = write_temp_file("# Just a plain note\nNo frontmatter here.\n");
        assert!(parse_file(f.path(), &chrono_tz::UTC).unwrap().is_none());
    }

    #[test]
    fn test_parse_reviewed() {
        let content =
            "---\ntitle: Reviewed task\nstatus: todo\nreviewed: 2026-04-01\n---\n";
        let f = write_temp_file(content);
        let (_, extra) = parse_file(f.path(), &chrono_tz::UTC).unwrap().unwrap();
        assert_eq!(
            extra.reviewed,
            Some(NaiveDate::parse_from_str("2026-04-01", "%Y-%m-%d").unwrap())
        );
    }

    #[test]
    fn test_write_roundtrip() {
        let content = "---\ntitle: Original task\nstatus: todo\n---\nBody text.\n";
        let f = write_temp_file(content);
        let (mut task, extra) = parse_file(f.path(), &chrono_tz::UTC).unwrap().unwrap();
        task.uuid = Some(Uuid::parse_str("a80c42ce-dd29-4dc7-8582-34f36fcf8b80").unwrap());
        task.status = Status::Complete;

        write_file(f.path(), &task, &extra).unwrap();

        let (updated, _) = parse_file(f.path(), &chrono_tz::UTC).unwrap().unwrap();
        assert_eq!(updated.status, Status::Complete);
        assert_eq!(updated.uuid, task.uuid);
        // Body should be preserved
        let raw = std::fs::read_to_string(f.path()).unwrap();
        assert!(raw.contains("Body text."));
    }

    #[test]
    fn test_write_reviewed_roundtrip() {
        let content = "---\ntitle: Original task\nstatus: todo\n---\n";
        let f = write_temp_file(content);
        let (task, _) = parse_file(f.path(), &chrono_tz::UTC).unwrap().unwrap();
        let extra = TaskNotesExtra {
            reviewed: Some(NaiveDate::parse_from_str("2026-04-15", "%Y-%m-%d").unwrap()),
        };
        write_file(f.path(), &task, &extra).unwrap();
        let (_, updated_extra) = parse_file(f.path(), &chrono_tz::UTC).unwrap().unwrap();
        assert_eq!(updated_extra.reviewed, extra.reviewed);
    }

    #[test]
    fn test_write_strips_inline_tags() {
        use crate::taskparser::ObsidianTaskBuilder;
        let task = ObsidianTaskBuilder::new()
            .description("My task #work #urgent")
            .tags(&["work", "urgent"])
            .build();
        let extra = TaskNotesExtra::default();
        let content = "---\ntitle: My task\n---\n";
        let f = write_temp_file(content);
        write_file(f.path(), &task, &extra).unwrap();
        let (updated, _) = parse_file(f.path(), &chrono_tz::UTC).unwrap().unwrap();
        assert_eq!(updated.description, "My task");
        assert_eq!(updated.tags, vec!["work", "urgent"]);
    }

    #[test]
    fn test_status_cancelled() {
        let f = write_temp_file("---\ntitle: Dropped task\nstatus: cancelled\n---\n");
        let (task, _) = parse_file(f.path(), &chrono_tz::UTC).unwrap().unwrap();
        assert_eq!(task.status, Status::Canceled);
    }
}
