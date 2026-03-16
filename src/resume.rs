use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};

use error_stack::ResultExt;
use serde_derive::Deserialize;

use crate::{
    configs::Config,
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
    /// None = local, Some(host) = remote
    pub host: Option<String>,
}

impl ClaudeSession {
    pub fn display_line(&self) -> String {
        let time = format_timestamp(self.timestamp);
        let msg = truncate(&self.last_message, 50);
        let name = match &self.host {
            Some(host) => format!("{}@{}", self.project_name, host),
            None => self.project_name.clone(),
        };
        format!("{} │ {:<35} │ {}", time, name, msg)
    }

    pub fn resume_command(&self) -> String {
        let resume = format!("cd {} && claude --resume {}", self.project, self.session_id);
        match &self.host {
            Some(host) => format!("ssh -t {} '{}'", host, resume.replace('\'', "'\\''")),
            None => resume,
        }
    }

    pub fn label(&self) -> String {
        let name = match &self.host {
            Some(host) => format!("{}@{}", self.project_name, host),
            None => self.project_name.clone(),
        };
        format!("{} | {}", name, truncate(&self.last_message, 30))
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
        format!("{}...", &s[..max])
    }
}

fn parse_history(content: &str, host: Option<&str>) -> Vec<ClaudeSession> {
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

        let is_command = display.starts_with('/');

        let project_name = PathBuf::from(&project)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| project.clone());

        // For remote sessions, prefix session_id with host to avoid collisions
        let unique_id = match host {
            Some(h) => format!("{}:{}", h, session_id),
            None => session_id.clone(),
        };

        if let Some(existing) = sessions.get_mut(&unique_id) {
            if timestamp > existing.timestamp {
                existing.timestamp = timestamp;
                if !is_command && !display.is_empty() {
                    existing.last_message = display;
                }
            }
        } else {
            sessions.insert(unique_id, ClaudeSession {
                session_id,
                project,
                project_name,
                last_message: if is_command { String::new() } else { display },
                timestamp,
                host: host.map(String::from),
            });
        }
    }

    sessions
        .into_values()
        .filter(|s| s.timestamp != 0)
        .map(|mut s| {
            if s.last_message.is_empty() {
                s.last_message = "(no messages)".to_string();
            }
            if s.project.is_empty() {
                s.project_name = "(unknown)".to_string();
            }
            s
        })
        .collect()
}

fn fetch_remote_history(host: &str) -> Option<String> {
    let output = Command::new("ssh")
        .args(["-o", "ConnectTimeout=5", "-o", "BatchMode=yes", host])
        .arg("cat ~/.claude/history.jsonl 2>/dev/null")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let content = String::from_utf8_lossy(&out.stdout).to_string();
            if content.is_empty() { None } else { Some(content) }
        }
        _ => {
            eprintln!("tms: could not fetch history from '{}'", host);
            None
        }
    }
}

pub fn load_claude_sessions(max: usize, config: &Config) -> Result<Vec<ClaudeSession>> {
    let mut all_sessions: Vec<ClaudeSession> = Vec::new();

    // Local sessions
    let history_path = dirs::home_dir()
        .ok_or(TmsError::IoError)
        .attach_printable("Could not find home directory")?
        .join(".claude/history.jsonl");

    if let Ok(content) = fs::read_to_string(&history_path) {
        all_sessions.extend(parse_history(&content, None));
    }

    // Remote sessions from configured hosts
    if let Some(hosts) = &config.remote_hosts {
        for host in hosts {
            if let Some(content) = fetch_remote_history(&host.host) {
                all_sessions.extend(parse_history(&content, Some(&host.name)));
            }
        }
    }

    all_sessions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    all_sessions.truncate(max);

    Ok(all_sessions)
}
