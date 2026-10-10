//! Emails and Slack messages a bot writes in a chat, as drafts the user sends, after Grok Bot's
//! draft cards. A manifest names the tools that write to people (`tools.messages`) and the
//! arguments that hold each part of the message. In a chat, with the bot's drafts on, a call to
//! one does not run: the exact call is staged as a [review item](crate::review_queue)
//! ([`stage`]), which the chat shows as a `draft` card ([`card`]). The card's edits go onto the
//! call's arguments ([`write`]) as the item's next version, and Send approves that version, which
//! the Runner runs as it runs any approved item. Gmail's server only makes drafts, so a Gmail
//! card's Send then sends the draft it made with Gmail's own API ([`deliver`]).

use std::path::PathBuf;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::app::App;
use crate::model::{Author, Body, DraftAttachment, Message, MessageDraft};
use crate::plugins::MessageTool;
use crate::review_queue::{ReviewItem, ReviewPayload, ReviewState};

/// An attachment's content waits on the Runner, outside the reviewed call, which has to stay
/// small: the call names it by this prefix, its size, and the content's hash.
const STASHED: &str = "lorca-draft-file:";

/// The message tool an item's call is, with the account's display name ("Gmail · Work").
pub fn tool_of(app: &App, payload: &ReviewPayload) -> Option<(MessageTool, String)> {
    let ReviewPayload::Plugin { plugin_id, tool, .. } = payload else { return None };
    let store = app.plugins.lock().unwrap();
    let plugin = store.get(plugin_id)?;
    Some((plugin.manifest.tools.message(tool)?.clone(), plugin.display_name()))
}

fn field<'a>(arguments: &'a Value, key: &Option<String>) -> &'a Value {
    key.as_deref().and_then(|key| arguments.get(key)).unwrap_or(&Value::Null)
}

fn strings(value: &Value) -> Vec<String> {
    let parts: Vec<&str> = match value {
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
        Value::String(text) => text.split(',').collect(),
        _ => Vec::new(),
    };
    parts.into_iter().map(str::trim).filter(|part| !part.is_empty()).map(String::from).collect()
}

/// The decoded size of base64 `text`.
fn decoded_len(text: &str) -> u64 {
    (text.trim_end_matches('=').len() as u64 * 3) / 4
}

fn attachment(item: &Value) -> DraftAttachment {
    let content = item["content"].as_str().unwrap_or_default();
    let size = match content.strip_prefix(STASHED) {
        Some(rest) => rest.split(':').next().and_then(|size| size.parse().ok()).unwrap_or(0),
        None => decoded_len(content),
    };
    DraftAttachment { name: item["filename"].as_str().unwrap_or("Attachment").into(), size }
}

/// The message a call writes, as its card shows it.
pub fn read(tool: &MessageTool, arguments: &Value) -> MessageDraft {
    MessageDraft {
        kind: tool.kind.clone(),
        to: strings(field(arguments, &tool.to)),
        cc: strings(field(arguments, &tool.cc)),
        bcc: strings(field(arguments, &tool.bcc)),
        subject: field(arguments, &tool.subject).as_str().unwrap_or_default().into(),
        body: arguments.get(&tool.body).and_then(Value::as_str).unwrap_or_default().into(),
        attachments: field(arguments, &tool.attachments).as_array().map(|items| items.iter().map(attachment).collect()).unwrap_or_default(),
        reply: field(arguments, &tool.reply).as_str().filter(|reply| !reply.is_empty()).map(String::from),
    }
}

