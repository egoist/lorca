//! Google Drive's tools over the Drive API (v3). Files read as the MCP server's `File` (`title`,
//! `parentId`, `viewUrl`); Google Docs, Sheets, and Slides read as text by exporting them.

use std::sync::Arc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::{json, Value};

use super::{google_api, number, param, required, result, text, Api, Failure};
use crate::app::App;
use crate::plugins::builtin::Service;

pub struct Server;

/// The server of a Drive account, in its manifest.
const SERVER: &str = "drive";

/// The most a file's text or download carries, so one call cannot fill a turn.
const MOST_TEXT: usize = 1 << 20;
const MOST_BYTES: usize = 10 << 20;

const FILE_FIELDS: &str = "id,name,mimeType,size,description,fileExtension,webViewLink,sharedWithMeTime,createdTime,modifiedTime,viewedByMeTime,owners(displayName,emailAddress),parents,capabilities(canAddChildren)";

#[async_trait::async_trait]
impl Service for Server {
    fn name(&self) -> &'static str {
        "google-drive"
    }

    fn instructions(&self) -> Option<String> {
        Some("Google Drive for one account. search_files takes Drive's query syntax (name contains 'plan', fullText contains 'budget', mimeType = 'application/vnd.google-apps.document', modifiedTime > '2026-01-01T00:00:00', 'FOLDER_ID' in parents); plain words search the files' text. read_file_content gives a file as text; download_file_content gives its bytes as base64.".into())
    }

    fn tools(&self, _app: &Arc<App>, _plugin_id: &str) -> Vec<Value> {
        super::tools("drive")
    }

    async fn call(&self, app: &Arc<App>, plugin_id: &str, tool: &str, args: Value, _context: &Value) -> Value {
        let api = Api::new(app, plugin_id, SERVER);
        result(match tool {
            "search_files" => search_files(&api, &args).await,
            "list_recent_files" => list_recent_files(&api, &args).await,
            "get_file_metadata" => get_file_metadata(&api, &args).await,
            "get_file_permissions" => get_file_permissions(&api, &args).await,
            "read_file_content" => read_file_content(&api, &args).await,
            "download_file_content" => download_file_content(&api, &args).await,
            "create_file" => create_file(&api, &args).await,
            "copy_file" => copy_file(&api, &args).await,
            _ => Err(format!("Google Drive has no tool {tool}.")),
        })
    }
}

fn files_url() -> String {
    format!("{}/drive/v3/files", google_api())
}

fn file_url(id: &str) -> String {
    format!("{}/{}", files_url(), id.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect::<String>())
}

/// A Drive API file as the MCP server's `File`.
fn file(found: &Value) -> Value {
    json!({
        "id": found["id"],
        "title": found["name"],
        "parentId": found["parents"][0],
        "mimeType": found["mimeType"],
        "fileSize": found["size"],
        "description": found["description"],
        "fileExtension": found["fileExtension"],
        "viewUrl": found["webViewLink"],
        "sharedWithMeTime": found["sharedWithMeTime"],
        "createdTime": found["createdTime"],
        "modifiedTime": found["modifiedTime"],
        "viewedByMeTime": found["viewedByMeTime"],
        "owner": found["owners"][0]["emailAddress"],
        "canAddChildren": found["capabilities"]["canAddChildren"],
    })
}

