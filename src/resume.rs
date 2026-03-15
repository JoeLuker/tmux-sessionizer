use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
};

use error_stack::ResultExt;
use serde_derive::Deserialize;

use crate::{
    error::TmsError,
    Result,
};

#[derive(Debug, Deserialize)]
struct HistoryEntry {
    display: Option<String>,
    timestamp: Option<i64>,
    project: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClaudeSession {
    pub session_id: String,
    pub project: String,
    pub project_name: String,
    pub last_message: String,
    pub timestamp: i64,
}

impl ClaudeSession {
    pub fn display_line(&self) -> String {
        let time = format_timestamp(self.timestamp);
        let msg = truncate(&self.last_message, 50);
        format!("{} │ {:<30} │ {}", time, self.project_name, msg)
    }
}

fn format_timestamp(ts: i64) -> String {
    let secs = ts / 1000;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let ago = now - secs;

    if ago < 3600 {
        format!("{:>3}m ago", ago / 60)
    } else if ago < 86400 {
        format!("{:>3}h ago", ago / 3600)
    } else {
        format!("{:>3}d ago", ago / 86400)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

pub fn load_claude_sessions(max: usize) -> Result<Vec<ClaudeSession>> {
    let history_path = dirs::home_dir()
        .ok_or(TmsError::IoError)
        .attach_printable("Could not find home directory")?
        .join(".claude/history.jsonl");

    let content = fs::read_to_string(&history_path)
        .change_context(TmsError::IoError)
        .attach_printable("Could not read ~/.claude/history.jsonl")?;

    let mut sessions: HashMap<String, ClaudeSession> = HashMap::new();

    for line in content.lines() {
        let entry: HistoryEntry = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let session_id = match entry.session_id {
            Some(id) if !id.is_empty() => id,
            _ => continue,
        };

        let project = entry.project.unwrap_or_default();
        let display = entry.display.unwrap_or_default();
        let timestamp = entry.timestamp.unwrap_or(0);

        // Skip /exit, /resume, and other slash commands as the "last message"
        let is_command = display.starts_with('/');

        let project_name = PathBuf::from(&project)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| project.clone());

        if let Some(existing) = sessions.get_mut(&session_id) {
            if timestamp > existing.timestamp {
                existing.timestamp = timestamp;
                if !is_command && !display.is_empty() {
                    existing.last_message = display;
                }
            }
        } else {
            sessions.insert(session_id.clone(), ClaudeSession {
                session_id,
                project,
                project_name,
                last_message: if is_command { String::new() } else { display },
                timestamp,
            });
        }
    }

    let mut sorted: Vec<ClaudeSession> = sessions.into_values().collect();
    sorted.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    sorted.truncate(max);

    Ok(sorted)
}