/// Puts a card's edit onto the call's arguments: the recipients, subject, and text it shows, and
/// the attachments it still names. What it does not show, such as the message a reply answers,
/// stays as the bot wrote it.
pub fn write(tool: &MessageTool, arguments: &mut Value, draft: &MessageDraft) -> Result<(), String> {
    if draft.body.trim().is_empty() {
        return Err("Write a message first.".into());
    }
    if draft.to.iter().all(|to| to.trim().is_empty()) {
        return Err("Add a recipient first.".into());
    }
    let email = tool.kind == "email";
    if email {
        for address in draft.to.iter().chain(&draft.cc).chain(&draft.bcc).map(|address| address.trim()) {
            if !address.contains('@') || address.contains(char::is_whitespace) || address.contains(['<', '>', ',']) {
                return Err(format!("“{address}” is not an email address."));
            }
        }
    }
    let object = arguments.as_object_mut().ok_or("The draft's call has no arguments.")?;
    let mut set_list = |key: &Option<String>, values: &[String]| {
        let Some(key) = key else { return };
        let values: Vec<&str> = values.iter().map(|value| value.trim()).filter(|value| !value.is_empty()).collect();
        if values.is_empty() {
            object.remove(key);
        } else if !email || object.get(key).is_some_and(Value::is_string) {
            // A channel is one id; an address list the bot wrote as text stays text.
            object.insert(key.clone(), json!(if email { values.join(", ") } else { values[0].to_string() }));
        } else {
            object.insert(key.clone(), json!(values));
        }
    };
    set_list(&tool.to, &draft.to);
    set_list(&tool.cc, &draft.cc);
    set_list(&tool.bcc, &draft.bcc);
    if let Some(key) = &tool.subject {
        object.insert(key.clone(), json!(draft.subject.trim()));
    }
    object.insert(tool.body.clone(), json!(draft.body));
    if let Some(key) = &tool.attachments {
        if let Some(Value::Array(items)) = object.get_mut(key) {
            let mut kept: Vec<&str> = draft.attachments.iter().map(|attachment| attachment.name.as_str()).collect();
            items.retain(|item| match kept.iter().position(|name| Some(*name) == item["filename"].as_str()) {
                Some(index) => {
                    kept.remove(index);
                    true
                }
                None => false,
            });
            if items.is_empty() {
                object.remove(key);
            }
        }
    }
    Ok(())
}

/// The message as words, for feedback and for the bot's later turns: its header lines, then its
/// text.
pub fn text(draft: &MessageDraft) -> String {
    let mut lines = Vec::new();
    let mut line = |label: &str, values: &[String]| {
        if !values.is_empty() {
            lines.push(format!("{label}: {}", values.join(", ")));
        }
    };
    line("To", &draft.to);
    line("Cc", &draft.cc);
    line("Bcc", &draft.bcc);
    if !draft.subject.is_empty() {
        lines.push(format!("Subject: {}", draft.subject));
    }
    let names: Vec<String> = draft.attachments.iter().map(|attachment| attachment.name.clone()).collect();
    if !names.is_empty() {
        lines.push(format!("Attachments: {}", names.join(", ")));
    }
    lines.push(String::new());
    lines.push(draft.body.clone());
    lines.join("\n")
}

/// What the message is, in a line: "Email to ana@example.com: Lunch", "Slack message to C024BE91L".
pub fn summary(draft: &MessageDraft) -> String {
    let what = if draft.kind == "email" { "Email" } else { "Slack message" };
    let to = draft.to.join(", ");
    match (to.is_empty(), draft.subject.trim()) {
        (true, _) => what.to_string(),
        (false, "") => format!("{what} to {to}"),
        (false, subject) => format!("{what} to {to}: {subject}"),
    }
}

/// The item's draft and the call's message tool, when it writes a message.
pub fn draft_of(app: &App, item: &ReviewItem) -> Option<MessageDraft> {
    let (tool, _) = tool_of(app, &item.payload)?;
    let ReviewPayload::Plugin { arguments, .. } = &item.payload else { return None };
    Some(read(&tool, arguments))
}

