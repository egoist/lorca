//! Gmail, Calendar, and Drive over a stand-in for Google's REST APIs: the index's entries, the
//! tools by Google's MCP names, what each call asks of the API, and what a refused sign-in reads as.

use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use serde_json::{json, Value};

use crate::app::App;
use crate::plugins::builtin::Service;
use crate::plugins::{AuthSpec, ServerSpec};

struct Scratch(Arc<App>, std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

fn scratch() -> Scratch {
    let home = std::env::temp_dir().join(format!("lorca-google-{}", uuid::Uuid::new_v4()));
    let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
    Scratch(app, home)
}

/// One request the stand-in took.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    bearer: String,
    content_type: String,
    body: Vec<u8>,
}

impl Seen {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    fn param(&self, key: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

type Requests = Arc<Mutex<Vec<Seen>>>;

/// Google's APIs as these tests need them, for whichever service a path names.
async fn google(State(seen): State<Requests>, method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Response {
    let bearer = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().trim_start_matches("Bearer ").to_string();
    let query: Vec<(String, String)> = uri.query().map(|q| reqwest::Url::parse(&format!("http://x/?{q}")).unwrap().query_pairs().into_owned().collect()).unwrap_or_default();
    let path = uri.path().to_string();
    let request = Seen { method: method.to_string(), path: path.clone(), query, bearer: bearer.clone(), content_type: headers.get("content-type").and_then(|v| v.to_str().ok()).unwrap_or_default().into(), body: body.to_vec() };
    seen.lock().unwrap().push(request.clone());
    if bearer == "revoked" {
        return (StatusCode::UNAUTHORIZED, axum::Json(json!({ "error": { "code": 401, "message": "Request had invalid authentication credentials.", "status": "UNAUTHENTICATED" } }))).into_response();
    }
    if bearer == "narrow" {
        let challenge = "Bearer realm=\"https://accounts.google.com/\", error=\"insufficient_scope\", scope=\"https://www.googleapis.com/auth/gmail.modify\"";
        return (StatusCode::FORBIDDEN, [("www-authenticate", challenge)], axum::Json(json!({ "error": { "code": 403, "message": "Request had insufficient authentication scopes.", "details": [{ "reason": "ACCESS_TOKEN_SCOPE_INSUFFICIENT" }] } }))).into_response();
    }
    let b64 = |text: &str| URL_SAFE_NO_PAD.encode(text);
    let message = json!({
        "id": "m1", "threadId": "t1", "snippet": "Plan attached", "labelIds": ["INBOX", "UNREAD"],
        "payload": {
            "mimeType": "multipart/mixed",
            "headers": [
                { "name": "Subject", "value": "Plan" }, { "name": "From", "value": "Ariel <ariel@example.com>" },
                { "name": "To", "value": "me@example.com, team@example.com" }, { "name": "Date", "value": "Thu, 15 Oct 2026 09:00:00 +0000" },
                { "name": "Message-ID", "value": "<plan@example.com>" },
            ],
            "parts": [
                { "mimeType": "multipart/alternative", "parts": [
                    { "mimeType": "text/plain", "body": { "data": b64("The plan is attached.") } },
                    { "mimeType": "text/html", "body": { "data": b64("<p>The plan is attached.</p>") } },
                ] },
                { "mimeType": "application/pdf", "filename": "plan.pdf", "body": { "attachmentId": "att1", "size": 10 } },
            ],
        },
    });
    let event = json!({
        "id": "e1", "status": "confirmed", "summary": "Customer call", "htmlLink": "https://calendar.google.com/event?eid=e1",
        "start": { "dateTime": "2026-10-19T15:00:00Z" }, "end": { "dateTime": "2026-10-19T15:30:00Z" },
        "conferenceData": { "entryPoints": [{ "entryPointType": "video", "uri": "https://meet.google.com/abc-defg-hij" }] },
        "attendees": [{ "email": "me@example.com", "self": true, "responseStatus": "needsAction" }, { "email": "ariel@example.com", "responseStatus": "accepted" }],
    });
    let doc = json!({ "id": "f1", "name": "Plan", "mimeType": "application/vnd.google-apps.document", "webViewLink": "https://docs.google.com/document/d/f1", "parents": ["root"], "owners": [{ "emailAddress": "me@example.com" }] });
    let answer = match (method.as_str(), path.as_str()) {
        ("GET", "/gmail/v1/users/me/threads") => json!({ "threads": [{ "id": "t1", "snippet": "Plan attached" }], "nextPageToken": "next", "resultSizeEstimate": 1 }),
        ("GET", "/gmail/v1/users/me/threads/t1") => json!({ "id": "t1", "messages": [message] }),
        ("GET", "/gmail/v1/users/me/messages/m1") => message,
        ("POST", "/gmail/v1/users/me/drafts") => json!({ "id": "d1", "message": { "id": "m2", "threadId": request.json()["message"]["threadId"] } }),
        ("POST", "/gmail/v1/users/me/messages/m1/modify") => json!({ "id": "m1" }),
        ("GET", "/gmail/v1/users/me/labels") => json!({ "labels": [{ "id": "INBOX", "name": "INBOX", "type": "system" }, { "id": "Label_1", "name": "Receipts", "type": "user" }] }),
        ("GET", "/calendar/v3/calendars/primary/events") => json!({ "summary": "me@example.com", "timeZone": "UTC", "items": [event] }),
        ("GET", "/calendar/v3/calendars/primary/events/e1") => event,
        ("PATCH", "/calendar/v3/calendars/primary/events/e1") => {
            let mut patched = event;
            for (key, value) in request.json().as_object().cloned().unwrap_or_default() {
                patched[key] = value;
            }
            patched
        }
        ("POST", "/calendar/v3/calendars/primary/events") => {
            let mut made = request.json();
            made["id"] = json!("e2");
            made
        }
        ("POST", "/calendar/v3/freeBusy") => json!({ "calendars": {
            "ariel@example.com": { "busy": [{ "start": "2026-10-19T09:00:00Z", "end": "2026-10-19T10:00:00Z" }] },
            "primary": { "busy": [{ "start": "2026-10-19T10:00:00Z", "end": "2026-10-19T10:30:00Z" }] },
        } }),
        ("GET", "/drive/v3/files") => json!({ "files": [doc] }),
        ("GET", "/drive/v3/files/f1") => doc,
        ("GET", "/drive/v3/files/f1/export") => return (StatusCode::OK, [("content-type", "text/plain")], "The plan, as text.").into_response(),
        ("POST", "/upload/drive/v3/files") => json!({ "id": "f2", "name": "Notes", "mimeType": "application/vnd.google-apps.document", "webViewLink": "https://docs.google.com/document/d/f2" }),
        _ => return (StatusCode::NOT_FOUND, axum::Json(json!({ "error": { "code": 404, "message": format!("No {method} {path} here") } }))).into_response(),
    };
    axum::Json(answer).into_response()
}

static GOOGLE_API: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Starts the stand-in and points the services at it until the guard drops.
async fn google_stub() -> (Requests, tokio::sync::MutexGuard<'static, ()>) {
    let guard = GOOGLE_API.lock().await;
    let seen: Requests = Arc::default();
    let router = Router::new().fallback(google).with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    std::env::set_var("LORCA_GOOGLE_API_URL", format!("http://{address}"));
    (seen, guard)
}

/// An account of the service from the bundled index, signed in with `token`, which is fresh.
fn account(app: &Arc<App>, service: &str, server: &str, token: &str) -> String {
    let manifest = crate::marketplace::bundled().plugins.into_iter().find(|m| m.id == service).unwrap();
    let status = crate::plugins::accounts::install(app, manifest, "marketplace", None).unwrap();
    let saved = json!({ "native_flow": true, "client_id": "own.apps.googleusercontent.com", "client_secret": "own-secret", "token_endpoint": "https://oauth2.googleapis.com/token", "signed_in_at": crate::config::now_secs(), "tokens": { "access_token": token, "refresh_token": "refresh", "expires_in": 3600, "token_type": "Bearer" } });
    crate::plugins::set_oauth(app, &status.id, server, Some(saved)).unwrap();
    status.id
}

async fn call(service: &dyn Service, app: &Arc<App>, id: &str, tool: &str, args: Value) -> Value {
    service.call(app, id, tool, args, &Value::Null).await
}

fn ok(result: &Value) -> &Value {
    assert_eq!(result["isError"], false, "{result}");
    &result["structuredContent"]
}

fn find(seen: &Requests, matches: impl Fn(&Seen) -> bool) -> Seen {
    seen.lock().unwrap().iter().find(|s| matches(s)).cloned().expect("the request went out")
}

#[test]
fn the_index_runs_each_service_here_with_its_google_sign_in_and_older_builds_keep_the_http_server() {
    let raw: Value = serde_json::from_str(include_str!("../../../marketplace/index.json")).unwrap();
    let scratch = scratch();
    for (service, server) in [("gmail", "gmail"), ("google-calendar", "calendar"), ("google-drive", "drive")] {
        let manifest = crate::marketplace::bundled().plugins.into_iter().find(|m| m.id == service).unwrap();
        let Some(ServerSpec::Builtin { service: runs, auth: Some(AuthSpec::Oauth { client_id_variable, scopes, authorization_endpoint, .. }), .. }) = manifest.servers.get(server) else {
            panic!("{service}: {:?}", manifest.servers.get(server));
        };
        assert_eq!((runs.as_str(), client_id_variable.as_deref()), (service, Some("GOOGLE_CLIENT_ID")));
        assert!(authorization_endpoint.is_some() && !scopes.is_empty());
        // A build without these servers reads the entry's HTTP server, as before.
        let entry = raw["plugins"].as_array().unwrap().iter().find(|p| p["id"] == service).unwrap();
        assert_eq!(entry["servers"][server]["type"], "http");
        assert_eq!(entry["builtin_servers"][server], service);
        // Every hinted tool is one the server offers.
        let offered: Vec<String> = super::service(service).unwrap().tools(&scratch.0, "x").iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        for hinted in manifest.tools.readonly.iter().chain(manifest.tools.draft.iter()).filter(|name| !name.contains('.')) {
            assert!(offered.contains(hinted), "{service} hints {hinted}, which it does not offer");
        }
    }
}

#[test]
fn the_tools_take_googles_mcp_arguments() {
    let scratch = scratch();
    let tool = |service: &str, name: &str| super::service(service).unwrap().tools(&scratch.0, "x").into_iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("{service} {name}"));
    let draft = tool("gmail", "create_draft");
    for key in ["to", "cc", "bcc", "subject", "body", "attachments", "replyToMessageId"] {
        assert!(draft["inputSchema"]["properties"][key].is_object(), "create_draft {key}");
    }
    let events = tool("google-calendar", "list_events");
    for key in ["startTime", "endTime", "orderBy", "pageSize", "fullText"] {
        assert!(events["inputSchema"]["properties"][key].is_object(), "list_events {key}");
    }
    assert_eq!(events["annotations"]["readOnlyHint"], true);
    assert_eq!(tool("google-calendar", "delete_event")["annotations"]["readOnlyHint"], false);
}

#[tokio::test]
async fn an_account_needs_its_own_client_then_reads_sign_in_and_does_not_connect() {
    let scratch = scratch();
    let app = &scratch.0;
    let manifest = crate::marketplace::bundled().plugins.into_iter().find(|m| m.id == "gmail").unwrap();
    let status = crate::plugins::accounts::install(app, manifest, "marketplace", None).unwrap();
    assert_eq!(status.state, "needs_setup", "the user's own Google client comes first: {status:?}");
    let values = [("GOOGLE_CLIENT_ID".to_string(), "own.apps.googleusercontent.com".to_string())].into_iter().collect();
    let status = crate::plugins::set_variables(app, &status.id, &values).unwrap();
    assert_eq!(status.state, "needs_auth", "{status:?}");
    let error = app.mcp.server(app, &status.id, "gmail").await.err().unwrap_or_default();
    assert!(error.contains("Sign in to"), "{error}");
}

#[tokio::test]
async fn a_signed_in_account_connects_through_the_pool_with_googles_tool_names() {
    let scratch = scratch();
    let app = &scratch.0;
    let id = account(app, "gmail", "gmail", "gmail-token");
    let server = app.mcp.server(app, &id, "gmail").await.unwrap();
    let names: Vec<String> = server.tools().iter().map(|t| t.name.to_string()).collect();
    for name in ["search_threads", "get_thread", "create_draft", "list_labels", "label_thread"] {
        assert!(names.contains(&name.to_string()), "{names:?}");
    }
}

#[tokio::test]
async fn gmail_searches_reads_drafts_replies_and_labels_with_the_accounts_token() {
    let (seen, _guard) = google_stub().await;
    let scratch = scratch();
    let app = &scratch.0;
    let id = account(app, "gmail", "gmail", "gmail-token");
    let gmail = super::service("gmail").unwrap();

    let found = call(&*gmail, app, &id, "search_threads", json!({ "query": "from:ariel newer_than:7d", "pageSize": 5 })).await;
    let found = ok(&found);
    assert_eq!((found["nextPageToken"].as_str(), found["resultCountEstimate"].as_u64()), (Some("next"), Some(1)));
    let first = &found["threads"][0]["messages"][0];
    assert_eq!((first["subject"].as_str(), first["sender"].as_str()), (Some("Plan"), Some("Ariel <ariel@example.com>")));
    assert_eq!(first["toRecipients"], json!(["me@example.com", "team@example.com"]));
    let search = find(&seen, |s| s.path == "/gmail/v1/users/me/threads");
    assert_eq!((search.param("q"), search.param("maxResults"), search.bearer.as_str()), (Some("from:ariel newer_than:7d"), Some("5"), "gmail-token"));
    assert_eq!(find(&seen, |s| s.path == "/gmail/v1/users/me/threads/t1").param("format"), Some("metadata"), "a search reads its threads' headers only");

    let thread = call(&*gmail, app, &id, "get_thread", json!({ "threadId": "t1" })).await;
    let message = &ok(&thread)["messages"][0];
    assert_eq!(message["plaintextBody"], "The plan is attached.");
    assert_eq!(message["htmlBody"], "<p>The plan is attached.</p>");
    assert_eq!(message["attachments"], json!([{ "id": "att1", "filename": "plan.pdf", "mimeType": "application/pdf" }]));

    // A reply joins the thread, quotes the message, and keeps its subject; what it made is a
    // draft the card's Send finds by id.
    let made = call(&*gmail, app, &id, "create_draft", json!({ "to": ["ariel@example.com"], "body": "Looks good, ship it.", "replyToMessageId": "m1" })).await;
    let made = ok(&made);
    assert_eq!((made["id"].as_str(), made["threadId"].as_str(), made["subject"].as_str()), (Some("d1"), Some("t1"), Some("Re: Plan")));
    let posted = find(&seen, |s| s.path == "/gmail/v1/users/me/drafts").json();
    assert_eq!(posted["message"]["threadId"], "t1");
    let raw = String::from_utf8(URL_SAFE_NO_PAD.decode(posted["message"]["raw"].as_str().unwrap()).unwrap()).unwrap();
    assert!(raw.contains("To: ariel@example.com\r\n") && raw.contains("Subject: Re: Plan\r\n"), "{raw}");
    assert!(raw.contains("In-Reply-To: <plan@example.com>\r\nReferences: <plan@example.com>\r\n"), "{raw}");
    assert!(raw.contains(&STANDARD.encode("Looks good, ship it.")), "{raw}");

    ok(&call(&*gmail, app, &id, "label_message", json!({ "messageId": "m1", "labelIds": ["Label_1"] })).await);
    assert_eq!(find(&seen, |s| s.path.ends_with("/messages/m1/modify")).json(), json!({ "addLabelIds": ["Label_1"] }));
    let labels = call(&*gmail, app, &id, "list_labels", json!({})).await;
    assert_eq!((ok(&labels)["labels"][1]["labelId"].as_str(), ok(&labels)["labels"][1]["name"].as_str()), (Some("Label_1"), Some("Receipts")));
}

#[test]
fn a_draft_with_html_and_an_attachment_is_a_mixed_message() {
    let attachment = super::gmail::Attachment { filename: "notes.txt".into(), mime_type: "text/plain".into(), content: b"hello".to_vec() };
    let raw = String::from_utf8(super::gmail::compose(&["a@example.com".into()], &[], &["b@example.com".into()], "Café", &[], Some("Hi"), Some("<b>Hi</b>"), &[attachment])).unwrap();
    assert!(raw.contains("Bcc: b@example.com\r\n"));
    assert!(raw.contains(&format!("Subject: =?UTF-8?B?{}?=\r\n", STANDARD.encode("Café"))), "{raw}");
    assert!(raw.contains("Content-Type: multipart/mixed;") && raw.contains("Content-Type: multipart/alternative;"));
    assert!(raw.contains("Content-Disposition: attachment; filename=\"notes.txt\"") && raw.contains(&STANDARD.encode("hello")));
}

#[tokio::test]
async fn calendar_lists_with_the_routines_arguments_creates_answers_and_suggests_times() {
    let (seen, _guard) = google_stub().await;
    let scratch = scratch();
    let app = &scratch.0;
    let id = account(app, "google-calendar", "calendar", "calendar-token");
    let calendar = super::service("google-calendar").unwrap();

    // What a calendar routine sends (`routine_triggers`).
    let listed = call(&*calendar, app, &id, "list_events", json!({ "startTime": "2026-10-19T00:00:00Z", "endTime": "2026-10-20T00:00:00Z", "orderBy": "startTime", "pageSize": 100, "fullText": "customer call" })).await;
    let listed = ok(&listed);
    assert_eq!(listed["events"][0]["conferenceUrl"], "https://meet.google.com/abc-defg-hij");
    assert!(listed["events"][0]["start"].is_object() && listed["events"][0]["end"].is_object(), "a routine finds events by their start and end");
    let request = find(&seen, |s| s.method == "GET" && s.path == "/calendar/v3/calendars/primary/events");
    assert_eq!(
        (request.param("timeMin"), request.param("timeMax"), request.param("q"), request.param("orderBy"), request.param("singleEvents"), request.param("maxResults")),
        (Some("2026-10-19T00:00:00Z"), Some("2026-10-20T00:00:00Z"), Some("customer call"), Some("startTime"), Some("true"), Some("100"))
    );

    let made = call(&*calendar, app, &id, "create_event", json!({ "summary": "Design review", "startTime": "2026-10-20T10:00:00", "endTime": "2026-10-20T11:00:00", "timeZone": "Europe/Zurich", "attendeeEmails": ["ariel@example.com"], "addGoogleMeetUrl": true, "notificationLevel": "ALL" })).await;
    ok(&made);
    let posted = find(&seen, |s| s.method == "POST" && s.path == "/calendar/v3/calendars/primary/events");
    let body = posted.json();
    assert_eq!(body["start"], json!({ "dateTime": "2026-10-20T10:00:00+02:00", "timeZone": "Europe/Zurich" }));
    assert_eq!(body["attendees"], json!([{ "email": "ariel@example.com" }]));
    assert_eq!(body["conferenceData"]["createRequest"]["conferenceSolutionKey"]["type"], "hangoutsMeet");
    assert_eq!((posted.param("sendUpdates"), posted.param("conferenceDataVersion")), (Some("all"), Some("1")));

    ok(&call(&*calendar, app, &id, "respond_to_event", json!({ "eventId": "e1", "responseStatus": "accepted" })).await);
    let patch = find(&seen, |s| s.method == "PATCH").json();
    assert_eq!(patch["attendees"][0], json!({ "email": "me@example.com", "self": true, "responseStatus": "accepted" }));
    assert_eq!(patch["attendees"][1]["responseStatus"], "accepted", "the others' answers stay");

    let suggested = call(&*calendar, app, &id, "suggest_time", json!({ "attendeeEmails": ["ariel@example.com"], "startTime": "2026-10-19T08:00:00Z", "endTime": "2026-10-19T17:00:00Z", "durationMinutes": 60, "preferences": { "pageSize": 2 } })).await;
    let slots: Vec<&str> = ok(&suggested)["timeSlots"].as_array().unwrap().iter().map(|s| s["startTime"].as_str().unwrap()).collect();
    // 09:00–10:30 is busy for one of them, and the working day starts at nine.
    assert_eq!(slots, ["2026-10-19T10:30:00+00:00", "2026-10-19T11:30:00+00:00"]);
    assert_eq!(find(&seen, |s| s.path == "/calendar/v3/freeBusy").json()["items"], json!([{ "id": "ariel@example.com" }, { "id": "primary" }]));
}

#[test]
fn free_time_skips_weekends_and_the_hours_outside_the_day() {
    use chrono::{NaiveTime, TimeZone, Utc};
    let (from, to) = (Utc.with_ymd_and_hms(2026, 10, 16, 16, 0, 0).unwrap(), Utc.with_ymd_and_hms(2026, 10, 19, 10, 0, 0).unwrap());
    let (nine, five) = (NaiveTime::from_hms_opt(9, 0, 0).unwrap(), NaiveTime::from_hms_opt(17, 0, 0).unwrap());
    let slots = super::calendar::free_slots(from, to, &[], chrono_tz::UTC, nine, five, true, chrono::Duration::minutes(60), 5);
    let starts: Vec<&str> = slots.iter().map(|s| s["startTime"].as_str().unwrap()).collect();
    // Friday 16:00–17:00, then Monday from nine; nothing on the weekend.
    assert_eq!(starts, ["2026-10-16T16:00:00+00:00", "2026-10-19T09:00:00+00:00"]);
}

#[tokio::test]
async fn drive_searches_in_its_syntax_reads_docs_as_text_and_uploads_as_docs() {
    let (seen, _guard) = google_stub().await;
    let scratch = scratch();
    let app = &scratch.0;
    let id = account(app, "google-drive", "drive", "drive-token");
    let drive = super::service("google-drive").unwrap();

    let found = call(&*drive, app, &id, "search_files", json!({ "query": "quarterly plan" })).await;
    let first = &ok(&found)["files"][0];
    assert_eq!((first["title"].as_str(), first["parentId"].as_str(), first["viewUrl"].as_str(), first["owner"].as_str()), (Some("Plan"), Some("root"), Some("https://docs.google.com/document/d/f1"), Some("me@example.com")));
    assert_eq!(find(&seen, |s| s.path == "/drive/v3/files").param("q"), Some("(fullText contains 'quarterly plan') and trashed = false"));

    let read = call(&*drive, app, &id, "read_file_content", json!({ "fileId": "f1" })).await;
    assert_eq!(ok(&read)["fileContent"], "The plan, as text.");
    assert_eq!(find(&seen, |s| s.path.ends_with("/export")).param("mimeType"), Some("text/plain"));

    let made = call(&*drive, app, &id, "create_file", json!({ "title": "Notes", "textContent": "First draft", "parentId": "folder1" })).await;
    assert_eq!(ok(&made)["title"], "Notes");
    let upload = find(&seen, |s| s.path == "/upload/drive/v3/files");
    assert_eq!(upload.param("uploadType"), Some("multipart"));
    assert!(upload.content_type.starts_with("multipart/related; boundary="));
    let body = String::from_utf8_lossy(&upload.body);
    assert!(body.contains(r#""mimeType":"application/vnd.google-apps.document""#) && body.contains(r#""parents":["folder1"]"#) && body.contains("Content-Type: text/plain\r\n\r\nFirst draft"), "{body}");
    assert!(seen.lock().unwrap().iter().all(|s| s.bearer == "drive-token"));
}

#[test]
fn drive_queries_read_googles_mcp_names_as_drives() {
    use super::drive::drive_query;
    assert_eq!(drive_query(""), "trashed = false");
    assert_eq!(drive_query("title contains 'Plan' and parentId = 'abc'"), "(name contains 'Plan' and 'abc' in parents) and trashed = false");
    assert_eq!(drive_query("owner = 'me'"), "('me' in owners) and trashed = false");
    assert_eq!(drive_query("sharedWithMe"), "(sharedWithMe = true) and trashed = false");
    assert_eq!(drive_query("trashed = true"), "trashed = true");
    assert_eq!(drive_query("Bob's notes"), "(fullText contains 'Bob\\'s notes') and trashed = false");
}

#[tokio::test]
async fn a_refused_sign_in_reads_as_one_and_a_narrow_one_as_needing_more_access() {
    let (_seen, _guard) = google_stub().await;
    let scratch = scratch();
    let app = &scratch.0;
    let gmail = super::service("gmail").unwrap();
    let revoked = account(app, "gmail", "gmail", "revoked");
    let failed = call(&*gmail, app, &revoked, "list_labels", json!({})).await;
    assert_eq!(failed["isError"], true);
    let words = failed["content"][0]["text"].as_str().unwrap();
    assert!(words.contains("invalid_token") && words.contains("401 Unauthorized"), "the pool forgets the sign-in on these: {words}");

    let narrow = account(app, "gmail", "gmail", "narrow");
    let failed = call(&*gmail, app, &narrow, "label_message", json!({ "messageId": "m1", "labelIds": ["Label_1"] })).await;
    let words = failed["content"][0]["text"].as_str().unwrap();
    assert!(words.contains("insufficient_scope") && words.contains("scope=\"https://www.googleapis.com/auth/gmail.modify\""), "{words}");
}
