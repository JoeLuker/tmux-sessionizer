use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use error_stack::ResultExt;

use crate::{
    configs::Config,
    dirty_paths::DirtyUtf8Path,
    error::TmsError,
    remote::find_remote_projects,
    repos::{find_repos, find_submodules, RepoProvider},
    tmux::Tmux,
    Result,
};

pub struct Session {
    pub name: String,
    pub session_type: SessionType,
    pub last_activity: i64,
}

pub enum SessionType {
    Git(RepoProvider),
    Bookmark(PathBuf),
    Remote {
        host: String,
        remote_path: String,
    },
}

impl Session {
    pub fn new(name: String, session_type: SessionType) -> Self {
        Session {
            name,
            session_type,
            last_activity: 0,
        }
    }

    pub fn with_activity(name: String, session_type: SessionType, last_activity: i64) -> Self {
        Session {
            name,
            session_type,
            last_activity,
        }
    }

    pub fn path(&self) -> &Path {
        match &self.session_type {
            SessionType::Git(repo) if repo.is_bare() => repo.path(),
            SessionType::Git(repo) => repo.path().parent().unwrap(),
            SessionType::Bookmark(path) => path,
            SessionType::Remote { .. } => Path::new("/tmp"),
        }
    }

    pub fn switch_to(&self, tmux: &Tmux, config: &Config) -> Result<()> {
        match &self.session_type {
            SessionType::Git(repo) => self.switch_to_repo_session(repo, tmux, config),
            SessionType::Bookmark(path) => self.switch_to_bookmark_session(tmux, path, config),
            SessionType::Remote { host, remote_path } => {
                self.switch_to_remote_session(tmux, host, remote_path)
            }
        }
    }

    fn switch_to_repo_session(
        &self,
        repo: &RepoProvider,
        tmux: &Tmux,
        config: &Config,
    ) -> Result<()> {
        let path = if repo.is_bare() {
            repo.path().to_path_buf().to_string()?
        } else {
            repo.work_dir()
                .expect("bare repositories should all have parent directories")
                .canonicalize()
                .change_context(TmsError::IoError)?
                .to_string()?
        };
        let session_name = self.name.replace('.', "_");

        if !tmux.session_exists(&session_name) {
            tmux.new_session(Some(&session_name), Some(&path));
            tmux.set_up_tmux_env(repo, &session_name, config)?;
            tmux.run_session_create_script(self.path(), &session_name, config)?;
        }

        tmux.switch_to_session(&session_name);

        Ok(())
    }

    fn switch_to_bookmark_session(&self, tmux: &Tmux, path: &Path, config: &Config) -> Result<()> {
        let session_name = self.name.replace('.', "_");

        if !tmux.session_exists(&session_name) {
            tmux.new_session(Some(&session_name), path.to_str());
            tmux.run_session_create_script(path, &session_name, config)?;
        }

        tmux.switch_to_session(&session_name);

        Ok(())
    }

    fn switch_to_remote_session(
        &self,
        tmux: &Tmux,
        host: &str,
        remote_path: &str,
    ) -> Result<()> {
        let session_name = self.name.replace('.', "_");

        if !tmux.session_exists(&session_name) {
            let ssh_command = format!("ssh -t {} 'cd {} && exec $SHELL'", host, remote_path);
            tmux.new_session_with_command(Some(&session_name), &ssh_command);
        }

        tmux.switch_to_session(&session_name);

        Ok(())
    }
}

pub trait SessionContainer {
    fn find_session(&self, name: &str) -> Option<&Session>;
    fn insert_session(&mut self, name: String, repo: Session);
    fn list(&self) -> Vec<String>;
    fn list_by_activity(&self) -> Vec<String>;
}

impl SessionContainer for HashMap<String, Session> {
    fn find_session(&self, name: &str) -> Option<&Session> {
        self.get(name)
    }

    fn insert_session(&mut self, name: String, session: Session) {
        self.insert(name, session);
    }

    fn list(&self) -> Vec<String> {
        let mut list: Vec<String> = self.keys().map(|s| s.to_owned()).collect();
        list.sort();

        list
    }

    fn list_by_activity(&self) -> Vec<String> {
        let mut entries: Vec<(&String, &Session)> = self.iter().collect();
        entries.sort_by(|a, b| b.1.last_activity.cmp(&a.1.last_activity));
        entries.into_iter().map(|(k, _)| k.to_owned()).collect()
    }
}

pub fn create_sessions(config: &Config) -> Result<impl SessionContainer> {
    let mut sessions = find_repos(config)?;
    sessions = append_bookmarks(config, sessions)?;
    sessions = append_remote_projects(config, sessions)?;

    let sessions = generate_session_container(sessions, config)?;

    Ok(sessions)
}