/// The chat's view of an item whose call writes a message: its draft card, by the bot, in the
/// place of the review's notice.
pub fn card(app: &App, item: &ReviewItem) -> Option<Message> {
    let (tool, account) = tool_of(app, &item.payload)?;
    let ReviewPayload::Plugin { plugin_id, arguments, .. } = &item.payload else { return None };
    let outcome = item.outcome.as_ref();
    let note = match item.state {
        ReviewState::Pending | ReviewState::Uncertain => outcome.map(|outcome| outcome.summary.clone()),
        ReviewState::Failed => outcome.and_then(|outcome| outcome.result.as_ref()).and_then(|result| result["text"].as_str()).map(|text| text.chars().take(300).collect()),
        _ => None,
    };
    let state = serde_json::to_value(item.state).ok()?.as_str()?.to_string();
    let mut message = Message::new(
        &item.origin.chat_id,
        Author::Bot { bot_id: item.bot_id.clone() },
        Body::Draft {
            review_id: item.id.clone(),
            version: item.version,
            state,
            plugin_id: plugin_id.clone(),
            account,
            draft: read(&tool, arguments),
            note: note.filter(|note| !note.trim().is_empty()),
            direct: tool.send.is_none(),
        },
    );
    message.id = item.message_id();
    message.created_at = item.created_at;
    Some(message)
}

/// How a draft card reads to the bot in later turns: what became of it, and the message as it
/// stands, which the user may have changed before sending.
pub fn history_line(state: &str, draft: &MessageDraft) -> String {
    let what = match state {
        "pending" => "waits for the user to send it",
        "approved" | "executing" => "is being sent",
        "succeeded" => "was sent by the user",
        "failed" => "could not be sent",
        "rejected" | "cancelled" => "was discarded by the user",
        _ => "may or may not have been sent",
    };
    format!("[Your draft {what}: {}]\n{}", summary(draft), text(draft))
}

fn stash_dir(app: &App) -> PathBuf {
    app.config.home.join("drafts")
}

/// Moves the call's attachment contents into the Runner's own encrypted files and names them in
/// the call by size and hash, so a review item of a few hundred bytes stands for a message of
/// megabytes, and the hash binds the files to what the user reviewed.
pub fn stash(app: &App, tool: &MessageTool, arguments: &mut Value) -> Result<(), String> {
    let Some(key) = &tool.attachments else { return Ok(()) };
    let Some(items) = arguments.get_mut(key.as_str()).and_then(Value::as_array_mut) else { return Ok(()) };
    let dek = app.dek().ok_or("Pair or create an identity first.")?;
    for item in items {
        let Some(content) = item["content"].as_str().filter(|content| !content.starts_with(STASHED)) else { continue };
        let hash = Sha256::digest(content.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        let path = stash_dir(app).join(&hash);
        if !path.exists() {
            std::fs::create_dir_all(stash_dir(app)).map_err(|error| error.to_string())?;
            let sealed = crate::crypto::encrypt(&dek, "draft_attachment", content.as_bytes()).map_err(|error| error.to_string())?;
            std::fs::write(&path, sealed).map_err(|error| error.to_string())?;
        }
        item["content"] = json!(format!("{STASHED}{}:{hash}", decoded_len(content)));
    }
    Ok(())
}

/// The stashed files a call names.
fn stashed(tool: &MessageTool, arguments: &Value) -> Vec<String> {
    field(arguments, &tool.attachments)
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["content"].as_str()?.strip_prefix(STASHED)?.split(':').nth(1).map(String::from))
        .collect()
}

/// The call as it goes out: each stashed attachment's content back in its place.
pub fn restore(app: &App, tool: &MessageTool, arguments: &mut Value) -> Result<(), String> {
    let Some(key) = &tool.attachments else { return Ok(()) };
    let Some(items) = arguments.get_mut(key.as_str()).and_then(Value::as_array_mut) else { return Ok(()) };
    let dek = app.dek().ok_or("Pair or create an identity first.")?;
    for item in items {
        let Some(hash) = item["content"].as_str().and_then(|content| content.strip_prefix(STASHED)).and_then(|rest| rest.split(':').nth(1)).map(String::from) else { continue };
        let missing = || format!("{} is no longer on this Runner. Remove it, or ask the bot to attach it again.", item["filename"].as_str().unwrap_or("An attachment"));
        let sealed = std::fs::read(stash_dir(app).join(&hash)).map_err(|_| missing())?;
        let content = crate::crypto::decrypt(&dek, "draft_attachment", &sealed).map_err(|_| missing())?;
        item["content"] = json!(String::from_utf8(content).map_err(|_| missing())?);
    }
    Ok(())
}

