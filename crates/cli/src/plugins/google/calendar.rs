//! Google Calendar's tools over the Calendar API (v3). Events read as the API gives them, with
//! the meeting link as `conferenceUrl`; the MCP server's own arguments (`startTime`,
//! `attendeeEmails`, `notificationLevel`) become the API's fields.

use std::sync::Arc;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc, Weekday};
use serde_json::{json, Map, Value};

use super::{google_api, list, number, param, required, result, text, Api, Failure};
use crate::app::App;
use crate::plugins::builtin::Service;

pub struct Server;

/// The server of a Calendar account, in its manifest.
const SERVER: &str = "calendar";

#[async_trait::async_trait]
impl Service for Server {
    fn name(&self) -> &'static str {
        "google-calendar"
    }

    fn instructions(&self) -> Option<String> {
        Some("Google Calendar for one account. Times are ISO 8601; calendarId defaults to the account's primary calendar. Event ids come from list_events and search_events.".into())
    }

    fn tools(&self, _app: &Arc<App>, _plugin_id: &str) -> Vec<Value> {
        super::tools("calendar")
    }

    async fn call(&self, app: &Arc<App>, plugin_id: &str, tool: &str, args: Value, _context: &Value) -> Value {
        let api = Api::new(app, plugin_id, SERVER);
        result(match tool {
            "list_calendars" => list_calendars(&api, &args).await,
            "list_events" => list_events(&api, &args).await,
            "search_events" => search_events(&api, &args).await,
            "get_event" => get_event(&api, &args).await,
            "create_event" => create_event(&api, &args).await,
            "update_event" => update_event(&api, &args).await,
            "delete_event" => delete_event(&api, &args).await,
            "respond_to_event" => respond_to_event(&api, &args).await,
            "suggest_time" => suggest_time(&api, &args).await,
            _ => Err(format!("Google Calendar has no tool {tool}.")),
        })
    }
}

fn base() -> String {
    format!("{}/calendar/v3", google_api())
}

fn calendar(args: &Value) -> String {
    let id = text(args, "calendarId").unwrap_or_else(|| "primary".into());
    url_segment(&id)
}

fn events_url(args: &Value) -> String {
    format!("{}/calendars/{}/events", base(), calendar(args))
}

fn event_url(args: &Value, id: &str) -> String {
    format!("{}/{}", events_url(args), url_segment(id))
}

