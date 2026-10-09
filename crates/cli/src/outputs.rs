//! Bot deliverables and completion evidence. Metadata is part of an encrypted chat message;
//! files use the account's encrypted attachment transport. Each version is a new message.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::model::{Author, Body, Message};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Output {
    /// Stable across versions. A message id identifies one immutable version.
    pub id: String,
    pub name: String,
    pub mime: String,
    pub bot_id: String,
    pub chat_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<OutputEvidence>,
}

/// Shared artifact reference for durable tasks, handoffs and project context.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputReference {
    pub chat_id: String,
    pub message_id: String,
    pub output_id: String,
    pub version: u32,
}

impl Output {
    pub fn reference(&self, message_id: &str) -> OutputReference {
        OutputReference { chat_id: self.chat_id.clone(), message_id: message_id.into(), output_id: self.id.clone(), version: self.version }
    }

    /// The canonical TaskEvidence reference shape. This module does not mutate task state.
    pub fn task_evidence(&self, message_id: &str) -> serde_json::Value {
        let mut reference = serde_json::to_value(self.reference(message_id)).unwrap();
        reference["kind"] = serde_json::json!("output");
        reference["label"] = serde_json::json!(self.name);
        reference
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputEvidence {
    pub kind: EvidenceKind,
    pub summary: String,
    pub status: EvidenceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    TestResult,
    BeforeScreenshot,
    AfterScreenshot,
    Verification,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Passed,
    Failed,
    Unverified,
}

/// Publication shares an existing file or document reference with paired Devices. Creating,
/// uploading or publishing on an external service runs through the reviewed plugin/shell tools.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishOutput {
    pub name: String,
    pub path: Option<String>,
    pub url: Option<String>,
    pub mime: Option<String>,
    pub task_id: Option<String>,
    /// The previous version's message id, in this chat and produced by this bot.
    pub replaces: Option<String>,
    pub evidence: Option<OutputEvidence>,
}

/// Output-version messages in transcript order, including evidence. `task_id` is the canonical
/// task's reference; this module creates no task records and never marks a task complete.
pub fn list(app: &App, chat_id: &str, task_id: Option<&str>) -> Result<Vec<Message>, String> {
    app.chat(chat_id).ok_or("Unknown chat")?;
    app.store.outputs(chat_id, task_id).map_err(|error| error.to_string())
}

/// Publishes on the bot's assigned Runner. Bound runtime context supplies bot/chat/workdir;
/// a model cannot select another author or another chat. Relative paths use the bot's workdir.
pub fn publish(app: &App, chat_id: &str, bot_id: &str, workdir: &Path, request: PublishOutput) -> Result<Message, String> {
    let _publication = app.output_publication.lock().unwrap();
    app.dek().ok_or("Pair this Device before publishing outputs")?;
    let bot = app.bot(bot_id).ok_or("Unknown producing bot")?;
    if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
        return Err("Publish files on the producing bot's assigned Runner".into());
    }
    let chat = app.chat(chat_id).ok_or("Unknown chat")?;
    if !chat.meta.bot_ids.iter().any(|id| id == bot_id) {
        return Err("The producing bot is not in this chat".into());
    }
    let name = bounded(&request.name, "name", 256)?;
    if request.path.is_some() == request.url.is_some() {
        return Err("Provide exactly one path or HTTPS document url".into());
    }
    let mut task_id = request.task_id.as_deref().map(|id| bounded(id, "task_id", 256)).transpose()?;
    if task_id.as_ref().is_some_and(|id| id.strip_prefix("task-").and_then(|uuid| uuid::Uuid::parse_str(uuid).ok()).is_none()) {
        return Err("task_id must reference a canonical task-UUID".into());
    }
    let url = request.url.as_deref().map(document_url).transpose()?;
    if let Some(evidence) = &request.evidence {
        bounded(&evidence.summary, "evidence summary", 4000)?;
        if let Some(command) = &evidence.command {
            bounded(command, "evidence command", 8000)?;
        }
        if matches!((evidence.status, evidence.exit_code), (EvidenceStatus::Passed, Some(code)) if code != 0)
            || matches!((evidence.status, evidence.exit_code), (EvidenceStatus::Failed, Some(0)))
        {
            return Err("Evidence status conflicts with its exit code".into());
        }
    }
    let previous = match &request.replaces {
        Some(id) => {
            let message = app.message(chat_id, id).ok_or("Previous output version is unavailable")?;
            let output = message.output.ok_or("replaces must name an output message")?;
            if output.bot_id != bot_id || output.chat_id != chat_id || (task_id.is_some() && output.task_id != task_id) {
                return Err("An output version keeps its producing bot, chat and task".into());
            }
            task_id = output.task_id.clone();
            if list(app, chat_id, None)?.iter().any(|message| message.output.as_ref().is_some_and(|newer| newer.id == output.id && newer.version > output.version)) {
                return Err("replaces must name the latest output version".into());
            }
            Some(output)
        }
        None => None,
    };
    let version = previous.as_ref().map_or(Ok(1), |output| output.version.checked_add(1).ok_or("Output version limit reached"))?;
    let mime = request.mime.as_deref().map(|mime| bounded(mime, "mime", 128)).transpose()?;
    if mime.as_ref().is_some_and(|mime| !mime.contains('/') || !mime.is_ascii() || mime.chars().any(char::is_whitespace)) {
        return Err("mime must be a media type such as image/png".into());
    }
    let attachments = match &request.path {
        Some(path) => {
            let path = bounded(path, "path", 8192)?;
            let path = Path::new(&path);
            let source = if path.is_absolute() { path.to_path_buf() } else { workdir.join(path) };
            let attachment = crate::files::store(
                app,
                &crate::files::OutgoingFile { path: source.to_string_lossy().into_owned(), id: None, name: Some(name.clone()), mime: mime.clone(), width: None, height: None },
            )
            .map_err(|error| format!("Output file unavailable: {error}"))?;
            if request.evidence.as_ref().is_some_and(|e| matches!(e.kind, EvidenceKind::BeforeScreenshot | EvidenceKind::AfterScreenshot))
                && (!attachment.is_image() || attachment.width.unwrap_or(0) == 0 || attachment.height.unwrap_or(0) == 0)
            {
                let _ = std::fs::remove_file(crate::files::local_path(app, &attachment.id));
                return Err("Screenshot evidence requires a PNG, JPEG, GIF or WebP image file".into());
            }
            crate::files::push_blob(app, Some(chat_id), &attachment).map_err(|error| error.to_string())?;
            vec![attachment]
        }
        None => Vec::new(),
    };
    let output = Output {
        id: previous.as_ref().map(|output| output.id.clone()).unwrap_or_else(|| format!("out-{}", uuid::Uuid::new_v4())),
        name,
        mime: attachments.first().map(|attachment| attachment.mime.clone()).or(mime).unwrap_or_else(|| "text/html".into()),
        bot_id: bot_id.into(),
        chat_id: chat_id.into(),
        task_id,
        version,
        previous_message_id: request.replaces,
        url,
        evidence: request.evidence,
    };
    let text = message_text(&output);
    let mut message = Message::new(chat_id, Author::Bot { bot_id: bot_id.into() }, Body::Text { text, attachments, mentions: Vec::new(), reply_to: None });
    message.output = Some(output);
    app.upsert_message(message.clone(), true);
    if app.message(chat_id, &message.id).is_none() {
        return Err("Output could not be stored; the chat may have been deleted".into());
    }
    Ok(message)
}

