//! Gmail's tools over the Gmail API (v1). Messages, threads, and drafts read as Google's MCP
//! server gives them: the headers by name, the plain and HTML bodies decoded, attachments as
//! metadata.

use std::sync::Arc;

use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use serde_json::{json, Value};

use super::{gmail_api, list, number, param, required, result, text, Api, Failure};
use crate::app::App;
use crate::plugins::builtin::Service;

pub struct Server;

/// The server of a Gmail account, in its manifest.
const SERVER: &str = "gmail";

/// The most threads or drafts one listing reads in full.
const MOST: u64 = 25;

#[async_trait::async_trait]
impl Service for Server {
    fn name(&self) -> &'static str {
        "gmail"
    }

    fn instructions(&self) -> Option<String> {
        Some("Gmail for one account. search_threads takes Gmail's search syntax (from:, to:, subject:, is:unread, newer_than:7d, label:). Message and thread ids come from search_threads, get_thread, and list_drafts.".into())
    }

    fn tools(&self, _app: &Arc<App>, _plugin_id: &str) -> Vec<Value> {
        super::tools("gmail")
    }

    async fn call(&self, app: &Arc<App>, plugin_id: &str, tool: &str, args: Value, _context: &Value) -> Value {
        let api = Api::new(app, plugin_id, SERVER);
        result(match tool {
            "search_threads" => search_threads(&api, &args).await,
            "get_thread" => get_thread(&api, &args).await,
            "get_message" => get_message(&api, &args).await,
            "list_labels" => list_labels(&api, &args).await,
            "create_label" => create_label(&api, &args).await,
            "label_message" => modify(&api, "messages", "messageId", "addLabelIds", &args).await,
            "unlabel_message" => modify(&api, "messages", "messageId", "removeLabelIds", &args).await,
            "label_thread" => modify(&api, "threads", "threadId", "addLabelIds", &args).await,
            "unlabel_thread" => modify(&api, "threads", "threadId", "removeLabelIds", &args).await,
            "list_drafts" => list_drafts(&api, &args).await,
            "create_draft" => create_draft(&api, &args).await,
            "send_message" => send_message(&api, &args).await,
            _ => Err(format!("Gmail has no tool {tool}.")),
        })
    }
}

fn url(path: &str) -> String {
    format!("{}/gmail/v1/users/me/{path}", gmail_api())
}