fn listing(args: &Value, query: &mut Vec<(&'static str, String)>) {
    query.push(("fields", format!("nextPageToken,files({FILE_FIELDS})")));
    query.push(("supportsAllDrives", "true".into()));
    query.push(("includeItemsFromAllDrives", "true".into()));
    param(query, "pageSize", number(args, "pageSize"));
    param(query, "pageToken", text(args, "pageToken"));
}

fn files(found: &Value) -> Value {
    let files: Vec<Value> = found["files"].as_array().into_iter().flatten().map(file).collect();
    json!({ "files": files, "nextPageToken": found["nextPageToken"] })
}

/// A query in Drive's syntax. Plain words search the files' text; the MCP server's names for
/// fields (`title`, `parentId`, `owner`, `sharedWithMe`) read as Drive's. Trashed files stay out
/// unless the query names `trashed`.
pub(super) fn drive_query(query: &str) -> String {
    let query = query.trim();
    let operators = [" contains ", "=", "<", ">", " in ", " has ", " and ", " or ", " not "];
    let mut q = if query.is_empty() {
        String::new()
    } else if !operators.iter().any(|op| query.contains(op)) && !query.contains("sharedWithMe") {
        format!("fullText contains '{}'", query.replace('\\', "\\\\").replace('\'', "\\'"))
    } else {
        let mut q = query.replace("title ", "name ").replace("title=", "name=");
        // parentId = 'x'  →  'x' in parents; owner = 'x'  →  'x' in owners.
        for (field, set) in [("parentId", "parents"), ("owner", "owners")] {
            while let Some(at) = q.find(field) {
                let rest = &q[at + field.len()..];
                let Some(after_eq) = rest.trim_start().strip_prefix('=') else { break };
                let value = after_eq.trim_start();
                let Some(end) = value.strip_prefix('\'').and_then(|v| v.find('\'')) else { break };
                let quoted = &value[..end + 2];
                let consumed = q.len() - (value.len() - quoted.len());
                q = format!("{}{quoted} in {set}{}", &q[..at], &q[consumed..]);
            }
        }
        if q.contains("sharedWithMe") && !q.contains("sharedWithMe =") && !q.contains("sharedWithMe=") {
            q = q.replace("sharedWithMe", "sharedWithMe = true");
        }
        q
    };
    if !q.contains("trashed") {
        q = if q.is_empty() { "trashed = false".into() } else { format!("({q}) and trashed = false") };
    }
    q
}

async fn search_files(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let mut query = vec![("q", drive_query(&text(args, "query").unwrap_or_default()))];
    listing(args, &mut query);
    Ok(files(&api.get(&files_url(), &query).await?))
}

async fn list_recent_files(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let order = match args["orderBy"].as_str() {
        Some("lastModified") => "modifiedTime desc",
        Some("lastModifiedByMe") => "modifiedByMeTime desc",
        _ => "recency desc",
    };
    let mut query = vec![("q", "trashed = false".to_string()), ("orderBy", order.to_string())];
    listing(args, &mut query);
    Ok(files(&api.get(&files_url(), &query).await?))
}

async fn metadata(api: &Api<'_>, id: &str) -> Result<Value, Failure> {
    api.get(&file_url(id), &[("fields", FILE_FIELDS.to_string()), ("supportsAllDrives", "true".into())]).await
}

async fn get_file_metadata(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    Ok(file(&metadata(api, &required(args, "fileId")?).await?))
}

async fn get_file_permissions(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let id = required(args, "fileId")?;
    let query = [("fields", "permissions(role,displayName,type,emailAddress,view)".to_string()), ("supportsAllDrives", "true".into())];
    let found = api.get(&format!("{}/permissions", file_url(&id)), &query).await?;
    Ok(json!({ "permissions": found["permissions"].as_array().cloned().unwrap_or_default() }))
}

/// The type a Google file exports to as text, when it has one.
fn text_export(mime: &str) -> Option<&'static str> {
    match mime {
        "application/vnd.google-apps.document" => Some("text/plain"),
        "application/vnd.google-apps.spreadsheet" => Some("text/csv"),
        "application/vnd.google-apps.presentation" => Some("text/plain"),
        "application/vnd.google-apps.script" => Some("application/vnd.google-apps.script+json"),
        _ => None,
    }
}

fn readable(mime: &str) -> bool {
    mime.starts_with("text/") || matches!(mime, "application/json" | "application/xml" | "application/javascript" | "application/x-yaml" | "application/yaml" | "application/csv")
}

async fn read_file_content(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let id = required(args, "fileId")?;
    let found = metadata(api, &id).await?;
    let mime = found["mimeType"].as_str().unwrap_or_default();
    let content = if let Some(export) = text_export(mime) {
        Some(api.bytes(&format!("{}/export", file_url(&id)), &[("mimeType", export.to_string())]).await?.0)
    } else if readable(mime) {
        Some(api.bytes(&file_url(&id), &[("alt", "media".to_string()), ("supportsAllDrives", "true".into())]).await?.0)
    } else {
        None
    };
    let mut out = json!({ "id": found["id"], "title": found["name"], "mimeType": mime, "viewUrl": found["webViewLink"] });
    match content {
        Some(bytes) => {
            let mut text = String::from_utf8_lossy(&bytes).into_owned();
            if text.len() > MOST_TEXT {
                let mut cut = MOST_TEXT;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                text.truncate(cut);
                out["truncated"] = json!(true);
            }
            out["fileContent"] = json!(text);
        }
        None => out["textFormattingNotSupported"] = json!(true),
    }
    if args["includeComments"].as_bool() == Some(true) {
        let query = [("fields", "comments(id,content,quotedFileContent,anchor,resolved,author(displayName,emailAddress),createdTime,replies(content,author(displayName,emailAddress),createdTime))".to_string()), ("pageSize", "100".into())];
        match api.get(&format!("{}/comments", file_url(&id)), &query).await {
            Ok(found) => {
                let thread = |c: &Value| json!({ "commentId": c["id"], "status": if c["resolved"].as_bool() == Some(true) { "resolved" } else { "open" }, "headPost": { "content": c["content"], "author": c["author"], "createdTime": c["createdTime"] }, "quotedContent": c["quotedFileContent"]["value"], "replies": c["replies"] });
                let comments = found["comments"].as_array().cloned().unwrap_or_default();
                let (anchored, unanchored): (Vec<&Value>, Vec<&Value>) = comments.iter().partition(|c| c["anchor"].is_string() || c["quotedFileContent"].is_object());
                out["contentAnchoredComments"] = json!(anchored.into_iter().map(thread).collect::<Vec<_>>());
                out["unanchoredComments"] = json!(unanchored.into_iter().map(thread).collect::<Vec<_>>());
            }
            Err(_) => out["commentsNotSupported"] = json!(true),
        }
    }
    Ok(out)
}