/// Deletes the stashed files of a message that was sent or discarded, apart from those another
/// open draft still names.
pub fn forget(app: &App, item: &ReviewItem) {
    if matches!(item.state, ReviewState::Pending | ReviewState::Approved | ReviewState::Executing) {
        return;
    }
    let Some((tool, _)) = tool_of(app, &item.payload) else { return };
    let ReviewPayload::Plugin { arguments, .. } = &item.payload else { return };
    forget_files(app, stashed(&tool, arguments), Some(&item.id));
}

/// Deletes `files` from the stash, apart from those an open draft other than `except` names.
fn forget_files(app: &App, files: Vec<String>, except: Option<&str>) {
    if files.is_empty() {
        return;
    }
    let open: Vec<String> = crate::review_queue::list(app)
        .unwrap_or_default()
        .iter()
        .filter(|other| Some(other.id.as_str()) != except && matches!(other.state, ReviewState::Pending | ReviewState::Approved | ReviewState::Executing))
        .filter_map(|other| match (&other.payload, tool_of(app, &other.payload)) {
            (ReviewPayload::Plugin { arguments, .. }, Some((tool, _))) => Some(stashed(&tool, arguments)),
            _ => None,
        })
        .flatten()
        .collect();
    for hash in files.iter().filter(|hash| !open.contains(hash)) {
        let _ = std::fs::remove_file(stash_dir(app).join(hash));
    }
}

#[cfg(feature = "runner")]
pub use runner::*;

#[cfg(feature = "runner")]
mod runner {
    use std::sync::Arc;

    use lorca_agent::BeforeToolCallResult;
    use serde_json::{json, Value};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::model::Bot;
    use crate::plugins::review::Trigger;
    use crate::review_queue::ReviewTarget;

    /// A message call in a chat whose bot drafts: the exact call waits as a review item, which
    /// the chat shows as a draft card with Send and Discard, and the call does not run. The
    /// script that made it stops there, and the bot hears that the draft is ready. A turn the user
    /// stopped (`cancel`) leaves no draft, and none of its files.
    #[allow(clippy::too_many_arguments)]
    pub async fn stage(
        app: &Arc<App>,
        bot: &Bot,
        chat_id: &str,
        trigger: &Trigger,
        call_id: &str,
        (plugin_id, server_name, plugin_name): (&str, &str, &str),
        name: &str,
        tool: &MessageTool,
        args: &Value,
        cancel: &CancellationToken,
    ) -> BeforeToolCallResult {
        if cancel.is_cancelled() {
            return crate::local_review::blocked("Stopped".into());
        }
        // A saved secret never goes into a message, and never into the chat a draft shows in.
        if holds_secret(app, tool, args) {
            return crate::local_review::blocked(
                "The message holds a saved secret or a secret's placeholder, so it was not drafted. Write it without the secret: \
                 secrets go only where the user saved them for, never into an email or a Slack message."
                    .into(),
            );
        }
        let draft = read(tool, args);
        let mut arguments = args.clone();
        let staged = match stash(app, tool, &mut arguments) {
            Ok(()) => {
                let files = stashed(tool, &arguments);
                let payload = ReviewPayload::Plugin { plugin_id: plugin_id.into(), server_name: server_name.into(), tool: name.into(), arguments };
                let target = ReviewTarget { account: plugin_name.into(), resource: summary(&draft) };
                let rationale = format!("{} wrote this for you to send.", bot.name);
                let staged = crate::review_execution::stage_call(app, bot, chat_id, trigger, call_id, payload, target, Some(&rationale), cancel).await;
                if staged.is_err() {
                    forget_files(app, files, None);
                }
                staged
            }
            Err(error) => Err(error),
        };
        // A draft saved before the Stop landed stays, and the bot hears of it.
        if staged.is_err() && cancel.is_cancelled() {
            return crate::local_review::blocked("Stopped".into());
        }
        crate::local_review::blocked(match staged {
            Ok(_) => format!(
                "{} is ready as a draft in the chat, where the user can edit it, then Send or Discard it. It has not been sent. Tell the user \
                 it is ready in a sentence; don't repeat it, send it another way, or draft it again. Write any other message with another call.",
                summary(&draft)
            ),
            Err(error) => format!("Could not put the draft in the chat: {error}"),
        })
    }