/// What every app shows for the output in the transcript: the link, and what the bot checked. A
/// file is its attachment; the bot and the version are the message's own.
fn message_text(output: &Output) -> String {
    let mut lines = Vec::new();
    if let Some(url) = &output.url {
        lines.push(format!("[{}]({})", markdown_words(&output.name), url.replace('(', "%28").replace(')', "%29")));
    }
    if let Some(evidence) = &output.evidence {
        let kind = match evidence.kind {
            EvidenceKind::TestResult => "Test result",
            EvidenceKind::BeforeScreenshot => "Before screenshot",
            EvidenceKind::AfterScreenshot => "After screenshot",
            EvidenceKind::Verification => "Check",
        };
        let status = match evidence.status {
            EvidenceStatus::Passed => "Passed",
            EvidenceStatus::Failed => "Failed",
            EvidenceStatus::Unverified => "Not verified",
        };
        lines.push(format!("{kind} · {status}: {}", markdown_words(&evidence.summary)));
        if let Some(command) = &evidence.command {
            lines.push(code_span(command));
        }
    }
    lines.join("\n")
}

/// `text` as inline code, fenced by more backticks than it holds in a row.
fn code_span(text: &str) -> String {
    let text = text.replace(['\n', '\r'], " ");
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') { " " } else { "" };
    format!("{fence}{pad}{text}{pad}{fence}")
}