/// A path segment, percent-encoded: calendar ids are email addresses.
fn url_segment(value: &str) -> String {
    value
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// An event as the API gives it, with its meeting link where the MCP server puts it.
fn event(found: Value) -> Value {
    let mut event = found;
    let video = event["conferenceData"]["entryPoints"].as_array().into_iter().flatten().find(|p| p["entryPointType"] == "video").and_then(|p| p["uri"].as_str()).map(str::to_string);
    if let Some(link) = video.or_else(|| event["hangoutLink"].as_str().map(str::to_string)) {
        event["conferenceUrl"] = json!(link);
    }
    event
}

async fn list_calendars(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let mut query = Vec::new();
    param(&mut query, "maxResults", number(args, "pageSize"));
    param(&mut query, "pageToken", text(args, "pageToken"));
    let found = api.get(&format!("{}/users/me/calendarList", base()), &query).await?;
    let calendars: Vec<Value> = found["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| json!({ "id": c["id"], "summary": c["summaryOverride"].as_str().or(c["summary"].as_str()), "description": c["description"], "timeZone": c["timeZone"], "primary": c["primary"].as_bool().unwrap_or(false), "accessRole": c["accessRole"] }))
        .collect();
    Ok(json!({ "calendars": calendars, "nextPageToken": found["nextPageToken"] }))
}

fn event_type(value: &str) -> Option<&'static str> {
    Some(match value {
        "DEFAULT" => "default",
        "OUT_OF_OFFICE" => "outOfOffice",
        "FOCUS_TIME" => "focusTime",
        "WORKING_LOCATION" => "workingLocation",
        "BIRTHDAY" => "birthday",
        "FROM_GMAIL" => "fromGmail",
        _ => return None,
    })
}

async fn list_events(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let mut query = vec![("singleEvents", "true".to_string())];
    param(&mut query, "timeMin", text(args, "startTime").map(|t| rfc3339(&t, args)));
    param(&mut query, "timeMax", text(args, "endTime").map(|t| rfc3339(&t, args)));
    param(&mut query, "timeZone", text(args, "timeZone"));
    param(&mut query, "q", text(args, "fullText"));
    param(&mut query, "maxResults", number(args, "pageSize"));
    param(&mut query, "pageToken", text(args, "pageToken"));
    let descending = args["orderBy"].as_str() == Some("startTimeDesc");
    match args["orderBy"].as_str() {
        Some("lastModified") => query.push(("orderBy", "updated".into())),
        Some("startTime" | "startTimeDesc") | None => query.push(("orderBy", "startTime".into())),
        _ => {}
    }
    for kind in list(args, "eventType").iter().chain(list(args, "eventTypeFilter").iter()).filter_map(|k| event_type(k)) {
        query.push(("eventTypes", kind.into()));
    }
    let found = api.get(&events_url(args), &query).await?;
    let mut events: Vec<Value> = found["items"].as_array().cloned().unwrap_or_default().into_iter().map(event).collect();
    if descending {
        events.reverse();
    }
    Ok(json!({ "summary": found["summary"], "description": found["description"], "updated": found["updated"], "timeZone": found["timeZone"], "accessRole": found["accessRole"], "defaultReminders": found["defaultReminders"], "events": events, "nextPageToken": found["nextPageToken"] }))
}

async fn search_events(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let mut query = vec![("singleEvents", "true".to_string()), ("q", required(args, "query")?)];
    param(&mut query, "maxResults", number(args, "pageSize"));
    param(&mut query, "pageToken", text(args, "pageToken"));
    let found = api.get(&events_url(&json!({})), &query).await?;
    let events: Vec<Value> = found["items"].as_array().cloned().unwrap_or_default().into_iter().map(event).collect();
    Ok(json!({ "events": events, "nextPageToken": found["nextPageToken"] }))
}

async fn get_event(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let id = required(args, "eventId")?;
    Ok(event(api.get(&event_url(args, &id), &[]).await?))
}

/// The query a change to an event goes with: who hears of it, and its meeting and attachments.
fn change_query(args: &Value) -> Vec<(&'static str, String)> {
    let mut query = vec![("conferenceDataVersion", "1".to_string()), ("supportsAttachments", "true".to_string())];
    let send = match args["notificationLevel"].as_str() {
        Some("NONE") => Some("none"),
        Some("EXTERNAL_ONLY") => Some("externalOnly"),
        Some("ALL") => Some("all"),
        _ => None,
    };
    param(&mut query, "sendUpdates", send.map(str::to_string));
    query
}

/// A time as RFC 3339: one without an offset is in the call's time zone, else UTC.
fn rfc3339(value: &str, args: &Value) -> String {
    if DateTime::parse_from_rfc3339(value).is_ok() {
        return value.to_string();
    }
    match local(value) {
        Some(naive) => {
            let zone: chrono_tz::Tz = text(args, "timeZone").and_then(|z| z.parse().ok()).unwrap_or(chrono_tz::UTC);
            zone.from_local_datetime(&naive).earliest().map(|t| t.to_rfc3339()).unwrap_or_else(|| value.to_string())
        }
        None => value.to_string(),
    }
}

fn local(value: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M"))
        .ok()
        .or_else(|| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok().map(|d| d.and_time(NaiveTime::MIN)))
}

/// An event's start and end: dates for an all-day event, else times in the call's time zone.
fn when(fields: &mut Map<String, Value>, args: &Value, start: Option<String>, end: Option<String>) {
    let all_day = args["allDay"].as_bool() == Some(true);
    let date = |value: &str| value.get(..10).and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
    let zone = text(args, "timeZone");
    let stamp = |value: &str| {
        let mut out = json!({ "dateTime": rfc3339(value, args) });
        if let Some(zone) = &zone {
            out["timeZone"] = json!(zone);
        }
        out
    };
    if all_day {
        let first = start.as_deref().and_then(date);
        if let Some(first) = first {
            fields.insert("start".into(), json!({ "date": first.to_string() }));
        }
        // The end is the day after the event, as the API takes it; an end on the first day
        // makes it one day long.
        if let Some(first) = first {
            let after = end.as_deref().and_then(date).filter(|last| *last > first).unwrap_or(first + Duration::days(1));
            fields.insert("end".into(), json!({ "date": after.to_string() }));
        }
    } else {
        if let Some(start) = start {
            fields.insert("start".into(), stamp(&start));
        }
        if let Some(end) = end {
            fields.insert("end".into(), stamp(&end));
        }
    }
}

fn attendee(value: &Value) -> Value {
    let mut out = json!({ "email": value["email"] });
    for (from, to) in [("displayName", "displayName"), ("optionalAttendee", "optional"), ("resource", "resource"), ("responseStatus", "responseStatus"), ("comment", "comment"), ("additionalGuests", "additionalGuests")] {
        if !value[from].is_null() {
            out[to] = value[from].clone();
        }
    }
    out
}

/// The fields an event's arguments set, apart from attendees and attachments.
fn fields(args: &Value) -> Map<String, Value> {
    let mut fields = Map::new();
    for key in ["summary", "description", "location", "colorId"] {
        if let Some(value) = args[key].as_str() {
            fields.insert(key.into(), json!(value));
        }
    }
    if let Some(visibility) = text(args, "visibility") {
        fields.insert("visibility".into(), json!(visibility));
    }
    match args["availability"].as_str() {
        Some("AVAILABILITY_FREE") => drop(fields.insert("transparency".into(), json!("transparent"))),
        Some("AVAILABILITY_BUSY") => drop(fields.insert("transparency".into(), json!("opaque"))),
        _ => {}
    }
    if let Some(kind) = args["eventType"].as_str().and_then(event_type) {
        fields.insert("eventType".into(), json!(kind));
    }
    if let Some(place) = args["workingLocationProperties"].as_object() {
        let mut properties = json!({});
        match place.get("type").and_then(Value::as_str) {
            Some("HOME_OFFICE") => {
                properties["type"] = json!("homeOffice");
                properties["homeOffice"] = json!({});
            }
            Some("CUSTOM_LOCATION") => {
                properties["type"] = json!("customLocation");
                properties["customLocation"] = json!({ "label": place.get("customLocationLabel").cloned().unwrap_or(Value::Null) });
            }
            _ => {}
        }
        fields.insert("workingLocationProperties".into(), properties);
    }
    if let Some(permissions) = args["guestPermissions"].as_object() {
        for key in ["guestsCanInviteOthers", "guestsCanModify", "guestsCanSeeGuests"] {
            if let Some(value) = permissions.get(key).filter(|v| v.is_boolean()) {
                fields.insert(key.into(), value.clone());
            }
        }
    }
    if let Some(reminders) = args["overrideReminders"].as_array() {
        let overrides: Vec<Value> = reminders.iter().map(|r| json!({ "method": r["method"].as_str().unwrap_or("popup"), "minutes": r["minutes"] })).collect();
        fields.insert("reminders".into(), json!({ "useDefault": false, "overrides": overrides }));
    }
    if args["addGoogleMeetUrl"].as_bool() == Some(true) {
        fields.insert("conferenceData".into(), json!({ "createRequest": { "requestId": uuid::Uuid::new_v4().to_string(), "conferenceSolutionKey": { "type": "hangoutsMeet" } } }));
    } else if let Some(link) = text(args, "googleMeetUrl") {
        fields.insert("conferenceData".into(), json!({ "conferenceSolution": { "key": { "type": "hangoutsMeet" } }, "entryPoints": [{ "entryPointType": "video", "uri": link }] }));
    }
    fields
}

async fn create_event(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let mut body = fields(args);
    body.insert("summary".into(), json!(required(args, "summary")?));
    when(&mut body, args, Some(required(args, "startTime")?), Some(required(args, "endTime")?));
    let mut attendees: Vec<Value> = list(args, "attendeeEmails").into_iter().map(|email| json!({ "email": email })).collect();
    attendees.extend(args["attendees"].as_array().into_iter().flatten().map(attendee));
    if !attendees.is_empty() {
        body.insert("attendees".into(), json!(attendees));
    }
    if let Some(rules) = args["recurrenceData"].as_array() {
        body.insert("recurrence".into(), json!(rules));
    }
    if let Some(files) = args["attachments"].as_array() {
        body.insert("attachments".into(), json!(files.iter().map(|f| json!({ "fileUrl": f["fileUrl"], "title": f["title"] })).collect::<Vec<_>>()));
    }
    Ok(event(api.post(&events_url(args), &change_query(args), Value::Object(body)).await?))
}

async fn update_event(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let id = required(args, "eventId")?;
    let current = api.get(&event_url(args, &id), &[]).await?;
    let mut body = fields(args);
    let (start, end) = (text(args, "startTime"), text(args, "endTime"));
    if start.is_some() || end.is_some() {
        when(&mut body, args, start, end);
    }
    let removed: Vec<String> = list(args, "removedAttendeeEmails").into_iter().map(|e| e.to_lowercase()).collect();
    let added: Vec<Value> = list(args, "addedAttendeeEmails").into_iter().map(|email| json!({ "email": email })).chain(args["addedAttendees"].as_array().into_iter().flatten().map(attendee)).collect();
    if !removed.is_empty() || !added.is_empty() {
        let mut attendees: Vec<Value> = current["attendees"].as_array().cloned().unwrap_or_default().into_iter().filter(|a| !a["email"].as_str().is_some_and(|e| removed.contains(&e.to_lowercase()))).collect();
        for person in added {
            if !attendees.iter().any(|a| a["email"].as_str().map(str::to_lowercase) == person["email"].as_str().map(str::to_lowercase)) {
                attendees.push(person);
            }
        }
        body.insert("attendees".into(), json!(attendees));
    }
    let gone = list(args, "removedAttachmentFileUrls");
    let new: Vec<Value> = args["addedAttachments"].as_array().into_iter().flatten().map(|f| json!({ "fileUrl": f["fileUrl"], "title": f["title"] })).collect();
    if !gone.is_empty() || !new.is_empty() {
        let mut files: Vec<Value> = current["attachments"].as_array().cloned().unwrap_or_default().into_iter().filter(|f| !f["fileUrl"].as_str().is_some_and(|u| gone.iter().any(|g| g == u))).collect();
        files.extend(new);
        body.insert("attachments".into(), json!(files));
    }
    Ok(event(api.patch(&event_url(args, &id), &change_query(args), Value::Object(body)).await?))
}

async fn delete_event(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let id = required(args, "eventId")?;
    let current = api.get(&event_url(args, &id), &[]).await?;
    let mut query = change_query(args);
    query.retain(|(key, _)| *key == "sendUpdates");
    api.delete(&event_url(args, &id), &query).await?;
    let mut gone = event(current);
    gone["status"] = json!("cancelled");
    Ok(gone)
}

async fn respond_to_event(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let id = required(args, "eventId")?;
    let status = required(args, "responseStatus")?;
    if !matches!(status.as_str(), "accepted" | "declined" | "tentative") {
        return Err("responseStatus is accepted, declined, or tentative.".into());
    }
    let current = api.get(&event_url(args, &id), &[]).await?;
    let mut attendees = current["attendees"].as_array().cloned().unwrap_or_default();
    let me = attendees.iter_mut().find(|a| a["self"].as_bool() == Some(true)).ok_or("The account is not invited to this event, so it has nothing to answer.")?;
    me["responseStatus"] = json!(status);
    if let Some(comment) = text(args, "responseComment") {
        me["comment"] = json!(comment);
    }
    let mut query = change_query(args);
    query.retain(|(key, _)| *key == "sendUpdates");
    Ok(event(api.patch(&event_url(args, &id), &query, json!({ "attendees": attendees })).await?))
}

async fn suggest_time(api: &Api<'_>, args: &Value) -> Result<Value, Failure> {
    let people = list(args, "attendeeEmails");
    if people.is_empty() {
        return Err("attendeeEmails is required.".into());
    }
    let zone: chrono_tz::Tz = text(args, "timeZone").and_then(|z| z.parse().ok()).unwrap_or(chrono_tz::UTC);
    let parse = |key: &str| -> Result<DateTime<Utc>, Failure> {
        let value = required(args, key)?;
        DateTime::parse_from_rfc3339(&rfc3339(&value, args)).map(|t| t.with_timezone(&Utc)).map_err(|_| format!("{key} is not an ISO 8601 time."))
    };
    let (from, to) = (parse("startTime")?, parse("endTime")?);
    if to <= from {
        return Err("endTime is after startTime.".into());
    }
    let mut items: Vec<Value> = people.iter().map(|email| json!({ "id": email })).collect();
    items.push(json!({ "id": "primary" }));
    let found = api.post(&format!("{}/freeBusy", base()), &[], json!({ "timeMin": from.to_rfc3339(), "timeMax": to.to_rfc3339(), "timeZone": zone.name(), "items": items })).await?;
    let mut busy: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
    for calendar in found["calendars"].as_object().into_iter().flat_map(|c| c.values()) {
        for span in calendar["busy"].as_array().into_iter().flatten() {
            let at = |key: &str| span[key].as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&Utc));
            if let (Some(start), Some(end)) = (at("start"), at("end")) {
                busy.push((start, end));
            }
        }
    }
    let preferences = &args["preferences"];
    let hour = |key: &str, default: (u32, u32)| preferences[key].as_str().and_then(|h| NaiveTime::parse_from_str(h, "%H:%M").ok()).unwrap_or_else(|| NaiveTime::from_hms_opt(default.0, default.1, 0).unwrap_or(NaiveTime::MIN));
    let (day_start, day_end) = (hour("startHour", (9, 0)), hour("endHour", (17, 0)));
    let weekends = preferences["excludeWeekends"].as_bool() != Some(false);
    let most = preferences["pageSize"].as_u64().unwrap_or(5).clamp(1, 50) as usize;
    let minutes = args["durationMinutes"].as_i64().filter(|m| *m > 0).unwrap_or(30);
    Ok(json!({ "timeSlots": free_slots(from, to, &busy, zone, day_start, day_end, weekends, Duration::minutes(minutes), most) }))
}