    /// Whether a message call carries a secret saved on this Runner, in its fields or in a text
    /// file it attaches, or a secret's `{{secret:NAME}}` placeholder, which only Browser fills.
    fn holds_secret(app: &App, tool: &MessageTool, arguments: &Value) -> bool {
        fn texts(value: &Value, out: &mut Vec<String>) {
            match value {
                Value::String(text) => out.push(text.clone()),
                Value::Array(items) => items.iter().for_each(|item| texts(item, out)),
                Value::Object(fields) => fields.values().for_each(|item| texts(item, out)),
                _ => {}
            }
        }
        let mut all = Vec::new();
        texts(arguments, &mut all);
        if all.iter().any(|text| text.contains("{{secret:")) {
            return true;
        }
        let redactions = crate::secrets::Redactions::load(app);
        if redactions.is_empty() {
            return false;
        }
        if all.iter().any(|text| redactions.text(text).is_some()) {
            return true;
        }
        use base64::Engine;
        field(arguments, &tool.attachments).as_array().into_iter().flatten().filter_map(|item| item["content"].as_str()).any(|content| {
            base64::engine::general_purpose::STANDARD
                .decode(content.trim())
                .is_ok_and(|bytes| redactions.text(&String::from_utf8_lossy(&bytes)).is_some())
        })
    }

    /// The Gmail API's address; tests point it at a local server.
    fn gmail_api() -> String {
        std::env::var("LORCA_GMAIL_API_URL").unwrap_or_else(|_| "https://gmail.googleapis.com".into())
    }

    /// The draft id in what Gmail's `create_draft` answered: in its structured content, or in
    /// its text as JSON.
    fn draft_id(result: &Value) -> Option<String> {
        let pick = |value: &Value| {
            [&value["id"], &value["draft"]["id"], &value["draftId"]].into_iter().find_map(|id| id.as_str().filter(|id| !id.is_empty()).map(String::from))
        };
        pick(&result["structuredContent"]).or_else(|| {
            result["content"].as_array()?.iter().filter_map(|part| part["text"].as_str()).find_map(|text| pick(&serde_json::from_str(text).ok()?))
        })
    }

    /// Why a send did not happen: refused, or cut off with no answer, in which case it may have
    /// gone out.
    pub enum Undelivered {
        Refused(String),
        Unknown(String),
    }

    /// After the reviewed call made the draft, sends it, for a message tool whose server only
    /// drafts. Gmail: `drafts.send` with the account's own sign-in.
    pub async fn deliver(app: &Arc<App>, plugin_id: &str, server_name: &str, tool: &MessageTool, result: &Value) -> Result<(), Undelivered> {
        if tool.send.as_deref() != Some("gmail") {
            return Ok(());
        }
        let id = draft_id(result).ok_or_else(|| Undelivered::Refused("Gmail made the draft but did not say which, so it was not sent. It is in the account's Drafts.".into()))?;
        let token = crate::plugins::mcp::service_token(app, plugin_id, server_name)
            .await
            .map_err(|error| Undelivered::Refused(format!("The draft is in the account's Drafts, but it was not sent: {error}")))?;
        let response = app
            .http
            .post(format!("{}/gmail/v1/users/me/drafts/send", gmail_api()))
            .bearer_auth(token)
            .json(&json!({ "id": id }))
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .map_err(|error| Undelivered::Unknown(format!("Gmail did not answer ({}).", error.without_url())))?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let body: Value = response.json().await.unwrap_or_default();
        let reason = body["error"]["message"].as_str().map(|text| text.chars().take(300).collect::<String>()).unwrap_or_else(|| status.to_string());
        if status.is_server_error() {
            return Err(Undelivered::Unknown(format!("Gmail answered with an error ({reason}).")));
        }
        Err(Undelivered::Refused(format!("Gmail did not send it: {reason}. The draft is in the account's Drafts.")))
    }