/// The type a Google file downloads as when the call names none.
fn default_export(mime: &str) -> Option<&'static str> {
    match mime {
        "application/vnd.google-apps.document" | "application/vnd.google-apps.presentation" => Some("application/pdf"),
        "application/vnd.google-apps.spreadsheet" => Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        "application/vnd.google-apps.drawing" => Some("image/png"),
        _ => None,
    }
}

async fn download_file_content(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let id = required(args, "fileId")?;
    let found = metadata(api, &id).await?;
    let mime = found["mimeType"].as_str().unwrap_or_default();
    let (bytes, kind) = if mime.starts_with("application/vnd.google-apps.") {
        let export = text(args, "exportMimeType").or_else(|| default_export(mime).map(str::to_string)).ok_or_else(|| format!("{mime} files cannot be downloaded; name an exportMimeType."))?;
        api.bytes(&format!("{}/export", file_url(&id)), &[("mimeType", export)]).await?
    } else {
        if found["size"].as_str().and_then(|s| s.parse::<usize>().ok()).is_some_and(|size| size > MOST_BYTES) {
            return Err(format!("The file is over {} MB, more than one call carries. Open it from its viewUrl: {}", MOST_BYTES >> 20, found["webViewLink"].as_str().unwrap_or_default()));
        }
        api.bytes(&file_url(&id), &[("alt", "media".to_string()), ("supportsAllDrives", "true".into())]).await?
    };
    if bytes.len() > MOST_BYTES {
        return Err(format!("The file is over {} MB, more than one call carries.", MOST_BYTES >> 20));
    }
    Ok(json!({ "id": found["id"], "title": found["name"], "mimeType": kind.split(';').next().unwrap_or(&kind).trim(), "content": STANDARD.encode(bytes) }))
}

/// The Google type an upload of this type becomes, unless the call keeps it.
fn google_type(mime: &str) -> Option<&'static str> {
    match mime {
        "text/plain" | "text/html" | "text/markdown" | "application/rtf" | "application/vnd.openxmlformats-officedocument.wordprocessingml.document" | "application/msword" | "application/vnd.oasis.opendocument.text" => Some("application/vnd.google-apps.document"),
        "text/csv" | "text/tab-separated-values" | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" | "application/vnd.ms-excel" | "application/vnd.oasis.opendocument.spreadsheet" => Some("application/vnd.google-apps.spreadsheet"),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" | "application/vnd.ms-powerpoint" | "application/vnd.oasis.opendocument.presentation" => Some("application/vnd.google-apps.presentation"),
        _ => None,
    }
}

async fn create_file(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let title = text(args, "title").unwrap_or_else(|| "Untitled".into());
    let content: Option<Vec<u8>> = match (args["textContent"].as_str(), text(args, "base64Content").or_else(|| text(args, "content"))) {
        (Some(words), _) => Some(words.as_bytes().to_vec()),
        (None, Some(encoded)) => Some(STANDARD.decode(encoded.trim()).map_err(|_| "The content is not base64.".to_string())?),
        (None, None) => None,
    };
    let given = text(args, "contentMimeType").or_else(|| text(args, "mimeType"));
    let mut metadata = json!({ "name": title });
    if let Some(parent) = text(args, "parentId") {
        metadata["parents"] = json!([parent]);
    }
    let query = format!("?fields={FILE_FIELDS}&supportsAllDrives=true");
    let made = match content {
        Some(bytes) => {
            let kind = given.unwrap_or_else(|| if args["textContent"].is_string() { "text/plain".into() } else { "application/octet-stream".into() });
            if args["disableConversionToGoogleType"].as_bool() != Some(true) {
                if let Some(google) = google_type(&kind) {
                    metadata["mimeType"] = json!(google);
                }
            }
            api.upload(&format!("{}/upload/drive/v3/files{query}", google_api()), metadata, &bytes, &kind).await?
        }
        // No content: an empty file of the type given, such as a folder or a blank Doc.
        None => {
            if let Some(kind) = given {
                metadata["mimeType"] = json!(kind);
            }
            api.post(&format!("{}{query}", files_url()), &[], metadata).await?
        }
    };
    Ok(file(&made))
}

async fn copy_file(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let id = required(args, "fileId")?;
    let mut body = json!({});
    if let Some(title) = text(args, "title") {
        body["name"] = json!(title);
    }
    if let Some(parent) = text(args, "parentId") {
        body["parents"] = json!([parent]);
    }
    let query = [("fields", FILE_FIELDS.to_string()), ("supportsAllDrives", "true".into())];
    Ok(file(&api.post(&format!("{}/copy", file_url(&id)), &query, body).await?))
}