fn page(args: &Value, query: &mut Vec<(&'static str, String)>) {
    param(query, "maxResults", number(args, "pageSize").map(|n| n.parse::<u64>().map(|n| n.clamp(1, 500).to_string()).unwrap_or(n)));
    param(query, "pageToken", text(args, "pageToken"));
}

/// The format Gmail reads a message in for one of the MCP server's.
fn format(args: &Value, default: &'static str) -> &'static str {
    match args["messageFormat"].as_str().unwrap_or_default() {
        "MINIMAL" => "minimal",
        "METADATA_ONLY" => "metadata",
        "FULL_CONTENT" => "full",
        _ => default,
    }
}

async fn search_threads(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let mut query = Vec::new();
    param(&mut query, "q", text(args, "query"));
    page(args, &mut query);
    if args["includeTrash"].as_bool() == Some(true) {
        query.push(("includeSpamTrash", "true".into()));
    }
    let found = api.get(&url("threads"), &query).await?;
    let listed: Vec<Value> = found["threads"].as_array().cloned().unwrap_or_default();
    let threads = if args["view"].as_str() == Some("THREAD_VIEW_MINIMAL") {
        listed.iter().map(|thread| json!({ "id": thread["id"], "snippet": thread["snippet"] })).collect()
    } else {
        // Each thread's messages by their headers, read together.
        let reads = listed.iter().take(MOST as usize).filter_map(|thread| thread["id"].as_str()).map(|id| read_thread(api, id, "metadata"));
        futures::future::join_all(reads).await.into_iter().collect::<Result<Vec<_>, _>>()?
    };
    Ok(json!({ "threads": threads, "nextPageToken": found["nextPageToken"], "resultCountEstimate": found["resultSizeEstimate"] }))
}

async fn read_thread(api: &Api<'_>, id: &str, format: &str) -> Result<Value, Failure> {
    let thread = api.get(&url(&format!("threads/{}", segment(id))), &headers_query(format)).await?;
    let messages: Vec<Value> = thread["messages"].as_array().into_iter().flatten().map(message).collect();
    Ok(json!({ "id": thread["id"], "messages": messages }))
}

fn headers_query(format: &str) -> Vec<(&'static str, String)> {
    let mut query = vec![("format", format.to_string())];
    if format == "metadata" {
        for header in ["Subject", "From", "To", "Cc", "Bcc", "Date"] {
            query.push(("metadataHeaders", header.into()));
        }
    }
    query
}

async fn get_thread(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    read_thread(api, &required(args, "threadId")?, format(args, "full")).await
}

async fn get_message(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let id = required(args, "messageId")?;
    let found = api.get(&url(&format!("messages/{}", segment(&id))), &headers_query(format(args, "full"))).await?;
    Ok(message(&found))
}

/// A Gmail API message as the MCP server's `Message`.
pub(super) fn message(found: &Value) -> Value {
    let headers = found["payload"]["headers"].as_array().cloned().unwrap_or_default();
    let header = |name: &str| headers.iter().find(|h| h["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name))).and_then(|h| h["value"].as_str()).map(str::to_string);
    let mut plain = String::new();
    let mut html = String::new();
    let mut attachments = Vec::new();
    walk(&found["payload"], &mut plain, &mut html, &mut attachments);
    let mut out = json!({
        "id": found["id"],
        "threadId": found["threadId"],
        "snippet": found["snippet"],
        "subject": header("Subject"),
        "sender": header("From"),
        "toRecipients": addresses(header("To")),
        "ccRecipients": addresses(header("Cc")),
        "date": header("Date"),
        "labelIds": found["labelIds"],
        "attachmentIds": attachments.iter().map(|a: &Value| a["id"].clone()).collect::<Vec<_>>(),
        "attachments": attachments,
    });
    if !plain.is_empty() {
        out["plaintextBody"] = json!(plain);
    }
    if !html.is_empty() {
        out["htmlBody"] = json!(html);
    }
    out
}

/// A message's parts, depth first: its text and HTML bodies, and its attachments.
fn walk(part: &Value, plain: &mut String, html: &mut String, attachments: &mut Vec<Value>) {
    let mime = part["mimeType"].as_str().unwrap_or_default();
    let filename = part["filename"].as_str().unwrap_or_default();
    if !filename.is_empty() || part["body"]["attachmentId"].is_string() {
        attachments.push(json!({ "id": part["body"]["attachmentId"], "filename": filename, "mimeType": mime }));
    } else if let Some(data) = part["body"]["data"].as_str() {
        let decoded = decode(data);
        match mime {
            "text/plain" => plain.push_str(&decoded),
            "text/html" => html.push_str(&decoded),
            _ => {}
        }
    }
    for child in part["parts"].as_array().into_iter().flatten() {
        walk(child, plain, html, attachments);
    }
}

fn decode(data: &str) -> String {
    let bytes = URL_SAFE_NO_PAD.decode(data.trim_end_matches('=')).or_else(|_| URL_SAFE.decode(data)).unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn addresses(header: Option<String>) -> Vec<String> {
    header.map(|h| h.split(',').map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).collect()).unwrap_or_default()
}

async fn list_labels(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let found = api.get(&url("labels"), &[]).await?;
    let labels: Vec<Value> = found["labels"].as_array().into_iter().flatten().map(label).collect();
    // The Gmail API lists every label at once; a page size cuts the list.
    let most = args["pageSize"].as_u64().map(|n| n as usize).unwrap_or(labels.len());
    Ok(json!({ "labels": labels.into_iter().take(most).collect::<Vec<_>>() }))
}

fn label(found: &Value) -> Value {
    json!({ "labelId": found["id"], "name": found["name"], "type": found["type"], "color": found["color"], "threadsTotal": found["threadsTotal"], "threadsUnread": found["threadsUnread"] })
}

async fn create_label(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let mut body = json!({ "name": required(args, "displayName")?, "labelListVisibility": "labelShow", "messageListVisibility": "show" });
    if args["color"].is_object() {
        body["color"] = args["color"].clone();
    }
    Ok(label(&api.post(&url("labels"), &[], body).await?))
}

async fn modify(api: &Api<'_>, kind: &str, id_key: &str, change: &str, args: &Value) -> Result<Value, Failure> {
    let id = required(args, id_key)?;
    let labels = list(args, "labelIds");
    if labels.is_empty() {
        return Err("labelIds is required.".into());
    }
    api.post(&url(&format!("{kind}/{}/modify", segment(&id))), &[], Value::Object([(change.to_string(), json!(labels))].into_iter().collect())).await?;
    Ok(json!({}))
}

async fn list_drafts(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let mut query = Vec::new();
    param(&mut query, "q", text(args, "query"));
    page(args, &mut query);
    let found = api.get(&url("drafts"), &query).await?;
    let listed: Vec<Value> = found["drafts"].as_array().cloned().unwrap_or_default();
    let format = if args["view"].as_str() == Some("DRAFT_VIEW_FULL") { "full" } else { "metadata" };
    let reads = listed.iter().take(MOST as usize).filter_map(|draft| draft["id"].as_str()).map(|id| async move {
        let found = api.get(&url(&format!("drafts/{}", segment(id))), &headers_query(format)).await?;
        Ok::<Value, Failure>(draft(&found))
    });
    let drafts = futures::future::join_all(reads).await.into_iter().collect::<Result<Vec<_>, _>>()?;
    Ok(json!({ "drafts": drafts, "nextPageToken": found["nextPageToken"] }))
}

/// A Gmail API draft as the MCP server's `Draft`.
fn draft(found: &Value) -> Value {
    let message = message(&found["message"]);
    let mut out = json!({
        "id": found["id"],
        "messageId": found["message"]["id"],
        "threadId": message["threadId"],
        "subject": message["subject"],
        "toRecipients": message["toRecipients"],
        "ccRecipients": message["ccRecipients"],
        "date": message["date"],
    });
    for key in ["plaintextBody", "htmlBody"] {
        if !message[key].is_null() {
            out[key] = message[key].clone();
        }
    }
    out
}

/// An email the call describes, ready to go: the Gmail API's `message` (its raw MIME and, for a
/// reply, the thread), and how it reads.
struct Outgoing {
    message: Value,
    subject: String,
    to: Vec<String>,
    cc: Vec<String>,
    bcc: Vec<String>,
}

async fn outgoing(api: &Api<'_>, args: &Value) -> Result<Outgoing, Failure> {
    let to = list(args, "to");
    let cc = list(args, "cc");
    let bcc = list(args, "bcc");
    let mut subject = text(args, "subject").unwrap_or_default();
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut thread_id = None;
    // A reply joins the message's thread, quoting its id, and keeps its subject.
    if let Some(reply_to) = text(args, "replyToMessageId") {
        let mut query = vec![("format", "metadata".to_string())];
        for header in ["Subject", "Message-ID", "References"] {
            query.push(("metadataHeaders", header.into()));
        }
        let original = api.get(&url(&format!("messages/{}", segment(&reply_to))), &query).await?;
        let found = original["payload"]["headers"].as_array().cloned().unwrap_or_default();
        let header = |name: &str| found.iter().find(|h| h["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name))).and_then(|h| h["value"].as_str()).map(str::to_string);
        if let Some(id) = header("Message-ID") {
            let references = header("References").map(|r| format!("{r} {id}")).unwrap_or_else(|| id.clone());
            headers.push(("In-Reply-To".into(), id));
            headers.push(("References".into(), references));
        }
        if subject.is_empty() {
            let original = header("Subject").unwrap_or_default();
            subject = if original.to_lowercase().starts_with("re:") { original } else { format!("Re: {original}") };
        }
        thread_id = original["threadId"].as_str().map(str::to_string);
    }
    let attachments: Vec<Attachment> = args["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            Ok(Attachment {
                filename: a["filename"].as_str().unwrap_or("attachment").to_string(),
                mime_type: a["mimeType"].as_str().unwrap_or("application/octet-stream").to_string(),
                content: STANDARD.decode(a["content"].as_str().unwrap_or_default().trim()).or_else(|_| URL_SAFE.decode(a["content"].as_str().unwrap_or_default().trim())).map_err(|_| format!("Attachment {} is not base64.", a["filename"].as_str().unwrap_or("attachment")))?,
            })
        })
        .collect::<Result<_, Failure>>()?;
    let raw = compose(&to, &cc, &bcc, &subject, &headers, text(args, "body").as_deref(), text(args, "htmlBody").as_deref(), &attachments);
    let mut message = json!({ "raw": URL_SAFE_NO_PAD.encode(raw) });
    if let Some(thread) = &thread_id {
        message["threadId"] = json!(thread);
    }
    Ok(Outgoing { message, subject, to, cc, bcc })
}

async fn create_draft(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let email = outgoing(api, args).await?;
    let made = api.post(&url("drafts"), &[], json!({ "message": email.message })).await?;
    Ok(json!({
        "id": made["id"],
        "messageId": made["message"]["id"],
        "threadId": made["message"]["threadId"],
        "subject": email.subject,
        "toRecipients": email.to,
        "ccRecipients": email.cc,
        "bccRecipients": email.bcc,
        "plaintextBody": text(args, "body"),
    }))
}

/// Sends an email at once (`messages.send`). Lorca's own tool, beside Google's: in a chat it
/// waits as a draft card like `create_draft`, and the card's Send makes this call.
async fn send_message(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let email = outgoing(api, args).await?;
    if email.to.is_empty() && email.cc.is_empty() && email.bcc.is_empty() {
        return Err("An email needs a recipient in to, cc, or bcc.".into());
    }
    let sent = api.post(&url("messages/send"), &[], email.message).await?;
    Ok(json!({
        "id": sent["id"],
        "threadId": sent["threadId"],
        "labelIds": sent["labelIds"],
        "subject": email.subject,
        "toRecipients": email.to,
        "ccRecipients": email.cc,
        "bccRecipients": email.bcc,
    }))
}

pub(super) struct Attachment {
    pub filename: String,
    pub mime_type: String,
    pub content: Vec<u8>,
}

/// An RFC 5322 message: the headers, then a text body, an HTML alternative, and attachments.
#[allow(clippy::too_many_arguments)]
pub(super) fn compose(to: &[String], cc: &[String], bcc: &[String], subject: &str, extra: &[(String, String)], plain: Option<&str>, html: Option<&str>, attachments: &[Attachment]) -> Vec<u8> {
    let mut head = String::from("MIME-Version: 1.0\r\n");
    for (name, values) in [("To", to), ("Cc", cc), ("Bcc", bcc)] {
        if !values.is_empty() {
            head.push_str(&format!("{name}: {}\r\n", values.join(", ")));
        }
    }
    head.push_str(&format!("Subject: {}\r\n", encoded_word(subject)));
    for (name, value) in extra {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    let text_part = |kind: &str, body: &str| format!("Content-Type: {kind}; charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n", wrapped(body.as_bytes()));
    let plain = plain.unwrap_or_default();
    let body = match html {
        Some(html) => {
            let boundary = boundary();
            format!("Content-Type: multipart/alternative; boundary=\"{boundary}\"\r\n\r\n--{boundary}\r\n{}--{boundary}\r\n{}--{boundary}--\r\n", text_part("text/plain", plain), text_part("text/html", html))
        }
        None => text_part("text/plain", plain),
    };
    let message = if attachments.is_empty() {
        format!("{head}{body}")
    } else {
        let boundary = boundary();
        let mut out = format!("{head}Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n--{boundary}\r\n{body}");
        for attachment in attachments {
            let name = attachment.filename.replace(['"', '\r', '\n'], "");
            out.push_str(&format!(
                "--{boundary}\r\nContent-Type: {}; name=\"{name}\"\r\nContent-Disposition: attachment; filename=\"{name}\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n",
                attachment.mime_type, wrapped(&attachment.content)
            ));
        }
        out.push_str(&format!("--{boundary}--\r\n"));
        out
    };
    message.into_bytes()
}

fn boundary() -> String {
    format!("lorca-{}", uuid::Uuid::new_v4().simple())
}

/// Base64 in lines of 76 characters.
fn wrapped(bytes: &[u8]) -> String {
    let encoded = STANDARD.encode(bytes);
    encoded.as_bytes().chunks(76).map(|line| std::str::from_utf8(line).unwrap_or_default()).collect::<Vec<_>>().join("\r\n")
}

/// A header value as it is when ASCII, else an RFC 2047 encoded word.
fn encoded_word(value: &str) -> String {
    let value = value.replace(['\r', '\n'], " ");
    if value.is_ascii() { value } else { format!("=?UTF-8?B?{}?=", STANDARD.encode(value.as_bytes())) }
}

/// An id as one path segment.
fn segment(id: &str) -> String {
    id.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect()
}
