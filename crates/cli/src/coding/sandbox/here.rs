//! A sandbox that is a folder on this computer, for the tests: its commands run here through
//! `sh`, and a process's streams come through [`super::attach`] as a provider's do, so cloning,
//! the agent's process group, its stop, and the changes it made are those of a real sandbox.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{mpsc, oneshot, Mutex};

use super::{Chunk, Output, Remote};
use crate::app::App;
use crate::coding::process::{Exit, Process, Stdout};

/// How often each sandbox paused, by name: what a test reads.
static PAUSES: std::sync::Mutex<Vec<(String, usize)>> = std::sync::Mutex::new(Vec::new());
/// A folder whose programs every test sandbox has on its `PATH`: the agents' stand-ins.
static PROGRAMS: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Has every test sandbox find the programs in `folder` first.
pub(crate) fn install(folder: &std::path::Path) {
    *PROGRAMS.lock().unwrap() = Some(folder.to_path_buf());
}

pub(crate) fn pauses(name: &str) -> usize {
    PAUSES.lock().unwrap().iter().find(|(known, _)| known == name).map(|(_, count)| *count).unwrap_or(0)
}

pub(crate) struct Shell {
    name: String,
    root: PathBuf,
}

impl Shell {
    /// The sandbox `name`: a folder of its own under the Runner's data directory.
    pub fn open(app: &App, name: &str) -> Shell {
        let root = app.config.home.join("sandboxes").join(name);
        std::fs::create_dir_all(&root).unwrap();
        Shell { name: name.into(), root }
    }

    pub fn root(&self) -> String {
        self.root.to_string_lossy().to_string()
    }

    fn command(&self, argv: &[String], cwd: &str, env: &BTreeMap<String, String>) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&argv[0]);
        command.args(&argv[1..]).current_dir(cwd).envs(env).env("HOME", &self.root);
        if let Some(programs) = PROGRAMS.lock().unwrap().clone() {
            let path = std::env::var_os("PATH").unwrap_or_default();
            let paths = std::iter::once(programs).chain(std::env::split_paths(&path));
            command.env("PATH", std::env::join_paths(paths).unwrap());
        }
        command
    }

    pub async fn run(&self, argv: &[String], cwd: &str, env: &BTreeMap<String, String>) -> Result<Output, String> {
        let output = self.command(argv, cwd, env).stdin(std::process::Stdio::null()).output().await.map_err(|e| e.to_string())?;
        Ok(Output { code: output.status.code(), stdout: String::from_utf8_lossy(&output.stdout).into(), stderr: String::from_utf8_lossy(&output.stderr).into() })
    }

    pub fn spawn(&self, argv: &[String], cwd: &str, env: &BTreeMap<String, String>) -> Result<(Arc<Process>, Stdout, oneshot::Receiver<Exit>), String> {
        use std::process::Stdio;
        let mut child: Child = self.command(argv, cwd, env).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::unbounded_channel();
        let forward = |mut stream: Box<dyn tokio::io::AsyncRead + Send + Unpin>, tx: mpsc::UnboundedSender<Chunk>, out: bool| {
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 8192];
                while let Ok(read) = stream.read(&mut buffer).await {
                    if read == 0 {
                        break;
                    }
                    let bytes = buffer[..read].to_vec();
                    let _ = tx.send(if out { Chunk::Stdout(bytes) } else { Chunk::Stderr(bytes) });
                }
            })
        };
        let stdout = forward(Box::new(child.stdout.take().unwrap()), tx.clone(), true);
        let stderr = forward(Box::new(child.stderr.take().unwrap()), tx.clone(), false);
        let remote = Arc::new(Here { stdin: Mutex::new(child.stdin.take()), pid: child.id() });
        tokio::spawn(async move {
            let status = child.wait().await.ok();
            let _ = (stdout.await, stderr.await);
            let _ = tx.send(Chunk::Exit(status.and_then(|status| status.code())));
        });
        Ok(super::attach(remote, rx, "The test sandbox".into()))
    }

    pub fn pause(&self) {
        let mut pauses = PAUSES.lock().unwrap();
        match pauses.iter_mut().find(|(known, _)| *known == self.name) {
            Some((_, count)) => *count += 1,
            None => pauses.push((self.name.clone(), 1)),
        }
    }
}

struct Here {
    stdin: Mutex<Option<ChildStdin>>,
    pid: Option<u32>,
}

#[async_trait]
impl Remote for Here {
    async fn write(&self, bytes: Vec<u8>) -> Result<(), String> {
        let mut stdin = self.stdin.lock().await;
        let pipe = stdin.as_mut().ok_or("closed")?;
        pipe.write_all(&bytes).await.map_err(|e| e.to_string())?;
        pipe.flush().await.map_err(|e| e.to_string())
    }

    async fn kill(&self) {
        if let Some(pid) = self.pid {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }
    }
}
