//! Where a coding agent works. In a git repository that is a worktree of its own, so it never
//! edits the checkout the user works in: a new one on a branch of its own name, or the one a
//! bot names again, under `worktrees/` in Lorca's folder. A folder that is not a repository is
//! worked in as it is. The proof the bot expects goes in `.lorca-proof/` there, which ignores
//! itself in git.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The folder the agent saves its proof in, inside the folder it works in.
pub(crate) const PROOF_FOLDER: &str = ".lorca-proof";
/// The most diff an output carries.
const DIFF_BYTES: usize = 2 * 1024 * 1024;
/// New files a diff shows in full, at most, and how big each may be.
const NEW_FILES: usize = 50;
const NEW_FILE_BYTES: u64 = 256 * 1024;

/// Where an agent works.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Place {
    pub folder: PathBuf,
    /// The worktree's name and branch, in a repository.
    pub branch: Option<String>,
    /// The commit its work starts from, which its diff is against.
    pub base: Option<String>,
}

/// Prepares the folder for an agent: `folder` (relative to `workdir`, the bot's working
/// directory, where it also defaults to), and in a repository the worktree `worktree` names or
/// a new one named after `task`.
pub(crate) async fn prepare(home: &Path, workdir: &Path, folder: Option<&str>, worktree: Option<&str>, task: &str) -> Result<Place, String> {
    let folder = resolve(workdir, folder);
    if !folder.is_dir() {
        return Err(format!("{} is not a folder on this Runner", super::home_relative(&folder)));
    }
    let Some(repo) = git(&folder, &["rev-parse", "--show-toplevel"]).await.ok().map(PathBuf::from) else {
        if worktree.is_some() {
            return Err(format!("{} is not a git repository, so it has no worktrees. Leave worktree out to work in the folder itself.", super::home_relative(&folder)));
        }
        create_proof_folder(&folder)?;
        return Ok(Place { folder, branch: None, base: None });
    };
    let name = match worktree {
        Some(name) => slug(name).ok_or("A worktree name needs letters or digits")?,
        None => format!("{}-{}", slug(task).unwrap_or_else(|| "work".into()), &uuid::Uuid::new_v4().simple().to_string()[..4]),
    };
    let path = worktrees_folder(home, &repo).join(&name);
    let head = git(&repo, &["rev-parse", "HEAD"]).await.map_err(|error| format!("{} has no commit to start from: {error}", super::home_relative(&repo)))?;
    if path.exists() {
        // A worktree named again: it goes on from where it is.
        let top = git(&path, &["rev-parse", "--show-toplevel"]).await.map(PathBuf::from).ok();
        if top.as_deref().map(canonical) != Some(canonical(&path)) {
            return Err(format!("{} is in the way of the worktree {name}", super::home_relative(&path)));
        }
        let branch = git(&path, &["branch", "--show-current"]).await.ok().filter(|branch| !branch.is_empty());
        let base = git(&path, &["merge-base", "HEAD", &head]).await.ok();
        create_proof_folder(&path)?;
        return Ok(Place { folder: path, branch, base });
    }
    std::fs::create_dir_all(path.parent().unwrap_or(home)).map_err(|e| e.to_string())?;
    let target = path.to_string_lossy().to_string();
    let exists = git(&repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{name}")]).await.is_ok();
    let added = if exists { git(&repo, &["worktree", "add", &target, &name]).await } else { git(&repo, &["worktree", "add", "-b", &name, &target, &head]).await };
    added.map_err(|error| format!("Could not make the worktree {name}: {error}"))?;
    let base = git(&path, &["rev-parse", "HEAD"]).await.ok();
    create_proof_folder(&path)?;
    Ok(Place { folder: path, branch: Some(name), base })
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

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The folder a repository's worktrees go in: `worktrees/<name>-<hash>` in Lorca's folder, so two
/// repositories of one name keep theirs apart.
fn worktrees_folder(home: &Path, repo: &Path) -> PathBuf {
    let hash = Sha256::digest(canonical(repo).to_string_lossy().as_bytes());
    let name = repo.file_name().and_then(|name| name.to_str()).and_then(slug).unwrap_or_else(|| "repo".into());
    home.join("worktrees").join(format!("{name}-{}", hex(&hash[..3])))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A worktree and branch name from free text: lowercase words joined by `-`, at most four words
/// and 40 characters.
pub(crate) fn slug(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(4)
        .map(|word| word.to_ascii_lowercase())
        .collect();
    let mut slug = words.join("-");
    slug.truncate(40);
    let slug = slug.trim_matches('-').to_string();
    (!slug.is_empty()).then_some(slug)
}

/// The proof folder, which ignores itself and everything in it.
fn create_proof_folder(folder: &Path) -> Result<(), String> {
    let proof = folder.join(PROOF_FOLDER);
    std::fs::create_dir_all(&proof).map_err(|e| format!("Could not make {}: {e}", super::home_relative(&proof)))?;
    let ignore = proof.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, "*\n").map_err(|e| e.to_string())?;
    }
    Ok(())
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

/// Everything the agent changed since `base`: commits, edits not committed yet, and new files.
/// Empty when nothing changed.
pub(crate) async fn diff(folder: &Path, base: &str) -> String {
    let mut command = lorca_agent::login_shell::command("git").await;
    command.arg("-C").arg(folder).args(["-c", "core.quotepath=off", "diff", base, "--"]).stdin(std::process::Stdio::null());
    let mut diff = match command.output().await {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout).to_string(),
        _ => String::new(),
    };
    let untracked = git(folder, &["ls-files", "--others", "--exclude-standard"]).await.unwrap_or_default();
    for file in untracked.lines().filter(|file| !file.is_empty()).take(NEW_FILES) {
        let path = folder.join(file);
        if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > NEW_FILE_BYTES) {
            diff.push_str(&format!("diff --git a/{file} b/{file}\nnew file (too large to show)\n"));
            continue;
        }
        let mut command = lorca_agent::login_shell::command("git").await;
        // Git takes `/dev/null` as the empty side on every platform.
        command.arg("-C").arg(folder).args(["diff", "--no-index", "--", "/dev/null", file]).stdin(std::process::Stdio::null());
        // `git diff --no-index` exits 1 when the files differ, as a new one does.
        if let Ok(output) = command.output().await {
            diff.push_str(&String::from_utf8_lossy(&output.stdout));
        }
        if diff.len() > DIFF_BYTES {
            break;
        }
    }
    if diff.len() > DIFF_BYTES {
        let mut cut = DIFF_BYTES;
        while !diff.is_char_boundary(cut) {
            cut -= 1;
        }
        diff.truncate(cut);
        diff.push_str("\n… (the diff is longer; read the rest in the worktree)\n");
    }
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_the_task() {
        assert_eq!(slug("Fix the login bug on Safari").as_deref(), Some("fix-the-login-bug"));
        assert_eq!(slug("  ¿¡ "), None);
        assert_eq!(slug("feature/Login_page").as_deref(), Some("feature-login-page"));
    }

    #[tokio::test]
    async fn a_repository_gets_a_worktree_and_a_folder_is_used_as_it_is() {
        let scratch = std::env::temp_dir().join(format!("lorca-worktree-{}", uuid::Uuid::new_v4()));
        let repo = scratch.join("shop");
        std::fs::create_dir_all(&repo).unwrap();
        let home = scratch.join("home");
        git(&repo, &["init", "-q"]).await.unwrap();
        std::fs::write(repo.join("README.md"), "shop\n").unwrap();
        git(&repo, &["add", "."]).await.unwrap();
        git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]).await.unwrap();

        let place = prepare(&home, &scratch, Some("shop"), Some("Fix login"), "anything").await.unwrap();
        assert_eq!(place.branch.as_deref(), Some("fix-login"));
        assert!(place.folder.starts_with(home.join("worktrees")), "{}", place.folder.display());
        assert!(place.folder.join("README.md").exists());
        assert!(place.folder.join(PROOF_FOLDER).is_dir());
        assert_eq!(git(&place.folder, &["status", "--porcelain"]).await.unwrap(), "", "the proof folder ignores itself");

        // Named again, it is the same worktree, and the diff shows what changed since.
        std::fs::write(place.folder.join("README.md"), "shop\nlogin\n").unwrap();
        std::fs::write(place.folder.join("new.txt"), "new\n").unwrap();
        let again = prepare(&home, &scratch, Some(repo.to_str().unwrap()), Some("fix-login"), "anything").await.unwrap();
        assert_eq!(again.folder, place.folder);
        let diff = diff(&again.folder, again.base.as_deref().unwrap()).await;
        assert!(diff.contains("+login") && diff.contains("+new"), "{diff}");

        // A new one without a name takes the task's words.
        let fresh = prepare(&home, &scratch, Some("shop"), None, "Add dark mode").await.unwrap();
        assert!(fresh.branch.as_deref().unwrap().starts_with("add-dark-mode-"));

        let plain = scratch.join("notes");
        std::fs::create_dir_all(&plain).unwrap();
        let place = prepare(&home, &scratch, Some("notes"), None, "Tidy").await.unwrap();
        assert_eq!((place.folder, place.branch), (std::fs::canonicalize(&plain).unwrap(), None));
        assert!(prepare(&home, &scratch, Some("notes"), Some("x"), "Tidy").await.is_err());
        assert!(prepare(&home, &scratch, Some("missing"), None, "Tidy").await.is_err());
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