fn generate_session_container(
    mut sessions: HashMap<String, Vec<Session>>,
    config: &Config,
) -> Result<impl SessionContainer> {
    let mut ret = HashMap::new();

    for list in sessions.values_mut() {
        if list.len() == 1 {
            let session = list.pop().unwrap();
            insert_session(&mut ret, session, config)?;
        } else {
            let deduplicated = deduplicate_sessions(list);

            for session in deduplicated {
                insert_session(&mut ret, session, config)?;
            }
        }
    }

    Ok(ret)
}

fn insert_session(
    sessions: &mut impl SessionContainer,
    session: Session,
    config: &Config,
) -> Result<()> {
    let visible_name = if config.display_full_path == Some(true) {
        session.path().display().to_string()
    } else {
        session.name.clone()
    };
    if let SessionType::Git(repo) = &session.session_type {
        if config.search_submodules == Some(true) {
            if let Ok(Some(submodules)) = repo.submodules() {
                find_submodules(submodules, &visible_name, sessions, config)?;
            }
        }
    }
    sessions.insert_session(visible_name, session);
    Ok(())
}

fn deduplicate_sessions(duplicate_sessions: &mut Vec<Session>) -> Vec<Session> {
    let mut depth = 1;
    let mut deduplicated = Vec::new();
    while let Some(current_session) = duplicate_sessions.pop() {
        let mut equal = true;
        let current_path = current_session.path();
        let mut current_depth = 1;

        while equal {
            equal = false;
            if let Some(current_str) = current_path.iter().rev().nth(current_depth) {
                for session in &mut *duplicate_sessions {
                    if let Some(str) = session.path().iter().rev().nth(current_depth) {
                        if str == current_str {
                            current_depth += 1;
                            equal = true;
                            break;
                        }
                    }
                }
            }
        }

        deduplicated.push(current_session);
        depth = depth.max(current_depth);
    }

    for session in &mut deduplicated {
        session.name = {
            let mut count = depth + 1;
            let mut iterator = session.path().iter().rev();
            let mut str = String::new();

            while count > 0 {
                if let Some(dir) = iterator.next() {
                    if str.is_empty() {
                        str = dir.to_string_lossy().to_string();
                    } else {
                        str = format!("{}/{}", dir.to_string_lossy(), str);
                    }
                    count -= 1;
                } else {
                    count = 0;
                }
            }

            str
        };
    }

    deduplicated
}

fn append_bookmarks(
    config: &Config,
    mut sessions: HashMap<String, Vec<Session>>,
) -> Result<HashMap<String, Vec<Session>>> {
    let bookmarks = config.bookmark_paths();

    for path in bookmarks {
        let session_name = path
            .file_name()
            .expect("The file name doesn't end in `..`")
            .to_string()?;
        let session = Session::new(session_name, SessionType::Bookmark(path));
        if let Some(list) = sessions.get_mut(&session.name) {
            list.push(session);
        } else {
            sessions.insert(session.name.clone(), vec![session]);
        }
    }

    Ok(sessions)
}

fn append_remote_projects(
    config: &Config,
    mut sessions: HashMap<String, Vec<Session>>,
) -> Result<HashMap<String, Vec<Session>>> {
    let remote_projects = find_remote_projects(config)?;
    let display_format = config
        .remote_display_format
        .clone()
        .unwrap_or_default();

    for project in remote_projects {
        let display_name = display_format.format(&project.project_name, &project.host_name);
        let session = Session::new(
            display_name.clone(),
            SessionType::Remote {
                host: project.host,
                remote_path: project.remote_path,
            },
        );
        if let Some(list) = sessions.get_mut(&display_name) {
            list.push(session);
        } else {
            sessions.insert(display_name, vec![session]);
        }
    }

    Ok(sessions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_session_name_deduplication() {
        let mut test_sessions = vec![
            Session::new(
                "test".into(),
                SessionType::Bookmark("/search/path/to/proj1/test".into()),
            ),
            Session::new(
                "test".into(),
                SessionType::Bookmark("/search/path/to/proj2/test".into()),
            ),
            Session::new(
                "test".into(),
                SessionType::Bookmark("/other/path/to/projects/proj2/test".into()),
            ),
        ];

        let deduplicated = deduplicate_sessions(&mut test_sessions);

        assert_eq!(deduplicated[0].name, "projects/proj2/test");
        assert_eq!(deduplicated[1].name, "to/proj2/test");
        assert_eq!(deduplicated[2].name, "to/proj1/test");
    }
}
