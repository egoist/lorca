//! A bot's working folder as the apps show it beside its chat: the files in it a folder at a time,
//! and, in a git repository, what changed there and each file's diff. Read on the bot's Runner,
//! here for the bots this Device runs and through a sealed request for the others (`requests.rs`).

use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use crate::app::App;
use crate::model::Bot;

/// A folder's entries, at most; the rest are counted.
const ENTRIES: usize = 500;
/// Changed files, at most; the rest are counted.
const CHANGED_FILES: usize = 300;
/// A new file's lines are counted up to this size.
const NEW_FILE_BYTES: u64 = 1024 * 1024;
/// A diff past this is cut.
const DIFF_BYTES: usize = 512 * 1024;
/// The tree a repository with no commit yet compares with.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Debug, Serialize, PartialEq)]
pub struct Entry {
    pub name: String,
    /// From the working folder, with `/` between its parts.
    pub path: String,
    pub dir: bool,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct ChangedFile {
    /// From the working folder, with `/` between its parts.
    pub path: String,
    /// `modified`, `added`, `deleted`, or `untracked`.
    pub status: String,
    pub added: u32,
    pub removed: u32,
}

/// Answers `workdir.files`, `workdir.changes`, and `workdir.diff` for a bot that runs here.
pub async fn serve(app: &App, verb: &str, body: &Value) -> Result<Value, String> {
    let bot_id = body["bot_id"].as_str().ok_or("missing bot_id")?;
    let bot = app.bot(bot_id).ok_or("Unknown bot")?;
    if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
        let runner = app.device(&bot.runner_id).map(|d| d.name).unwrap_or_else(|| "another Runner".into());
        return Err(format!("{} runs on {runner}, not here.", bot.name));
    }
    let root = folder(app, &bot)?;
    let path = body["path"].as_str().unwrap_or("");
    match verb {
        "workdir.files" => {
            let (dir, path) = (root.clone(), path.to_string());
            let (entries, more) = tokio::task::spawn_blocking(move || list(&dir, &path)).await.map_err(|e| e.to_string())??;
            Ok(json!({ "folder": crate::coding::home_relative(&root), "path": root, "entries": entries, "more": more }))
        }
        "workdir.changes" => Ok(match changes(&root).await? {
            Some((branch, files, more)) => json!({ "repository": true, "branch": branch, "files": files, "more": more }),
            None => json!({ "repository": false }),
        }),
        "workdir.diff" => {
            let (diff, truncated) = diff(&root, path).await?;
            Ok(json!({ "diff": diff, "truncated": truncated }))
        }
        other => Err(format!("Unknown request {other}")),
    }
}

/// The bot's working folder, made when it is the default one and not there yet.
fn folder(app: &App, bot: &Bot) -> Result<PathBuf, String> {
    let root = bot.working_directory(&app.config.home);
    if bot.workdir.is_none() {
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    }
    if !root.is_dir() {
        return Err(format!("{} is not a folder on this Runner", crate::coding::home_relative(&root)));
    }
    Ok(std::fs::canonicalize(&root).unwrap_or(root))
}

/// `path` under `root`, refused when it is absolute or climbs out with `..`.
fn inside(root: &Path, path: &str) -> Result<PathBuf, String> {
    let relative = Path::new(path);
    if relative.components().any(|part| !matches!(part, Component::Normal(_) | Component::CurDir)) {
        return Err(format!("{path} is not in the working folder"));
    }
    Ok(root.join(relative))
}

/// The entries of the folder at `path` under `root`, folders first, and how many more there are.
/// `.git` is left out.
fn list(root: &Path, path: &str) -> Result<(Vec<Entry>, usize), String> {
    let dir = inside(root, path)?;
    let read = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", crate::coding::home_relative(&dir)))?;
    let mut entries: Vec<Entry> = read
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name() != ".git")
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            // A link to a folder opens as one.
            let dir = std::fs::metadata(entry.path()).is_ok_and(|metadata| metadata.is_dir());
            let path = if path.trim_matches('/').is_empty() { name.clone() } else { format!("{}/{name}", path.trim_matches('/')) };
            Entry { name, path, dir }
        })
        .collect();
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())).then_with(|| a.name.cmp(&b.name)));
    let more = entries.len().saturating_sub(ENTRIES);
    entries.truncate(ENTRIES);
    Ok((entries, more))
}

/// Runs git in `folder` and returns what it printed as it is. An exit status in `ok` besides 0
/// is no failure.
async fn git(folder: &Path, args: &[&str], ok: &[i32]) -> Result<Vec<u8>, String> {
    let mut command = lorca_agent::login_shell::command("git").await;
    command.arg("-C").arg(folder).args(["-c", "core.quotepath=off"]).args(args).stdin(std::process::Stdio::null());
    let output = command.output().await.map_err(|e| format!("git did not start: {e}"))?;
    if output.status.success() || output.status.code().is_some_and(|code| ok.contains(&code)) {
        Ok(output.stdout)
    } else {
        let said = String::from_utf8_lossy(&output.stderr);
        Err(said.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("git failed").to_string())
    }
}