/// Slots of `length` in `[from, to)` that overlap no busy span, inside the working hours of each
/// local day, starting on the half hour, at most `most` of them.
#[allow(clippy::too_many_arguments)]
pub(super) fn free_slots(from: DateTime<Utc>, to: DateTime<Utc>, busy: &[(DateTime<Utc>, DateTime<Utc>)], zone: chrono_tz::Tz, day_start: NaiveTime, day_end: NaiveTime, no_weekends: bool, length: Duration, most: usize) -> Vec<Value> {
    let step = Duration::minutes(30);
    // The first half hour at or after `from`.
    let mut at = from.with_second(0).and_then(|t| t.with_nanosecond(0)).unwrap_or(from);
    let past = at.minute() % 30;
    if past != 0 || at < from {
        at += Duration::minutes(i64::from(30 - past));
    }
    let mut slots = Vec::new();
    while at + length <= to && slots.len() < most {
        let start = at.with_timezone(&zone);
        let end = (at + length).with_timezone(&zone);
        let weekday = start.weekday();
        let in_hours = start.time() >= day_start && end.time() <= day_end && end.date_naive() == start.date_naive();
        let weekend = no_weekends && matches!(weekday, Weekday::Sat | Weekday::Sun);
        let free = !busy.iter().any(|(b_start, b_end)| *b_start < at + length && *b_end > at);
        if in_hours && !weekend && free {
            slots.push(json!({
                "startTime": start.to_rfc3339(),
                "endTime": end.to_rfc3339(),
                "durationMinutes": length.num_minutes(),
                "start": { "dateTime": start.to_rfc3339(), "timeZone": zone.name() },
                "end": { "dateTime": end.to_rfc3339(), "timeZone": zone.name() },
            }));
            at += length;
        } else {
            at += step;
        }
    }
    slots
}
