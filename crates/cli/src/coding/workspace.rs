//! Where a coding agent works: the folder the bot names, as it is. Lorca makes no worktree or
//! branch for it; the agent decides how to work there, as it would started in a terminal. In a
//! git repository Lorca notes the branch and the commit it starts from, so what its work changed
//! can be counted when it is done.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::model::AgentChanges;

/// New files whose lines count toward the changes, at most, and how big each may be.
const NEW_FILES: usize = 200;
const NEW_FILE_BYTES: u64 = 1024 * 1024;

/// Where an agent works.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Place {
    pub folder: PathBuf,
    /// The branch checked out there as it starts, in a repository.
    pub branch: Option<String>,
    /// The commit its work starts from, which its changes count from.
    pub base: Option<String>,
    /// The repository it is a clone of, from its `origin` remote: "github.com/egoist/lorca".
    pub repository: Option<String>,
}

/// The folder for an agent: `folder`, relative to `workdir` (the bot's working directory, where
/// it also defaults to), with its branch, commit, and repository when it is in one.
pub(crate) async fn prepare(workdir: &Path, folder: Option<&str>) -> Result<Place, String> {
    let folder = resolve(workdir, folder);
    if !folder.is_dir() {
        return Err(format!("{} is not a folder on this Runner", super::home_relative(&folder)));
    }
    let base = git(&folder, &["rev-parse", "HEAD"]).await.ok();
    let (branch, repository) = match base {
        Some(_) => (branch(&folder).await, repository(&folder).await),
        None => (None, None),
    };
    Ok(Place { folder, branch, base, repository })
}

/// The branch checked out in `folder`, none when it is not on one.
pub(crate) async fn branch(folder: &Path) -> Option<String> {
    git(folder, &["branch", "--show-current"]).await.ok().filter(|branch| !branch.is_empty())
}

/// The repository `folder` is a clone of, from its `origin` remote.
pub(crate) async fn repository(folder: &Path) -> Option<String> {
    git(folder, &["remote", "get-url", "origin"]).await.ok().and_then(|remote| crate::agent_roster::repository_of_remote(&remote))
}

/// `folder` as a path: `~/…`, absolute, or relative to `workdir`.
fn resolve(workdir: &Path, folder: Option<&str>) -> PathBuf {
    let Some(folder) = folder.map(str::trim).filter(|folder| !folder.is_empty()) else { return workdir.to_path_buf() };
    let path = if folder == "~" {
        dirs::home_dir().unwrap_or_default()
    } else if let Some(rest) = folder.strip_prefix("~/") {
        dirs::home_dir().unwrap_or_default().join(rest)
    } else {
        workdir.join(folder)
    };
    std::fs::canonicalize(&path).unwrap_or(path)
}

/// Runs git in `folder` and returns what it printed, trimmed.
pub(crate) async fn git(folder: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = lorca_agent::login_shell::command("git").await;
    command.arg("-C").arg(folder).args(args).stdin(std::process::Stdio::null());
    let output = command.output().await.map_err(|e| format!("git did not start: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let said = String::from_utf8_lossy(&output.stderr);
        Err(said.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("git failed").to_string())
    }
}

/// What the agent changed since `base`: commits, edits not committed yet, and new files. None
/// when git cannot tell.
pub(crate) async fn changes(folder: &Path, base: &str) -> Option<AgentChanges> {
    let numstat = git(folder, &["-c", "core.quotepath=off", "diff", "--numstat", base, "--"]).await.ok()?;
    let mut changes = self::numstat(&numstat);
    let untracked = git(folder, &["ls-files", "--others", "--exclude-standard"]).await.unwrap_or_default();
    for file in untracked.lines().filter(|file| !file.is_empty()).take(NEW_FILES) {
        changes.files += 1;
        let path = folder.join(file);
        if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() <= NEW_FILE_BYTES) {
            if let Ok(text) = std::fs::read_to_string(&path) {
                changes.added += text.lines().count() as u32;
            }
        }
    }
    Some(changes)
}

/// The files and lines `git diff --numstat` counts.
pub(crate) fn numstat(text: &str) -> AgentChanges {
    let mut changes = AgentChanges::default();
    for line in text.lines() {
        let mut fields = line.split('\t');
        let (Some(added), Some(removed)) = (fields.next(), fields.next()) else { continue };
        changes.files += 1;
        // A binary file counts as changed, with no lines.
        changes.added += added.parse::<u32>().unwrap_or(0);
        changes.removed += removed.parse::<u32>().unwrap_or(0);
    }
    changes
}

static PULL_REQUEST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https://(?:github\.com/[\w.-]+/[\w.-]+/pull/\d+|gitlab\.com/[\w./-]+/-/merge_requests/\d+)").unwrap());

/// The last pull request (or GitLab merge request) `text` names.
pub(crate) fn pull_request(text: &str) -> Option<String> {
    PULL_REQUEST.find_iter(text).last().map(|found| found.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transcript_names_its_pull_request() {
        assert_eq!(pull_request("Opened https://github.com/egoist/lorca/pull/141 for review.").as_deref(), Some("https://github.com/egoist/lorca/pull/141"));
        assert_eq!(pull_request("https://gitlab.com/acme/shop/-/merge_requests/7").as_deref(), Some("https://gitlab.com/acme/shop/-/merge_requests/7"));
        assert_eq!(pull_request("See https://github.com/egoist/lorca/issues/134"), None);
    }

    #[tokio::test]
    async fn an_agent_works_in_the_folder_itself_and_its_changes_count_from_where_it_started() {
        let scratch = std::env::temp_dir().join(format!("lorca-workspace-{}", uuid::Uuid::new_v4()));
        let repo = scratch.join("shop");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]).await.unwrap();
        std::fs::write(repo.join("README.md"), "shop\n").unwrap();
        git(&repo, &["add", "."]).await.unwrap();
        git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]).await.unwrap();

        let place = prepare(&scratch, Some("shop")).await.unwrap();
        assert_eq!((place.folder.clone(), place.branch.as_deref()), (std::fs::canonicalize(&repo).unwrap(), Some("main")));
        // Whatever the agent does there, a branch of its own included, counts from where it started.
        git(&repo, &["checkout", "-q", "-b", "fix-login"]).await.unwrap();
        std::fs::write(repo.join("README.md"), "shop\nlogin\n").unwrap();
        std::fs::write(repo.join("new.txt"), "new\ntwo\n").unwrap();
        let changes = changes(&place.folder, place.base.as_deref().unwrap()).await.unwrap();
        assert_eq!(changes, AgentChanges { files: 2, added: 3, removed: 0 });
        assert_eq!(branch(&place.folder).await.as_deref(), Some("fix-login"));
        assert_eq!(place.repository, None);
        git(&repo, &["remote", "add", "origin", "git@github.com:acme/shop.git"]).await.unwrap();
        assert_eq!(prepare(&scratch, Some("shop")).await.unwrap().repository.as_deref(), Some("github.com/acme/shop"));

        let plain = scratch.join("notes");
        std::fs::create_dir_all(&plain).unwrap();
        let place = prepare(&scratch, Some("notes")).await.unwrap();
        assert_eq!((place.folder, place.branch, place.base, place.repository), (std::fs::canonicalize(&plain).unwrap(), None, None, None));
        assert!(prepare(&scratch, Some("missing")).await.is_err());
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