fn bounded(value: &str, field: &str, max: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > max || value.contains('\0') {
        return Err(format!("{field} must contain 1–{max} characters"));
    }
    Ok(value.to_string())
}

fn document_url(value: &str) -> Result<String, String> {
    let value = bounded(value, "url", 8192)?;
    let url = reqwest::Url::parse(&value).map_err(|_| "Invalid document URL")?;
    if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        return Err("Document links must use HTTPS without embedded credentials".into());
    }
    Ok(url.to_string())
}

fn markdown_words(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| {
            if "\\`*_{}[]<>()#!|".contains(c) {
                vec!['\\', c]
            } else if c == '\n' || c == '\r' {
                vec![' ']
            } else {
                vec![c]
            }
        })
        .collect()
}

#[cfg(feature = "runner")]
pub struct PublishOutputTool {
    pub app: std::sync::Arc<App>,
    pub chat_id: String,
    pub bot: crate::model::Bot,
    pub workdir: std::path::PathBuf,
}

#[cfg(feature = "runner")]
#[async_trait::async_trait]
impl lorca_agent::Tool for PublishOutputTool {
    fn name(&self) -> &str {
        "publish_output"
    }
    fn description(&self) -> &str {
        "Share a generated file or an existing HTTPS document link as an output in this chat, available to every paired Device. \
         Files are copied and encrypted; paths resolve in your working directory. Supply the canonical task_id when known. \
         Attach test logs/results and before/after screenshots as evidence; describe what was actually verified and any failure. \
         Use replaces (a previous output message_id) to preserve a new version. This records a link; creating or publishing on an \
         external service uses that service's tools and their authorization."
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object", "required": ["name"], "additionalProperties": false,
            "properties": {
                "name": {"type": "string"}, "path": {"type": "string"}, "url": {"type": "string"},
                "mime": {"type": "string"}, "task_id": {"type": "string"}, "replaces": {"type": "string"},
                "evidence": {
                    "type": "object", "required": ["kind", "summary", "status"], "additionalProperties": false,
                    "properties": {
                        "kind": {"type": "string", "enum": ["test_result", "before_screenshot", "after_screenshot", "verification"]},
                        "summary": {"type": "string"}, "status": {"type": "string", "enum": ["passed", "failed", "unverified"]},
                        "command": {"type": "string"}, "exit_code": {"type": "integer"}
                    }
                }
            }
        })
    }
    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({"type": "object", "required": ["message_id", "output", "task_evidence"], "properties": {
            "summary": {"type": "string"}, "message_id": {"type": "string"},
            "output": {"type": "object"}, "task_evidence": {"type": "object"}
        }}))
    }
    async fn execute(
        &self,
        _id: &str,
        args: serde_json::Value,
        cancel: tokio_util::sync::CancellationToken,
        _on_update: lorca_agent::ToolUpdateFn,
    ) -> Result<lorca_agent::ToolResult, lorca_agent::ToolError> {
        if cancel.is_cancelled() {
            return Err(lorca_agent::ToolError("Stopped".into()));
        }
        let request: PublishOutput = serde_json::from_value(args).map_err(|error| lorca_agent::ToolError(format!("Invalid output: {error}")))?;
        // Publishing a file reads it on the Runner, which the bot's Access may not allow.
        if request.path.is_some() {
            if let Err(denied) = crate::permissions::check_file_read(&self.app, &self.bot, self.name()) {
                let refused = crate::permissions::refuse(&self.app, &self.chat_id, &self.bot, denied);
                return Err(lorca_agent::ToolError(refused.reason.unwrap_or_default()));
            }
        }
        let app = self.app.clone();
        let chat_id = self.chat_id.clone();
        let bot_id = self.bot.id.clone();
        let workdir = self.workdir.clone();
        let message = tokio::task::spawn_blocking(move || {
            if cancel.is_cancelled() {
                return Err("Stopped".into());
            }
            publish(&app, &chat_id, &bot_id, &workdir, request)
        })
        .await
        .map_err(|error| lorca_agent::ToolError(error.to_string()))?
        .map_err(lorca_agent::ToolError)?;
        let output = message.output.as_ref().unwrap();
        let task_evidence = output.task_evidence(&message.id);
        let details = serde_json::json!({"summary": format!("Published {}", output.name), "message_id": message.id, "output": output, "task_evidence": task_evidence});
        let mut result =
            lorca_agent::ToolResult::text(format!("Published {} (v{}). Task evidence reference: {}", output.name, output.version, task_evidence)).with_details(details.clone());
        result.structured = Some(details);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    struct Fixture {
        app: Arc<App>,
        home: std::path::PathBuf,
        chat: String,
        bot: String,
        workdir: std::path::PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let home = std::env::temp_dir().join(format!("lorca-outputs-{}", uuid::Uuid::new_v4()));
            let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
            crate::identity::create(&app, Some("Output Runner".into())).unwrap();
            let bot = app.state.lock().unwrap().bots[0].clone();
            let chat = app.state.lock().unwrap().chats[0].meta.id.clone();
            let workdir = bot.working_directory(&home);
            std::fs::create_dir_all(&workdir).unwrap();
            Fixture { app, home, chat, bot: bot.id, workdir }
        }
        fn file(&self, text: &[u8]) -> PublishOutput {
            std::fs::write(self.workdir.join("report.txt"), text).unwrap();
            PublishOutput { name: "Report.txt".into(), path: Some("report.txt".into()), ..Default::default() }
        }
        fn publish(&self, request: PublishOutput) -> Result<Message, String> {
            publish(&self.app, &self.chat, &self.bot, &self.workdir, request)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }
    fn attachment(message: &Message) -> &crate::model::Attachment {
        let Body::Text { attachments, .. } = &message.body else { panic!("text output") };
        &attachments[0]
    }

    #[test]
    fn files_and_metadata_are_encrypted_and_versions_survive_source_changes_and_restart() {
        let f = Fixture::new();
        let task_id = format!("task-{}", uuid::Uuid::new_v4());
        let mut request = f.file(b"first private report");
        request.task_id = Some(task_id.clone());
        request.evidence = Some(OutputEvidence {
            kind: EvidenceKind::TestResult,
            summary: "All checks pass".into(),
            status: EvidenceStatus::Passed,
            command: Some("cargo test".into()),
            exit_code: Some(0),
        });
        let first = f.publish(request).unwrap();
        let first_attachment = attachment(&first);
        let Body::Text { text, .. } = &first.body else { panic!() };
        assert_eq!(text, "Test result · Passed: All checks pass\n`cargo test`", "the transcript says what was checked, in plain words");
        let outbox = f.app.store.outbox().unwrap();
        let file_index = outbox.iter().position(|blob| blob.id == first_attachment.id).unwrap();
        let file_blob = &outbox[file_index];
        assert_eq!(file_blob.kind, "file");
        assert_eq!(file_blob.group.as_deref(), Some(crate::model::relay_name(&f.chat).as_str()));
        let dek = f.app.dek().unwrap();
        assert_eq!(crate::crypto::decrypt(&dek, "file", &file_blob.ciphertext).unwrap(), b"first private report");
        assert!(!file_blob.ciphertext.windows(20).any(|bytes| bytes == b"first private report"));
        let (message_index, chat_blob) = outbox.iter().enumerate().find(|(_, blob)| {
            blob.kind == "chat" && matches!(crate::crypto::decrypt_json::<crate::model::ChatBlob>(&dek, "chat", &blob.ciphertext), Ok(crate::model::ChatBlob::Upsert { message }) if message.id == first.id)
        }).unwrap();
        assert!(file_index < message_index, "encrypted bytes precede the message that names them");
        let op: crate::model::ChatBlob = crate::crypto::decrypt_json(&dek, "chat", &chat_blob.ciphertext).unwrap();
        let crate::model::ChatBlob::Upsert { message } = op else { panic!() };
        assert_eq!(message, first);
        assert!(!serde_json::to_string(&first).unwrap().contains(f.workdir.to_str().unwrap()), "Runner paths never enter output metadata");
        assert!(first.for_app().output.is_some());

        let mut second_request = f.file(b"second report");
        second_request.replaces = Some(first.id.clone());
        let second = f.publish(second_request).unwrap();
        assert_eq!(first.output.as_ref().unwrap().id, second.output.as_ref().unwrap().id);
        assert_eq!(second.output.as_ref().unwrap().version, 2);
        assert!(matches!(&second.body, Body::Text { text, .. } if text.is_empty()), "a file alone is its attachment");
        assert_eq!(second.output.as_ref().unwrap().previous_message_id.as_deref(), Some(first.id.as_str()));
        assert_ne!(first_attachment.id, attachment(&second).id);
        std::fs::remove_file(f.workdir.join("report.txt")).unwrap();
        assert_eq!(std::fs::read(crate::files::local_path(&f.app, &first_attachment.id)).unwrap(), b"first private report");
        assert_eq!(std::fs::read(crate::files::local_path(&f.app, &attachment(&second).id)).unwrap(), b"second report");
        assert_eq!(list(&f.app, &f.chat, Some(&task_id)).unwrap(), [first.clone(), second.clone()]);
        assert!(list(&f.app, &f.chat, Some("another-task")).unwrap().is_empty());
        f.app.save_state_now();
        let restarted = App::load(f.app.config.clone()).unwrap();
        assert_eq!(list(&restarted, &f.chat, Some(&task_id)).unwrap(), [first.clone(), second]);
        assert_eq!(
            first.output.as_ref().unwrap().task_evidence(&first.id),
            json!({
                "kind":"output", "label":"Report.txt", "chat_id":f.chat, "message_id":first.id, "output_id":first.output.as_ref().unwrap().id, "version":1
            })
        );
    }

    #[test]
    fn commands_stay_inline_code() {
        assert_eq!(code_span("cargo test"), "`cargo test`");
        assert_eq!(code_span("echo `date`\nls"), "``echo `date` ls``");
        assert_eq!(code_span("`x`"), "`` `x` ``");
    }

    #[test]
    fn invalid_publications_create_no_output_and_cannot_replace_another_scope() {
        let f = Fixture::new();
        assert!(f.publish(PublishOutput { name: "Missing".into(), path: Some("absent.txt".into()), ..Default::default() }).unwrap_err().contains("unavailable"));
        let base = f.file(b"report");
        assert!(f.publish(PublishOutput { url: Some("https://example.com/report".into()), ..base.clone() }).unwrap_err().contains("exactly one"));
        assert!(f.publish(PublishOutput { task_id: Some("made-up-task".into()), ..base.clone() }).unwrap_err().contains("canonical"));
        assert!(
            f.publish(PublishOutput {
                evidence: Some(OutputEvidence { kind: EvidenceKind::TestResult, summary: "pass".into(), status: EvidenceStatus::Passed, command: None, exit_code: Some(1) }),
                ..base.clone()
            })
            .unwrap_err()
            .contains("conflicts")
        );
        assert!(
            f.publish(PublishOutput {
                evidence: Some(OutputEvidence {
                    kind: EvidenceKind::BeforeScreenshot,
                    summary: "Before".into(),
                    status: EvidenceStatus::Unverified,
                    command: None,
                    exit_code: None
                }),
                ..base.clone()
            })
            .unwrap_err()
            .contains("image")
        );
        assert!(list(&f.app, &f.chat, None).unwrap().is_empty());
        let first = f.publish(base.clone()).unwrap();
        let second = f.publish(PublishOutput { replaces: Some(first.id.clone()), ..base.clone() }).unwrap();
        assert!(f.publish(PublishOutput { replaces: Some(first.id), ..base.clone() }).unwrap_err().contains("latest"));
        assert!(f.publish(PublishOutput { replaces: Some(second.id), task_id: Some(format!("task-{}", uuid::Uuid::new_v4())), ..base }).unwrap_err().contains("keeps"));
        let bot = f.app.bot(&f.bot).unwrap();
        f.app.state.lock().unwrap().bots[0].runner_id = "remote-runner".into();
        assert!(f.publish(PublishOutput { name: "Doc".into(), url: Some("https://example.com".into()), ..Default::default() }).unwrap_err().contains("assigned Runner"));
        f.app.state.lock().unwrap().bots[0] = bot;
        f.app.state.lock().unwrap().chats[0].meta.bot_ids.clear();
        assert!(f.publish(PublishOutput { name: "Doc".into(), url: Some("https://example.com".into()), ..Default::default() }).unwrap_err().contains("not in this chat"));
    }

    #[test]
    fn before_and_after_screenshots_keep_their_task_and_underlying_files() {
        let f = Fixture::new();
        let task_id = format!("task-{}", uuid::Uuid::new_v4());
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(80, 60).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        std::fs::write(f.workdir.join("screen.png"), &png).unwrap();
        for kind in [EvidenceKind::BeforeScreenshot, EvidenceKind::AfterScreenshot] {
            let message = f
                .publish(PublishOutput {
                    name: format!("{kind:?}.png"),
                    path: Some("screen.png".into()),
                    task_id: Some(task_id.clone()),
                    evidence: Some(OutputEvidence { kind, summary: "The screen was inspected".into(), status: EvidenceStatus::Passed, command: None, exit_code: None }),
                    ..Default::default()
                })
                .unwrap();
            assert_eq!((attachment(&message).width, attachment(&message).height), (Some(80), Some(60)));
            assert_eq!(message.output.as_ref().unwrap().evidence.as_ref().unwrap().kind, kind);
            assert_eq!(std::fs::read(crate::files::local_path(&f.app, &attachment(&message).id)).unwrap(), png);
        }
        assert_eq!(list(&f.app, &f.chat, Some(&task_id)).unwrap().len(), 2);
    }

    #[test]
    fn existing_document_links_are_recorded_without_external_side_effects() {
        let f = Fixture::new();
        for url in ["http://example.com", "file:///etc/passwd", "javascript:alert(1)", "https://user:secret@example.com", "bad url"] {
            assert!(f.publish(PublishOutput { name: "Document".into(), url: Some(url.into()), ..Default::default() }).is_err(), "{url}");
        }
        let files_before = f.app.store.outbox().unwrap().iter().filter(|blob| blob.kind == "file").count();
        let link = f.publish(PublishOutput { name: "Shared report".into(), url: Some("https://docs.example.invalid/a?version=1".into()), ..Default::default() }).unwrap();
        assert_eq!(link.output.as_ref().unwrap().url.as_deref(), Some("https://docs.example.invalid/a?version=1"));
        let Body::Text { text, attachments, .. } = &link.body else { panic!() };
        assert_eq!(text, "[Shared report](https://docs.example.invalid/a?version=1)");
        assert!(attachments.is_empty());
        assert_eq!(f.app.store.outbox().unwrap().iter().filter(|blob| blob.kind == "file").count(), files_before);
    }

    #[cfg(feature = "runner")]
    #[tokio::test]
    async fn api_and_bot_tool_publish_task_evidence_and_respect_cancellation() {
        use lorca_agent::Tool;
        let f = Fixture::new();
        f.file(b"test output");
        let response = crate::api::dispatch(&f.app, "outputs.publish", json!({"chat_id":f.chat,"bot_id":f.bot,"name":"Tests.txt","path":"report.txt","evidence":{"kind":"test_result","summary":"Checks failed","status":"failed","command":"cargo test","exit_code":1}})).await.unwrap();
        assert_eq!(response["task_evidence"]["kind"], "output");
        assert_eq!(response["message"]["output"]["evidence"]["status"], "failed");
        assert_eq!(crate::api::dispatch(&f.app, "outputs.list", json!({"chat_id":f.chat})).await.unwrap()["outputs"].as_array().unwrap().len(), 1);
        let tool = PublishOutputTool { app: f.app.clone(), chat_id: f.chat.clone(), bot: f.app.bot(&f.bot).unwrap(), workdir: f.workdir.clone() };
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        assert!(tool.execute("stopped", json!({"name":"Stopped","path":"report.txt"}), cancel, Arc::new(|_| {})).await.is_err());
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(8, 6).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        std::fs::write(f.workdir.join("report.txt"), png).unwrap();
        let result = tool
            .execute(
                "publish",
                json!({"name":"Screenshot.png","path":"report.txt","mime":"image/png","evidence":{"kind":"after_screenshot","summary":"Layout checked","status":"passed"}}),
                tokio_util::sync::CancellationToken::new(),
                Arc::new(|_| {}),
            )
            .await
            .unwrap();
        assert!(result.structured.is_some());
        assert_eq!(result.structured.unwrap()["task_evidence"]["version"], 1);
        let catalog = Arc::new(lorca_agent::codemode::StaticCatalog::new(vec![Arc::new(tool)]));
        let codemode = lorca_agent::codemode::CodemodeTool::new(catalog, Default::default());
        let script_result = codemode
            .execute(
                "script",
                json!({"code":"const result = await tools.publish_output({name: 'Linked report', url: 'https://example.invalid/report'}); text(result.task_evidence.output_id);"}),
                tokio_util::sync::CancellationToken::new(),
                Arc::new(|_| {}),
            )
            .await
            .unwrap();
        assert!(script_result.text_content().contains("out-"), "codemode receives the structured evidence reference");
    }

    #[cfg(feature = "runner")]
    #[tokio::test]
    async fn a_bot_without_file_access_publishes_links_but_no_files() {
        use lorca_agent::Tool;
        let f = Fixture::new();
        f.file(b"private notes");
        f.app.state.lock().unwrap().bots[0].permissions =
            Some(crate::permissions::BotPermissions { filesystem: crate::permissions::FilesystemAccess::None, ..Default::default() });
        let tool = PublishOutputTool { app: f.app.clone(), chat_id: f.chat.clone(), bot: f.app.bot(&f.bot).unwrap(), workdir: f.workdir.clone() };
        let run = |args: serde_json::Value| tool.execute("publish", args, tokio_util::sync::CancellationToken::new(), Arc::new(|_| {}));
        let refused = run(json!({"name":"Notes","path":"report.txt"})).await.unwrap_err();
        assert!(refused.0.contains("reading files is off"), "{}", refused.0);
        assert!(list(&f.app, &f.chat, None).unwrap().is_empty(), "nothing was published");
        let requests = f.app.store.page(&f.chat, None, 50).unwrap().0;
        assert!(requests.iter().any(|m| matches!(&m.body, Body::Permission { tool, summary, .. } if tool == "access" && summary == "Reading files")), "the user is asked");
        run(json!({"name":"Spec","url":"https://docs.example.invalid/spec"})).await.unwrap();
        assert_eq!(list(&f.app, &f.chat, None).unwrap().len(), 1);
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn a_distinct_paired_device_fetches_encrypted_files_and_reports_missing_blobs() {
        use axum::http::StatusCode;
        use axum::{
            Json, Router,
            routing::{get, post},
        };
        let f = Fixture::new();
        let output = f.publish(f.file(b"private downloadable report")).unwrap();
        let ciphertext = f.app.store.outbox().unwrap().into_iter().find(|blob| blob.id == attachment(&output).id).unwrap().ciphertext;
        let expected_id = attachment(&output).id.clone();
        let router = Router::new()
            .route("/v1/auth/challenge", post(|| async { Json(json!({"nonce":"challenge"})) }))
            .route("/v1/auth/verify", post(|| async { Json(json!({"token":"paired-test-token","expires_at":crate::config::now_unix()+600})) }))
            .route(
                "/v1/files/{id}",
                get(move |axum::extract::Path(id): axum::extract::Path<String>| {
                    let ciphertext = ciphertext.clone();
                    let expected_id = expected_id.clone();
                    async move {
                        if id == expected_id {
                            (StatusCode::OK, [("content-type", "application/octet-stream")], ciphertext)
                        } else {
                            (StatusCode::NOT_FOUND, [("content-type", "application/octet-stream")], Vec::new())
                        }
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let paired_home = f.home.join("paired");
        let paired = App::load(crate::config::Config { home: paired_home.clone(), port: 0 }).unwrap();
        let mut machine_file = f.app.machine_file().unwrap();
        machine_file.machine_secret = crate::keys::b64(&crate::keys::Machine::generate().secret);
        machine_file.name = "Paired Device".into();
        machine_file.relay_url = Some(url.clone());
        *paired.machine.lock().unwrap() = Some(machine_file.clone());
        paired.set_relay_url(Some(url)).unwrap();
        assert_ne!(paired.this_device_id(), f.app.this_device_id());
        paired.state.lock().unwrap().bots = f.app.state.lock().unwrap().bots.clone();
        paired.state.lock().unwrap().chats = f.app.state.lock().unwrap().chats.clone();
        for (seq, blob) in f.app.store.outbox().unwrap().into_iter().filter(|blob| blob.kind == "chat").enumerate() {
            crate::sync::apply_blob(
                &paired,
                &machine_file,
                &crate::relay::BlobIn {
                    id: blob.id,
                    kind: blob.kind,
                    recipient_machine_pubkey: None,
                    seq: seq as i64 + 1,
                    ciphertext: crate::keys::b64(&blob.ciphertext),
                    created_at: crate::config::now_unix(),
                },
            );
        }
        assert_eq!(list(&paired, &f.chat, None).unwrap(), [output.clone()]);
        assert!(!crate::files::is_local(&paired, &attachment(&output).id));
        let path = crate::files::ensure_local(&paired, attachment(&output)).await.unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"private downloadable report");
        let named = crate::api::dispatch(&paired, "files.path", json!({"attachment":attachment(&output),"named":true})).await.unwrap();
        assert_eq!(std::path::Path::new(named["path"].as_str().unwrap()).file_name().unwrap(), "Report.txt");
        let mut missing = attachment(&output).clone();
        missing.id = "att-missing".into();
        assert!(crate::files::ensure_local(&paired, &missing).await.unwrap_err().to_string().contains("no longer has"));
        missing.id = "../../escape".into();
        assert!(crate::files::ensure_local(&paired, &missing).await.unwrap_err().to_string().contains("Invalid attachment"));
        server.abort();
    }
}