/// What the last commit compares with: HEAD, or the empty tree before the first commit.
async fn base(root: &Path) -> &'static str {
    if git(root, &["rev-parse", "--verify", "-q", "HEAD"], &[]).await.is_ok() { "HEAD" } else { EMPTY_TREE }
}

/// What changed in the working folder since the last commit: the branch, the files, and how many
/// more there are. None outside a git repository.
async fn changes(root: &Path) -> Result<Option<(Option<String>, Vec<ChangedFile>, usize)>, String> {
    if git(root, &["rev-parse", "--is-inside-work-tree"], &[]).await.is_err() {
        return Ok(None);
    }
    let base = base(root).await;
    let status = git(root, &["status", "--porcelain=v1", "-z", "--no-renames", "--untracked-files=all", "--", "."], &[]).await?;
    let numstat = git(root, &["diff", "--numstat", "-z", "--no-renames", "--relative", base, "--"], &[]).await?;
    // Status names files from the repository's top; the folder may be inside it.
    let prefix = String::from_utf8_lossy(&git(root, &["rev-parse", "--show-prefix"], &[]).await?).trim().to_string();
    let mut counts = std::collections::HashMap::new();
    for record in numstat.split(|byte| *byte == 0).filter(|record| !record.is_empty()) {
        let record = String::from_utf8_lossy(record);
        let mut fields = record.splitn(3, '\t');
        if let (Some(added), Some(removed), Some(path)) = (fields.next(), fields.next(), fields.next()) {
            // A binary file counts as changed, with no lines.
            counts.insert(path.to_string(), (added.parse().unwrap_or(0), removed.parse().unwrap_or(0)));
        }
    }
    let mut files = Vec::new();
    for record in status.split(|byte| *byte == 0).filter(|record| record.len() > 3) {
        let record = String::from_utf8_lossy(record);
        let (code, path) = record.split_at(3);
        let path = path.strip_prefix(prefix.as_str()).unwrap_or(path).to_string();
        let code = code.trim();
        let status = match code {
            "??" => "untracked",
            _ if code.contains('D') => "deleted",
            _ if code.contains('A') => "added",
            _ => "modified",
        };
        let (added, removed) = match status {
            "untracked" => (lines_in(&root.join(&path)), 0),
            _ => counts.get(&path).copied().unwrap_or((0, 0)),
        };
        files.push(ChangedFile { path, status: status.into(), added, removed });
    }
    let more = files.len().saturating_sub(CHANGED_FILES);
    files.truncate(CHANGED_FILES);
    Ok(Some((crate::coding::workspace::branch(root).await, files, more)))
}

/// The lines of a new text file, none for a large or binary one.
fn lines_in(path: &Path) -> u32 {
    if !std::fs::metadata(path).is_ok_and(|metadata| metadata.len() <= NEW_FILE_BYTES) {
        return 0;
    }
    std::fs::read_to_string(path).map(|text| text.lines().count() as u32).unwrap_or(0)
}

/// The diff of one file since the last commit, as git writes it, cut past `DIFF_BYTES`. A file
/// git does not track yet is all new lines.
async fn diff(root: &Path, path: &str) -> Result<(String, bool), String> {
    let file = inside(root, path)?;
    if path.trim().is_empty() {
        return Err("missing path".into());
    }
    let base = base(root).await;
    let mut text = String::from_utf8_lossy(&git(root, &["diff", "--no-renames", "--relative", base, "--", path], &[]).await?).into_owned();
    if text.is_empty() && file.is_file() {
        text = new_file_diff(path, &file);
    }
    let truncated = text.len() > DIFF_BYTES;
    if truncated {
        let mut cut = DIFF_BYTES;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(text[..cut].rfind('\n').map_or(cut, |line| line + 1));
    }
    Ok((text, truncated))
}

