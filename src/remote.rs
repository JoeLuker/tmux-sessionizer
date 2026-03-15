use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, SystemTime},
};

use error_stack::ResultExt;
use serde_derive::{Deserialize, Serialize};

use crate::{
    configs::{Config, RemoteHost},
    error::TmsError,
    Result,
};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RemoteProject {
    pub host_name: String,
    pub host: String,
    pub remote_path: String,
    pub project_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct RemoteCache {
    projects: Vec<RemoteProject>,
}

fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap().join(".cache"))
        .join("tms/remote")
}

fn cache_path(host_name: &str) -> PathBuf {
    cache_dir().join(format!("{}.json", host_name))
}

fn is_cache_stale(host: &RemoteHost) -> bool {
    let path = cache_path(&host.name);
    match fs::metadata(&path) {
        Ok(meta) => {
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let age = SystemTime::now()
                .duration_since(modified)
                .unwrap_or(Duration::MAX);
            age > Duration::from_secs(host.cache_ttl_secs)
        }
        Err(_) => true,
    }
}

fn discover_remote_projects(host: &RemoteHost) -> Vec<RemoteProject> {
    let find_commands: Vec<String> = host
        .search_paths
        .iter()
        .map(|path| {
            format!(
                "find {} -mindepth 1 -maxdepth {} -name .git -type d 2>/dev/null",
                path, host.max_depth
            )
        })
        .collect();

    let remote_script = format!(
        "{{ {}; }} | sed 's|/.git$||' | sort -u",
        find_commands.join("; ")
    );

    let output = Command::new("ssh")
        .args(["-o", "ConnectTimeout=5", "-o", "BatchMode=yes", &host.host])
        .arg(remote_script)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout
                .lines()
                .filter(|line| !line.is_empty())
                .map(|line| {
                    let project_name = line
                        .rsplit('/')
                        .next()
                        .unwrap_or(line)
                        .to_string();
                    RemoteProject {
                        host_name: host.name.clone(),
                        host: host.host.clone(),
                        remote_path: line.to_string(),
                        project_name,
                    }
                })
                .collect()
        }
        _ => {
            eprintln!("Warning: could not reach remote host '{}'", host.name);
            Vec::new()
        }
    }
}

fn write_cache(host_name: &str, projects: &[RemoteProject]) -> Result<()> {
    let dir = cache_dir();
    fs::create_dir_all(&dir).change_context(TmsError::IoError)?;

    let cache = RemoteCache {
        projects: projects.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache).change_context(TmsError::IoError)?;
    fs::write(cache_path(host_name), json).change_context(TmsError::IoError)?;

    Ok(())
}

fn read_cache(host_name: &str) -> Option<Vec<RemoteProject>> {
    let path = cache_path(host_name);
    let content = fs::read_to_string(path).ok()?;
    let cache: RemoteCache = serde_json::from_str(&content).ok()?;
    Some(cache.projects)
}

pub fn find_remote_projects(config: &Config) -> Result<Vec<RemoteProject>> {
    let hosts = match &config.remote_hosts {
        Some(hosts) if !hosts.is_empty() => hosts,
        _ => return Ok(Vec::new()),
    };

    let mut all_projects = Vec::new();

    for host in hosts {
        if host.auto_refresh && is_cache_stale(host) {
            let projects = discover_remote_projects(host);
            if !projects.is_empty() {
                let _ = write_cache(&host.name, &projects);
            }
            all_projects.extend(projects);
        } else if let Some(cached) = read_cache(&host.name) {
            all_projects.extend(cached);
        } else {
            let projects = discover_remote_projects(host);
            let _ = write_cache(&host.name, &projects);
            all_projects.extend(projects);
        }
    }

    Ok(all_projects)
}

pub fn refresh_remote_cache(config: &Config) -> Result<usize> {
    let hosts = match &config.remote_hosts {
        Some(hosts) => hosts,
        None => return Ok(0),
    };

    let mut total = 0;

    for host in hosts {
        let projects = discover_remote_projects(host);
        total += projects.len();
        write_cache(&host.name, &projects)?;
        eprintln!(
            "Cached {} projects from '{}'",
            projects.len(),
            host.name
        );
    }

    Ok(total)
}