    #[cfg(test)]
    #[test]
    fn a_draft_id_is_read_from_structured_content_or_text() {
        assert_eq!(draft_id(&json!({ "structuredContent": { "id": "r-1", "message": {} } })).as_deref(), Some("r-1"));
        assert_eq!(draft_id(&json!({ "content": [{ "type": "text", "text": "{\"draft\":{\"id\":\"r-2\"}}" }] })).as_deref(), Some("r-2"));
        assert_eq!(draft_id(&json!({ "content": [{ "type": "text", "text": "Draft created" }] })), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gmail() -> MessageTool {
        serde_json::from_value(json!({ "tool": "create_draft", "kind": "email", "to": "to", "cc": "cc", "bcc": "bcc", "subject": "subject",
            "body": "body", "attachments": "attachments", "reply": "replyToMessageId", "send": "gmail" }))
        .unwrap()
    }

    fn slack() -> MessageTool {
        serde_json::from_value(json!({ "tool": "slack_send_message", "kind": "slack", "to": "channel_id", "body": "message", "reply": "thread_ts" })).unwrap()
    }

    #[test]
    fn a_card_edit_goes_onto_the_call_and_keeps_what_it_does_not_show() {
        let tool = gmail();
        let mut arguments = json!({ "to": ["ana@example.com"], "subject": "Lunch", "body": "Noon?", "replyToMessageId": "m-1", "htmlBody": "<p>Noon?</p>",
            "attachments": [{ "filename": "menu.pdf", "content": "AAAA" }, { "filename": "map.png", "content": "BBBBBB==" }] });
        let draft = read(&tool, &arguments);
        assert_eq!(draft.to, ["ana@example.com"]);
        assert_eq!((draft.subject.as_str(), draft.reply.as_deref(), draft.attachments.len()), ("Lunch", Some("m-1"), 2));
        assert_eq!(draft.attachments[0], DraftAttachment { name: "menu.pdf".into(), size: 3 });

        let edited = MessageDraft { to: vec!["ana@example.com".into(), " bo@example.com ".into()], cc: vec![], subject: "Lunch at noon".into(),
            body: "Noon works.".into(), attachments: vec![draft.attachments[1].clone()], ..draft };
        write(&tool, &mut arguments, &edited).unwrap();
        assert_eq!(arguments, json!({ "to": ["ana@example.com", "bo@example.com"], "subject": "Lunch at noon", "body": "Noon works.",
            "replyToMessageId": "m-1", "htmlBody": "<p>Noon?</p>", "attachments": [{ "filename": "map.png", "content": "BBBBBB==" }] }));
        assert_eq!(text(&read(&tool, &arguments)), "To: ana@example.com, bo@example.com\nSubject: Lunch at noon\nAttachments: map.png\n\nNoon works.");

        let bad = MessageDraft { to: vec!["Ana <ana@example.com>".into()], body: "Hi".into(), ..Default::default() };
        assert!(write(&tool, &mut arguments.clone(), &bad).unwrap_err().contains("is not an email address"));
        assert_eq!(write(&tool, &mut arguments.clone(), &MessageDraft { to: vec!["a@b.c".into()], ..Default::default() }).unwrap_err(), "Write a message first.");
    }

    #[test]
    fn a_slack_message_has_one_channel() {
        let tool = slack();
        let mut arguments = json!({ "channel_id": "C024BE91L", "message": "Ship it", "thread_ts": "1.2" });
        let draft = read(&tool, &arguments);
        assert_eq!((draft.to.clone(), draft.subject.as_str(), draft.reply.as_deref()), (vec!["C024BE91L".to_string()], "", Some("1.2")));
        assert_eq!(summary(&draft), "Slack message to C024BE91L");
        write(&tool, &mut arguments, &MessageDraft { to: vec!["C9".into(), "C10".into()], body: "Shipped".into(), ..draft }).unwrap();
        assert_eq!(arguments, json!({ "channel_id": "C9", "message": "Shipped", "thread_ts": "1.2" }));
    }
}