/// A file git does not track yet, as a diff that adds every line.
fn new_file_diff(path: &str, file: &Path) -> String {
    let bytes = std::fs::read(file).unwrap_or_default();
    let header = format!("--- /dev/null\n+++ b/{path}\n");
    if bytes.contains(&0) {
        return format!("{header}Binary file\n");
    }
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let mut diff = format!("{header}@@ -0,0 +1,{} @@\n", lines.len());
    for line in lines {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn run(repo: &Path, args: &[&str]) {
        git(repo, args, &[]).await.unwrap();
    }

    #[test]
    fn a_folder_lists_folders_first_and_never_leaves_the_working_folder() {
        let root = std::env::temp_dir().join(format!("lorca-workdir-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("src/nested")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("README.md"), "hi").unwrap();
        std::fs::write(root.join("b.txt"), "b").unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();

        let (entries, more) = list(&root, "").unwrap();
        let names: Vec<_> = entries.iter().map(|entry| (entry.path.as_str(), entry.dir)).collect();
        assert_eq!(names, [("src", true), ("b.txt", false), ("README.md", false)]);
        assert_eq!(more, 0);
        let (entries, _) = list(&root, "src").unwrap();
        assert_eq!(entries.iter().map(|entry| entry.path.as_str()).collect::<Vec<_>>(), ["src/nested", "src/main.rs"]);

        assert!(list(&root, "../").is_err());
        assert!(list(&root, "src/../..").is_err());
        assert!(list(&root, "/etc").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn the_app_reads_the_folder_the_user_picked_for_the_bot() {
        let home = std::env::temp_dir().join(format!("lorca-workdir-app-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        crate::identity::create(&app, Some("Workbench".into())).unwrap();
        let runner_id = app.this_device_id().unwrap();
        let out = crate::api::dispatch(&app, "bots.create", json!({ "name": "Shop", "runner_id": runner_id })).await.unwrap();
        let bot_id = out["bot"]["id"].as_str().unwrap().to_string();

        // Its own workspace until the user picks a folder; the default is made when it is read.
        let files = crate::api::dispatch(&app, "bots.workdir.files", json!({ "bot_id": bot_id })).await.unwrap();
        assert_eq!((files["here"].as_bool(), files["entries"].as_array().map(Vec::len)), (Some(true), Some(0)));

        let project = home.join("shop");
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::write(project.join("src/lib.rs"), "").unwrap();
        crate::api::dispatch(&app, "bots.update", json!({ "id": bot_id, "workdir": project.display().to_string() })).await.unwrap();
        let files = crate::api::dispatch(&app, "bots.workdir.files", json!({ "bot_id": bot_id, "path": "src" })).await.unwrap();
        assert_eq!(files["entries"], json!([{ "name": "lib.rs", "path": "src/lib.rs", "dir": false }]));
        assert_eq!(files["path"], json!(std::fs::canonicalize(&project).unwrap()));
        let changes = crate::api::dispatch(&app, "bots.workdir.changes", json!({ "bot_id": bot_id })).await.unwrap();
        assert_eq!(changes["repository"], json!(false));

        // A folder that is gone says so, and an empty one goes back to the default.
        crate::api::dispatch(&app, "bots.update", json!({ "id": bot_id, "workdir": home.join("missing").display().to_string() })).await.unwrap();
        assert!(crate::api::dispatch(&app, "bots.workdir.files", json!({ "bot_id": bot_id })).await.unwrap_err().contains("is not a folder"));
        crate::api::dispatch(&app, "bots.update", json!({ "id": bot_id, "workdir": "" })).await.unwrap();
        assert_eq!(app.bot(&bot_id).unwrap().workdir, None);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn a_repository_reports_its_changes_and_each_files_diff() {
        let scratch = std::env::temp_dir().join(format!("lorca-workdir-{}", uuid::Uuid::new_v4()));
        let repo = scratch.join("shop");
        std::fs::create_dir_all(repo.join("web")).unwrap();
        let repo = std::fs::canonicalize(&repo).unwrap();
        assert_eq!(changes(&repo).await.unwrap(), None);

        run(&repo, &["init", "-q", "-b", "main"]).await;
        // Before the first commit everything is new.
        std::fs::write(repo.join("README.md"), "shop\n").unwrap();
        let (_, files, _) = changes(&repo).await.unwrap().unwrap();
        assert_eq!(files, [ChangedFile { path: "README.md".into(), status: "untracked".into(), added: 1, removed: 0 }]);
        assert!(diff(&repo, "README.md").await.unwrap().0.contains("+shop"));

        std::fs::write(repo.join("web/app.js"), "one\ntwo\n").unwrap();
        std::fs::write(repo.join("gone.txt"), "bye\n").unwrap();
        run(&repo, &["add", "."]).await;
        run(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]).await;
        assert_eq!(changes(&repo).await.unwrap(), Some((Some("main".into()), vec![], 0)));

        std::fs::write(repo.join("web/app.js"), "one\n2\nthree\n").unwrap();
        std::fs::remove_file(repo.join("gone.txt")).unwrap();
        std::fs::write(repo.join("web/new.js"), "a\nb\nc\n").unwrap();
        std::fs::write(repo.join("staged.txt"), "s\n").unwrap();
        run(&repo, &["add", "staged.txt"]).await;
        let (branch, mut files, _) = changes(&repo).await.unwrap().unwrap();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(branch.as_deref(), Some("main"));
        let got: Vec<_> = files.iter().map(|f| (f.path.as_str(), f.status.as_str(), f.added, f.removed)).collect();
        assert_eq!(got, [("gone.txt", "deleted", 0, 1), ("staged.txt", "added", 1, 0), ("web/app.js", "modified", 2, 1), ("web/new.js", "untracked", 3, 0)]);

        let (text, truncated) = diff(&repo, "web/app.js").await.unwrap();
        assert!(!truncated);
        assert!(text.contains("-two\n") && text.contains("+2\n") && text.contains("+three\n"), "{text}");
        assert!(diff(&repo, "web/new.js").await.unwrap().0.ends_with("+a\n+b\n+c\n"));
        assert!(diff(&repo, "../outside").await.is_err());

        // A working folder inside the repository sees its own part, named from itself.
        let web = repo.join("web");
        let (_, files, _) = changes(&web).await.unwrap().unwrap();
        let mut paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        paths.sort();
        assert_eq!(paths, ["app.js", "new.js"]);
        assert!(diff(&web, "app.js").await.unwrap().0.contains("+three"));
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
