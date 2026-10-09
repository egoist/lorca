//! Reviewed user skills, with explicit bot/group scope and encrypted revision history.
//! The account roster transports the library; bodies are read only on invocation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::app::App;
use crate::model::{Author, Body, Message};

pub const PROMPT_BYTES: usize = 6_000;
const MAX_CONTENT_BYTES: usize = 64 * 1024;
const MAX_LIBRARY_BYTES: usize = 2 * 1024 * 1024;
const MAX_RECORDS: usize = 128;
const MAX_REVISIONS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub kind: String,
    pub id: String,
}

impl Scope {
    pub fn bot(id: &str) -> Self {
        Self {
            kind: "bot".into(),
            id: id.into(),
        }
    }
    pub fn project(id: &str) -> Self {
        Self {
            kind: "project".into(),
            id: id.into(),
        }
    }

    pub fn validate(&self, app: &App) -> Result<(), String> {
        match self.kind.as_str() {
            "bot" if app.bot(&self.id).is_some() => Ok(()),
            "project" if app.chat(&self.id).is_some_and(|chat| chat.meta.is_group()) => Ok(()),
            _ => Err("Playbook scope must name an existing bot or group project".into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaybookContent {
    pub name: String,
    pub description: String,
    pub instructions: String,
    #[serde(default)]
    pub examples: String,
    #[serde(default)]
    pub references: Vec<Resource>,
    #[serde(default)]
    pub scripts: Vec<Resource>,
}

impl PlaybookContent {
    fn prepare(mut self) -> Result<Self, String> {
        self.name = self.name.trim().to_string();
        if self.name.is_empty()
            || self.name.len() > 64
            || !self
                .name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || self.name.starts_with('-')
            || self.name.ends_with('-')
            || self.name.contains("--")
        {
            return Err(
                "Skill name must be a lowercase slug of 1–64 letters, numbers, and single hyphens"
                    .into(),
            );
        }
        self.description = crate::memory::scrub(self.description.trim());
        if self.description.is_empty() || self.description.len() > 512 {
            return Err("Description must contain 1–512 bytes".into());
        }
        self.instructions = crate::memory::scrub(self.instructions.trim());
        if self.instructions.is_empty() {
            return Err("Instructions are required".into());
        }
        self.examples = crate::memory::scrub(&self.examples);
        let mut paths = BTreeSet::new();
        for (kind, resources) in [
            ("references", &mut self.references),
            ("scripts", &mut self.scripts),
        ] {
            if resources.len() > 16 {
                return Err("At most 16 references and 16 scripts per skill".into());
            }
            for resource in resources {
                if !safe_resource_path(&resource.path, kind) || !paths.insert(resource.path.clone())
                {
                    return Err(format!(
                        "Invalid or duplicate {kind} path: {}",
                        resource.path
                    ));
                }
                resource.text = crate::memory::scrub(&resource.text);
            }
        }
        if serde_json::to_vec(&self).map_err(|e| e.to_string())?.len() > MAX_CONTENT_BYTES {
            return Err("A skill's instructions and resources must fit in 64 KiB".into());
        }
        Ok(self)
    }

    pub fn hash(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("serializable content");
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub fn markdown(&self) -> String {
        let mut text = format!(
            "---\nname: {}\ndescription: {}\n---\n\n{}\n",
            self.name,
            serde_json::to_string(&self.description).unwrap(),
            self.instructions
        );
        if !self.examples.is_empty() {
            text.push_str(&format!("\n## Examples\n\n{}\n", self.examples));
        }
        for (title, resources) in [("References", &self.references), ("Scripts", &self.scripts)] {
            if !resources.is_empty() {
                text.push_str(&format!("\n## {title}\n\n"));
                for resource in resources {
                    text.push_str(&format!("- [{}]({})\n", resource.path, resource.path));
                }
            }
        }
        text
    }
}

fn safe_resource_path(path: &str, kind: &str) -> bool {
    path.len() <= 160
        && path.starts_with(&format!("{kind}/"))
        && path.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
}

/// Pattern scrubbing also removes exact account credential values with an opaque format.
/// Only secret fields are inspected; provider metadata never enters a playbook or export.
pub fn scrub(app: &App, text: &str) -> String {
    fn collect(value: &Value, values: &mut Vec<String>) {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    if matches!(
                        key.as_str(),
                        "api_key"
                            | "access_token"
                            | "refresh_token"
                            | "id_token"
                            | "client_secret"
                            | "token"
                    ) {
                        if let Some(secret) = value.as_str().filter(|s| !s.is_empty()) {
                            values.push(secret.into());
                        }
                    } else {
                        collect(value, values);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    collect(item, values);
                }
            }
            _ => {}
        }
    }
    let mut secrets = Vec::new();
    collect(
        &serde_json::to_value(&*app.credentials.lock().unwrap()).unwrap_or(Value::Null),
        &mut secrets,
    );
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    secrets.dedup();
    let mut text = crate::memory::scrub(text);
    for secret in secrets {
        text = text.replace(&secret, "«redacted credential»");
    }
    text
}

fn scrub_content(app: &App, mut content: PlaybookContent) -> Result<PlaybookContent, String> {
    if scrub(app, &content.name) != content.name
        || content
            .references
            .iter()
            .chain(&content.scripts)
            .any(|r| scrub(app, &r.path) != r.path)
    {
        return Err("Skill names and resource paths must not contain credentials".into());
    }
    content.description = scrub(app, &content.description);
    content.instructions = scrub(app, &content.instructions);
    content.examples = scrub(app, &content.examples);
    for resource in content.references.iter_mut().chain(&mut content.scripts) {
        resource.text = scrub(app, &resource.text);
    }
    content.prepare()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub chat_id: Option<String>,
    #[serde(default)]
    pub message_ids: Vec<String>,
    #[serde(default)]
    pub note: String,
}

impl Provenance {
    fn prepare(mut self, app: &App, scope: &Scope) -> Result<Self, String> {
        if self.kind.is_empty() {
            self.kind = "manual".into();
        }
        if self.kind.len() > 64 || self.note.len() > 2000 || self.message_ids.len() > 20 {
            return Err("Provenance exceeds its size limit".into());
        }
        self.note = scrub(app, &self.note);
        if let Some(chat_id) = &self.chat_id {
            let chat = app.chat(chat_id).ok_or("Source chat does not exist")?;
            if (scope.kind == "project" && scope.id != *chat_id)
                || (scope.kind == "bot" && !chat.meta.bot_ids.contains(&scope.id))
            {
                return Err("Source chat is outside the playbook's scope".into());
            }
            for id in &self.message_ids {
                if app.message(chat_id, id).is_none() {
                    return Err(format!("Source message {id} is unavailable"));
                }
            }
        } else if !self.message_ids.is_empty() {
            return Err("Cited messages require a source chat".into());
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Draft,
    Saved,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Revision {
    pub id: String,
    pub revision: u64,
    pub parent: Option<String>,
    pub status: Status,
    pub content: Option<PlaybookContent>,
    pub hash: String,
    pub provenance: Provenance,
    pub device_id: String,
    pub created_at: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Playbook {
    pub id: String,
    pub scope: Scope,
    pub revisions: Vec<Revision>,
}

impl Playbook {
    pub fn current(&self) -> &Revision {
        // Same-parent edits made offline both survive. One deterministic head is used until
        // the user resolves the branches with another guarded revision.
        self.revisions
            .iter()
            .max_by(|a, b| {
                a.revision
                    .cmp(&b.revision)
                    .then(a.created_at.total_cmp(&b.created_at))
                    .then(a.id.cmp(&b.id))
            })
            .expect("nonempty revisions")
    }
    fn summary(&self) -> Value {
        let head = self.current();
        json!({"id": self.id, "scope": self.scope, "revision": head.revision, "hash": head.hash,
            "status": head.status, "updated_at": head.created_at, "name": head.content.as_ref().map(|c| &c.name),
            "description": head.content.as_ref().map(|c| &c.description), "path": format!("playbook://{}/SKILL.md", self.id)})
    }
    fn view(&self) -> Value {
        let mut summary = self.summary();
        summary["content"] = json!(self.current().content);
        summary["provenance"] = json!(self.current().provenance);
        summary["revisions"] = json!(self.revisions);
        summary
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Library {
    #[serde(default)]
    pub records: BTreeMap<String, Playbook>,
}

impl Library {
    pub fn load(home: &Path, dek: Option<[u8; 32]>) -> anyhow::Result<Self> {
        let path = home.join("playbooks.enc");
        if !path.exists() {
            return Ok(Self::default());
        }
        let Some(dek) = dek else {
            return Ok(Self::default());
        };
        let library: Self = crate::crypto::decrypt_json(&dek, "playbooks", &std::fs::read(path)?)?;
        library.validate().map_err(anyhow::Error::msg)?;
        Ok(library)
    }

    fn persist(&self, app: &App) -> Result<(), String> {
        self.validate()?;
        let dek = app.dek().ok_or("Create or pair an identity first")?;
        let ciphertext =
            crate::crypto::encrypt_json(&dek, "playbooks", self).map_err(|e| e.to_string())?;
        crate::config::write_private(&app.config.home.join("playbooks.enc"), &ciphertext)
            .map_err(|e| e.to_string())
    }

    fn validate(&self) -> Result<(), String> {
        if self.records.len() > MAX_RECORDS
            || serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > MAX_LIBRARY_BYTES
        {
            return Err("Playbook library is full (128 skills / 2 MiB including history)".into());
        }
        for (id, record) in &self.records {
            if id != &record.id
                || !id.starts_with("playbook-")
                || id.len() > 64
                || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                || !matches!(record.scope.kind.as_str(), "bot" | "project")
                || record.scope.id.len() > 128
                || record.revisions.is_empty()
                || record.revisions.len() > MAX_REVISIONS
            {
                return Err("Invalid playbook record or revision limit reached".into());
            }
            let mut revision_ids = BTreeSet::new();
            for revision in &record.revisions {
                if revision.revision == 0
                    || !revision.created_at.is_finite()
                    || !revision_ids.insert(&revision.id)
                {
                    return Err("Invalid revision".into());
                }
                // Content was scrubbed and checked when it was written. Only its hash is checked
                // here, so a scrubber that learns a new pattern never rejects a stored library.
                if let Some(content) = &revision.content {
                    if revision.status == Status::Deleted || content.hash() != revision.hash {
                        return Err("Invalid revision content or hash".into());
                    }
                } else if revision.status != Status::Deleted || !revision.hash.is_empty() {
                    return Err("Missing revision content".into());
                }
            }
        }
        Ok(())
    }

    fn merge(&mut self, incoming: &Self) -> Result<bool, String> {
        incoming.validate()?;
        for (id, other) in &incoming.records {
            if let Some(local) = self.records.get_mut(id) {
                if local.scope != other.scope {
                    return Err("A synced playbook changed scope".into());
                }
                for revision in &other.revisions {
                    if let Some(known) =
                        local.revisions.iter().find(|known| known.id == revision.id)
                    {
                        if known != revision {
                            return Err("A synced revision changed contents".into());
                        }
                    } else {
                        local.revisions.push(revision.clone());
                    }
                }
                local.revisions.sort_by(|a, b| {
                    a.revision.cmp(&b.revision).then(a.created_at.total_cmp(&b.created_at)).then(a.id.cmp(&b.id))
                });
            } else {
                self.records.insert(id.clone(), other.clone());
            }
        }
        self.validate()?;
        // Whether this side has revisions of the incoming records that they lack.
        Ok(incoming.records.iter().any(|(id, other)| {
            self.records[id].revisions.iter().any(|revision| !other.revisions.contains(revision))
        }))
    }
}

/// Each skill syncs as a blob of its own, its whole record in one slot, so a change sends only
/// that skill and the roster carries none.
pub const BLOB_KIND: &str = "playbook";

/// Queues the record for the relay. It belongs to the group of its scope's chat, a group's own
/// or the bot's DM, so deleting that chat deletes it there too.
fn publish(app: &App, record: &Playbook) {
    let Some(dek) = app.dek() else { return };
    let group = {
        let state = app.state.lock().unwrap();
        state
            .chats
            .iter()
            .find(|chat| match record.scope.kind.as_str() {
                "bot" => !chat.meta.is_group() && chat.meta.bot_ids == [record.scope.id.clone()],
                _ => chat.meta.id == record.scope.id,
            })
            .map(|chat| crate::model::relay_name(&chat.meta.id))
    };
    match crate::crypto::encrypt_json(&dek, BLOB_KIND, record) {
        Ok(ciphertext) => app.push_slot_blob(BLOB_KIND, crate::app::Slot::latest(crate::model::relay_name(&record.id)), group, ciphertext),
        Err(error) => tracing::warn!(%error, "encrypting a playbook"),
    }
}

/// Merges a skill another Device sent, keeping every revision either side has. When this
/// Device has revisions the blob lacks, it sends the merged record back.
pub fn apply_remote(app: &App, dek: &[u8; 32], ciphertext: &[u8]) -> Result<(), String> {
    let record: Playbook = crate::crypto::decrypt_json(dek, BLOB_KIND, ciphertext).map_err(|e| e.to_string())?;
    let incoming = Library { records: BTreeMap::from([(record.id.clone(), record)]) };
    let merged = {
        let mut library = app.playbooks.lock().unwrap();
        let mut next = library.clone();
        let ahead = next.merge(&incoming)?;
        if next != *library {
            next.persist(app)?;
            *library = next;
        }
        let id = incoming.records.keys().next().unwrap();
        ahead.then(|| library.records[id].clone())
    };
    if let Some(record) = merged {
        publish(app, &record);
    }
    app.roster_changed(false);
    Ok(())
}

pub fn get(app: &App, scope: &Scope, id: &str) -> Result<Value, String> {
    scope.validate(app)?;
    let library = app.playbooks.lock().unwrap();
    let record = library
        .records
        .get(id)
        .filter(|r| &r.scope == scope)
        .ok_or("Playbook not found in this scope")?;
    Ok(record.view())
}

/// What the apps list in their snapshot: every skill and draft, without its body.
pub fn summaries(app: &App) -> Vec<Value> {
    let library = app.playbooks.lock().unwrap();
    library.records.values().filter(|r| r.current().status != Status::Deleted).map(Playbook::summary).collect()
}

/// Drops the skills of deleted bots and groups. Their blobs go with the deleted chats' groups on
/// the relay: a group's own chat, or the bot's DM.
pub fn forget_scopes(app: &App, bots: &[String], chats: &[String]) {
    if bots.is_empty() && chats.is_empty() {
        return;
    }
    let mut library = app.playbooks.lock().unwrap();
    let mut next = library.clone();
    next.records.retain(|_, r| !(if r.scope.kind == "bot" { bots.contains(&r.scope.id) } else { chats.contains(&r.scope.id) }));
    if next.records.len() == library.records.len() {
        return;
    }
    match next.persist(app) {
        Ok(()) => *library = next,
        Err(error) => tracing::warn!(%error, "dropping deleted bots' playbooks"),
    }
}

pub fn save(
    app: &App,
    scope: &Scope,
    id: Option<&str>,
    content: PlaybookContent,
    expected_revision: u64,
    expected_hash: &str,
    provenance: Provenance,
) -> Result<Value, String> {
    write_revision(
        app,
        scope,
        id,
        Some(content),
        Status::Saved,
        expected_revision,
        expected_hash,
        provenance,
    )
}

pub fn draft(
    app: &App,
    scope: &Scope,
    content: PlaybookContent,
    provenance: Provenance,
) -> Result<Value, String> {
    write_revision(
        app,
        scope,
        None,
        Some(content),
        Status::Draft,
        0,
        "",
        provenance,
    )
}

#[allow(clippy::too_many_arguments)]
fn write_revision(
    app: &App,
    scope: &Scope,
    id: Option<&str>,
    content: Option<PlaybookContent>,
    status: Status,
    expected_revision: u64,
    expected_hash: &str,
    provenance: Provenance,
) -> Result<Value, String> {
    scope.validate(app)?;
    let provenance = provenance.prepare(app, scope)?;
    let content = content
        .map(|content| scrub_content(app, content))
        .transpose()?;
    let device_id = app
        .this_device_id()
        .ok_or("Create or pair an identity first")?;
    let value;
    let written;
    {
        let mut library = app.playbooks.lock().unwrap();
        let mut next = library.clone();
        let creating = id.is_none();
        let id = id
            .map(str::to_string)
            .unwrap_or_else(|| format!("playbook-{}", uuid::Uuid::new_v4()));
        let previous = next.records.get(&id);
        if let Some(previous) = previous {
            if previous.scope != *scope {
                return Err("Playbook not found in this scope".into());
            }
            if previous.current().revision != expected_revision
                || previous.current().hash != expected_hash
            {
                return Err("Playbook changed since you opened it; reload before saving".into());
            }
            if previous.current().status == Status::Deleted {
                return Err("Playbook was removed; create a new skill".into());
            }
        } else if !creating
            || expected_revision != 0
            || !expected_hash.is_empty()
            || status == Status::Deleted
        {
            return Err("Playbook not found or changed since you opened it".into());
        }
        if let Some(content) = &content {
            if next.records.values().any(|r| {
                r.id != id
                    && r.scope == *scope
                    && r.current().status != Status::Deleted
                    && r.current()
                        .content
                        .as_ref()
                        .is_some_and(|c| c.name == content.name)
            }) {
                return Err("A skill with this name already exists in this scope".into());
            }
        }
        let parent = previous.map(|r| r.current().id.clone());
        let hash = content
            .as_ref()
            .map(PlaybookContent::hash)
            .unwrap_or_default();
        let record = next.records.entry(id.clone()).or_insert_with(|| Playbook {
            id,
            scope: scope.clone(),
            revisions: Vec::new(),
        });
        record.revisions.push(Revision {
            id: uuid::Uuid::new_v4().to_string(),
            revision: expected_revision
                .checked_add(1)
                .ok_or("Revision limit reached")?,
            parent,
            status,
            content,
            hash,
            provenance,
            device_id,
            created_at: crate::config::now_secs(),
        });
        let record_id = record.id.clone();
        next.persist(app)?;
        value = record_view(&next, &record_id);
        written = next.records[&record_id].clone();
        *library = next;
    }
    publish(app, &written);
    app.roster_changed(false);
    Ok(value)
}

fn record_view(library: &Library, id: &str) -> Value {
    library.records[id].view()
}

pub fn remove(
    app: &App,
    scope: &Scope,
    id: &str,
    expected_revision: u64,
    expected_hash: &str,
) -> Result<Value, String> {
    write_revision(
        app,
        scope,
        Some(id),
        None,
        Status::Deleted,
        expected_revision,
        expected_hash,
        Provenance {
            kind: "remove".into(),
            ..Default::default()
        },
    )
}

pub fn export(app: &App, scope: &Scope, id: &str) -> Result<Value, String> {
    let view = get(app, scope, id)?;
    if view["status"] != "saved" {
        return Err("Save and review the skill before exporting".into());
    }
    // Re-scrub exports as patterns improve. Export only allowlisted content, never provenance.
    let content: PlaybookContent =
        serde_json::from_value(view["content"].clone()).map_err(|e| e.to_string())?;
    let content = scrub_content(app, content)?;
    let mut files = vec![Resource {
        path: "SKILL.md".into(),
        text: content.markdown(),
    }];
    files.extend(content.references.clone());
    files.extend(content.scripts.clone());
    Ok(json!({"format": "lorca-playbook", "version": 1, "content": content, "files": files}))
}

/// A turn sees its bot's skills and only the project it is currently a member of.
pub fn scopes_for_turn(app: &App, bot_id: &str, chat_id: &str) -> Vec<Scope> {
    let mut scopes = vec![Scope::bot(bot_id)];
    if app
        .chat(chat_id)
        .is_some_and(|chat| chat.meta.is_group() && chat.meta.bot_ids.iter().any(|id| id == bot_id))
    {
        scopes.push(Scope::project(chat_id));
    }
    scopes
}

pub fn catalog(app: &App, scopes: &[Scope], query: &str, budget: usize) -> Value {
    let query = query.to_lowercase();
    let library = app.playbooks.lock().unwrap();
    let mut rows = Vec::new();
    let mut omitted = 0;
    for record in library
        .records
        .values()
        .filter(|r| scopes.contains(&r.scope) && r.current().status == Status::Saved)
    {
        let content = record.current().content.as_ref().unwrap();
        if !query.is_empty()
            && !format!("{} {}", content.name, content.description)
                .to_lowercase()
                .contains(&query)
        {
            continue;
        }
        rows.push(json!({"name": content.name, "description": content.description, "scope": record.scope.kind,
            "path": format!("playbook://{}/SKILL.md", record.id)}));
        if serde_json::to_vec(&json!({"items": &rows, "omitted": MAX_RECORDS}))
            .unwrap()
            .len()
            > budget
        {
            rows.pop();
            omitted += 1;
        }
    }
    json!({"items": rows, "omitted": omitted})
}

pub fn read_for_turn(app: &App, scopes: &[Scope], path: &str) -> Result<String, String> {
    let (id, resource) = path
        .strip_prefix("playbook://")
        .and_then(|p| p.split_once('/'))
        .ok_or("Use a playbook:// path from the catalog")?;
    let library = app.playbooks.lock().unwrap();
    let record = library
        .records
        .get(id)
        .filter(|r| scopes.contains(&r.scope) && r.current().status == Status::Saved)
        .ok_or("Saved playbook is not available in this turn's scope")?;
    let content = record.current().content.as_ref().unwrap();
    if resource == "SKILL.md" {
        return Ok(content.markdown());
    }
    content
        .references
        .iter()
        .chain(&content.scripts)
        .find(|r| r.path == resource)
        .map(|r| r.text.clone())
        .ok_or("Resource not found in this playbook".into())
}

/// Capture reads only explicitly selected completed text messages, never tool payloads,
/// attachments, memory, or another chat. Message ids are retained as private provenance.
pub fn selected_evidence(
    app: &App,
    scope: &Scope,
    bot_id: &str,
    chat_id: &str,
    kind: &str,
    ids: &[String],
) -> Result<Vec<Message>, String> {
    scope.validate(app)?;
    if !scopes_for_turn(app, bot_id, chat_id).contains(scope)
        || !app
            .chat(chat_id)
            .is_some_and(|c| c.meta.bot_ids.iter().any(|id| id == bot_id))
    {
        return Err("Capture bot and chat must belong to this scope".into());
    }
    if ids.is_empty() || ids.len() > 20 || ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err("Select 1–20 distinct source messages".into());
    }
    let mut messages = Vec::new();
    let mut bytes = 0;
    for id in ids {
        let mut message = app
            .message(chat_id, id)
            .ok_or("Selected source message is unavailable")?;
        if !message.is_complete() {
            return Err("Only completed messages can be captured".into());
        }
        if !matches!(&message.author, Author::You)
            && message.author
                != (Author::Bot {
                    bot_id: bot_id.into(),
                })
        {
            return Err("Capture includes messages from another bot".into());
        }
        let Body::Text { text, .. } = &message.body else {
            return Err("Select text messages, not tool or permission records".into());
        };
        if text.trim().is_empty() {
            return Err("Selected messages must contain text".into());
        }
        let text = scrub(app, text);
        bytes += text.len();
        if bytes > 24_000 {
            return Err("Selected workflow exceeds 24 KB; select a shorter example".into());
        }
        message.body = Body::text(text);
        messages.push(message);
    }
    messages.sort_by(|a, b| a.created_at.total_cmp(&b.created_at).then(a.id.cmp(&b.id)));
    match kind {
        "workflow"
            if messages
                .iter()
                .any(|m| matches!(&m.author, Author::Bot { .. })) => {}
        "corrections"
            if messages.len() >= 2 && messages.iter().all(|m| m.author == Author::You) => {}
        "workflow" => return Err("Select a completed bot reply as evidence of the workflow".into()),
        "corrections" => {
            return Err(
                "Select at least two user corrections to propose a standing instruction".into(),
            )
        }
        _ => return Err("Capture kind must be workflow or corrections".into()),
    }
    Ok(messages)
}

pub async fn dispatch(
    app: &std::sync::Arc<App>,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let scope: Scope = serde_json::from_value(params["scope"].clone())
        .map_err(|_| "Explicit playbook scope is required")?;
    let id = || {
        params["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Playbook id is required".to_string())
    };
    let guard = || -> Result<(u64, &str), String> {
        Ok((
            params["expected_revision"]
                .as_u64()
                .ok_or("expected_revision is required")?,
            params["expected_hash"]
                .as_str()
                .ok_or("expected_hash is required")?,
        ))
    };
    match method {
        "playbooks.get" => get(app, &scope, id()?),
        "playbooks.save" => {
            let content: PlaybookContent =
                serde_json::from_value(params["content"].clone()).map_err(|e| e.to_string())?;
            let provenance = params
                .get("provenance")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            let (revision, hash) = guard()?;
            save(
                app,
                &scope,
                params["id"].as_str(),
                content,
                revision,
                hash,
                provenance,
            )
        }
        "playbooks.remove" => {
            let (revision, hash) = guard()?;
            remove(app, &scope, id()?, revision, hash)
        }
        "playbooks.export" => export(app, &scope, id()?),
        "playbooks.draft" => {
            #[cfg(feature = "runner")]
            return crate::playbook_tools::capture(app, &scope, &params).await;
            // A phone asks the bot's Runner, which drafts with the bot's provider and syncs the
            // draft back as its blob.
            #[cfg(not(feature = "runner"))]
            {
                let bot = app.bot(params["bot_id"].as_str().unwrap_or_default()).ok_or("Bot not found")?;
                crate::requests::ask_within(app, &bot.runner_id, method, params.clone(), std::time::Duration::from_secs(150)).await
            }
        }
        _ => Err("Unknown playbook method".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::sync::Arc;

    struct Fixture {
        app: Arc<App>,
        home: std::path::PathBuf,
        bot: String,
        dm: String,
        project: String,
    }
    impl Fixture {
        fn new() -> Self {
            let home =
                std::env::temp_dir().join(format!("lorca-playbooks-{}", uuid::Uuid::new_v4()));
            let app = App::load(Config {
                home: home.clone(),
                port: 0,
            })
            .unwrap();
            crate::identity::create(&app, Some("Playbook test".into())).unwrap();
            let bot = app.state.lock().unwrap().bots[0].id.clone();
            let dm_chat = app
                .state
                .lock()
                .unwrap()
                .chats
                .iter()
                .find(|c| c.meta.bot_ids.contains(&bot))
                .unwrap()
                .clone();
            let dm = dm_chat.meta.id.clone();
            let mut group = dm_chat.meta;
            group.id = format!("chat-{}", uuid::Uuid::new_v4());
            group.kind = "group".into();
            group.title = Some("Project".into());
            let project = group.id.clone();
            app.create_chat(group).unwrap();
            Self {
                app,
                home,
                bot,
                dm,
                project,
            }
        }
        fn scope(&self) -> Scope {
            Scope::bot(&self.bot)
        }
        fn message(&self, author: Author, text: &str) -> Message {
            let message = Message::new(&self.dm, author, Body::text(text));
            self.app.upsert_message(message.clone(), false);
            message
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }
    fn content(name: &str) -> PlaybookContent {
        PlaybookContent {
            name: name.into(),
            description: "Use for a reviewed weekly report".into(),
            instructions: "BODY_ONLY: Compare the evidence and report the changes.".into(),
            examples: "A reusable example".into(),
            references: vec![Resource {
                path: "references/checklist.md".into(),
                text: "Check each source".into(),
            }],
            scripts: vec![Resource {
                path: "scripts/compare.sh".into(),
                text: "printf 'example'".into(),
            }],
        }
    }
    fn write(f: &Fixture, scope: &Scope, name: &str) -> Value {
        save(
            &f.app,
            scope,
            None,
            content(name),
            0,
            "",
            Provenance::default(),
        )
        .unwrap()
    }

    #[test]
    fn draft_review_edit_remove_are_guarded_and_retain_provenance() {
        let f = Fixture::new();
        let message = f.message(Author::You, "Make the report reproducible");
        let provenance = Provenance {
            kind: "workflow".into(),
            chat_id: Some(f.dm.clone()),
            message_ids: vec![message.id.clone()],
            note: "Selected successful workflow".into(),
        };
        let draft = draft(
            &f.app,
            &f.scope(),
            content("weekly-report"),
            provenance.clone(),
        )
        .unwrap();
        let id = draft["id"].as_str().unwrap();
        assert!(catalog(&f.app, &[f.scope()], "", PROMPT_BYTES)["items"].as_array().unwrap().is_empty());
        assert_eq!(summaries(&f.app).len(), 1);
        assert_eq!(summaries(&f.app)[0]["status"], "draft");
        assert!(read_for_turn(&f.app, &[f.scope()], draft["path"].as_str().unwrap()).is_err());
        assert!(export(&f.app, &f.scope(), id).is_err());
        let saved = save(
            &f.app,
            &f.scope(),
            Some(id),
            content("weekly-report"),
            1,
            draft["hash"].as_str().unwrap(),
            provenance,
        )
        .unwrap();
        assert_eq!(saved["status"], "saved");
        assert_eq!(saved["revision"], 2);
        assert_eq!(saved["provenance"]["message_ids"][0], message.id);
        assert!(save(
            &f.app,
            &f.scope(),
            Some(id),
            content("weekly-report"),
            1,
            draft["hash"].as_str().unwrap(),
            Provenance::default()
        )
        .unwrap_err()
        .contains("changed since"));
        assert!(save(
            &f.app,
            &f.scope(),
            Some(id),
            content("weekly-report"),
            2,
            "wrong",
            Provenance::default()
        )
        .is_err());
        assert!(save(
            &f.app,
            &Scope::project(&f.project),
            Some(id),
            content("weekly-report"),
            2,
            saved["hash"].as_str().unwrap(),
            Provenance::default()
        )
        .is_err());
        let mut updated = content("weekly-report");
        updated.instructions = "Use the corrected comparison procedure".into();
        let edit = save(
            &f.app,
            &f.scope(),
            Some(id),
            updated,
            2,
            saved["hash"].as_str().unwrap(),
            Provenance {
                kind: "correction".into(),
                note: "Corrected after review".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_ne!(edit["hash"], saved["hash"]);
        assert_eq!(edit["revisions"].as_array().unwrap().len(), 3);
        assert_eq!(
            edit["revisions"][0]["content"]["instructions"],
            content("weekly-report").instructions
        );
        assert!(remove(&f.app, &f.scope(), id, 2, saved["hash"].as_str().unwrap()).is_err());
        let removed = remove(&f.app, &f.scope(), id, 3, edit["hash"].as_str().unwrap()).unwrap();
        assert_eq!(removed["status"], "deleted");
        assert_eq!(removed["revisions"].as_array().unwrap().len(), 4);
        assert!(summaries(&f.app).is_empty());
        assert!(save(
            &f.app,
            &f.scope(),
            Some(id),
            content("weekly-report"),
            4,
            "",
            Provenance::default()
        )
        .is_err());
        assert_eq!(
            f.app.state.lock().unwrap().auto_review.rules.len(),
            0,
            "Skill lifecycle never adds execution permissions"
        );
    }

    #[test]
    fn persistence_and_roster_are_encrypted_and_restart_keeps_the_library() {
        let f = Fixture::new();
        let rosters = || f.app.store.outbox().unwrap().iter().filter(|item| item.kind == "roster").count();
        let before = rosters();
        let saved = write(&f, &f.scope(), "report");
        assert_eq!(rosters(), before, "a skill change uploads no roster");
        let bytes = std::fs::read(f.home.join("playbooks.enc")).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("BODY_ONLY"));
        let dek = f.app.dek().unwrap();
        assert!(crate::crypto::decrypt_json::<Library>(&[42; 32], "playbooks", &bytes).is_err());
        assert!(crate::crypto::decrypt_json::<Library>(&dek, "roster", &bytes).is_err());
        let restored = App::load(Config {
            home: f.home.clone(),
            port: 0,
        })
        .unwrap();
        assert_eq!(
            get(&restored, &f.scope(), saved["id"].as_str().unwrap()).unwrap()["hash"],
            saved["hash"]
        );
        let records = crate::crypto::decrypt_json::<Library>(&dek, "playbooks", &bytes).unwrap();
        assert_eq!(records.records.len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(f.home.join("playbooks.enc"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        // Each skill goes up as a blob of its own; the roster carries none.
        let outbox = f.app.store.outbox().unwrap();
        let blob = outbox.iter().find(|item| item.kind == BLOB_KIND).unwrap();
        assert!(!String::from_utf8_lossy(&blob.ciphertext).contains("BODY_ONLY"));
        assert!(crate::crypto::decrypt_json::<Playbook>(&dek, "roster", &blob.ciphertext).is_err());
        assert_eq!(crate::crypto::decrypt_json::<Playbook>(&dek, BLOB_KIND, &blob.ciphertext).unwrap(), records.records[saved["id"].as_str().unwrap()]);
        assert!(apply_remote(&restored, &dek, &blob.ciphertext).is_ok());
        f.app.forget_identity().unwrap();
        assert!(!f.home.join("playbooks.enc").exists());
        assert!(f.app.playbooks.lock().unwrap().records.is_empty());
    }

    #[test]
    fn sync_preserves_offline_branches_and_deletion_and_rejects_mutated_revisions() {
        let f = Fixture::new();
        let saved = write(&f, &f.scope(), "report");
        let id = saved["id"].as_str().unwrap();
        let baseline = f.app.playbooks.lock().unwrap().clone();
        let mut left = baseline.clone();
        let mut right = baseline.clone();
        for (library, revision_id, wording) in [
            (&mut left, "branch-a", "First correction"),
            (&mut right, "branch-b", "Second correction"),
        ] {
            let record = library.records.get_mut(id).unwrap();
            let mut revision = record.current().clone();
            revision.parent = Some(revision.id.clone());
            revision.id = revision_id.into();
            revision.revision = 2;
            revision.created_at = 100.0;
            revision.content.as_mut().unwrap().instructions = wording.into();
            revision.hash = revision.content.as_ref().unwrap().hash();
            record.revisions.push(revision);
        }
        assert!(left.merge(&right).unwrap());
        assert!(right.merge(&left).is_ok());
        assert_eq!(left, right);
        assert_eq!(left.records[id].revisions.iter().map(|r| r.revision).collect::<Vec<_>>(), vec![1, 2, 2]);
        assert_eq!(left.records[id].current().id, "branch-b");
        let mut deletion = left.records[id].current().clone();
        deletion.id = "removed".into();
        deletion.revision = 3;
        deletion.status = Status::Deleted;
        deletion.content = None;
        deletion.hash.clear();
        left.records.get_mut(id).unwrap().revisions.push(deletion);
        left.merge(&baseline).unwrap();
        assert_eq!(left.records[id].current().status, Status::Deleted);
        // A Device that sends an older copy of the skill gets the merged record back.
        let dek = f.app.dek().unwrap();
        let ours = f.app.playbooks.lock().unwrap().records[id].clone();
        let mut stale = ours.clone();
        stale.revisions.truncate(1);
        let mut newer = ours.clone();
        let mut edit = newer.current().clone();
        edit.id = "edited-elsewhere".into();
        edit.revision = 2;
        edit.content.as_mut().unwrap().instructions = "Edited on another Device".into();
        edit.hash = edit.content.as_ref().unwrap().hash();
        newer.revisions.push(edit);
        // The outbox keeps the newest blob of each slot: the revisions it would send.
        let queued = || {
            let outbox = f.app.store.outbox().unwrap();
            let blob = outbox.iter().rev().find(|item| item.kind == BLOB_KIND).unwrap();
            crate::crypto::decrypt_json::<Playbook>(&dek, BLOB_KIND, &blob.ciphertext).unwrap().revisions.len()
        };
        assert_eq!(queued(), 1);
        apply_remote(&f.app, &dek, &crate::crypto::encrypt_json(&dek, BLOB_KIND, &newer).unwrap()).unwrap();
        assert_eq!(get(&f.app, &f.scope(), id).unwrap()["revision"], 2);
        assert_eq!(queued(), 1, "nothing to send back when the blob has everything");
        apply_remote(&f.app, &dek, &crate::crypto::encrypt_json(&dek, BLOB_KIND, &stale).unwrap()).unwrap();
        assert_eq!(queued(), 2, "the merged record goes back to the relay");
        let mut corrupt = baseline.clone();
        corrupt.records.get_mut(id).unwrap().revisions[0]
            .provenance
            .note = "Forged revision".into();
        assert!(left
            .merge(&corrupt)
            .unwrap_err()
            .contains("changed contents"));
    }

    #[test]
    fn deleted_bots_and_groups_take_their_skills_along() {
        let f = Fixture::new();
        write(&f, &f.scope(), "bot-skill");
        write(&f, &Scope::project(&f.project), "group-skill");
        assert_eq!(summaries(&f.app).len(), 2);
        f.app.delete_chat(&f.project);
        assert_eq!(summaries(&f.app).len(), 1);
        f.app.delete_bot(&f.bot).unwrap();
        assert!(f.app.playbooks.lock().unwrap().records.is_empty());
        let stored: Library = crate::crypto::decrypt_json(&f.app.dek().unwrap(), "playbooks", &std::fs::read(f.home.join("playbooks.enc")).unwrap()).unwrap();
        assert!(stored.records.is_empty());
    }

    #[test]
    fn discovery_is_bounded_lazy_and_isolated_by_project_and_membership() {
        let f = Fixture::new();
        for i in 0..16 {
            write(&f, &f.scope(), &format!("report-{i}"));
        }
        let project = Scope::project(&f.project);
        let saved = write(&f, &project, "project-report");
        let scopes = scopes_for_turn(&f.app, &f.bot, &f.project);
        assert_eq!(scopes.len(), 2);
        let catalog = catalog(&f.app, &scopes, "", 1000);
        assert!(serde_json::to_vec(&catalog).unwrap().len() <= 1000);
        assert!(catalog["omitted"].as_u64().unwrap() > 0);
        assert!(!catalog.to_string().contains("BODY_ONLY"));
        let targeted = super::catalog(&f.app, &scopes, "project-report", PROMPT_BYTES);
        assert_eq!(targeted["items"].as_array().unwrap().len(), 1);
        let path = saved["path"].as_str().unwrap();
        assert!(read_for_turn(&f.app, &scopes, path)
            .unwrap()
            .contains("BODY_ONLY"));
        let resource = path.replace("SKILL.md", "references/checklist.md");
        assert_eq!(
            read_for_turn(&f.app, &scopes, &resource).unwrap(),
            "Check each source"
        );
        assert!(read_for_turn(&f.app, &scopes_for_turn(&f.app, &f.bot, &f.dm), path).is_err());
        f.app
            .state
            .lock()
            .unwrap()
            .chats
            .iter_mut()
            .find(|c| c.meta.id == f.project)
            .unwrap()
            .meta
            .bot_ids
            .clear();
        assert_eq!(scopes_for_turn(&f.app, &f.bot, &f.project), vec![f.scope()]);
        assert!(Scope::project(&f.dm).validate(&f.app).is_err());
    }

    #[test]
    fn export_has_only_selected_portable_content_and_safe_resources() {
        let f = Fixture::new();
        let source = f.message(
            Author::You,
            "Unrelated private client context must not export",
        );
        let mut value = content("report");
        value
            .instructions
            .push_str(" API_KEY=abcdef123456789 sk-abcdefghijklmnopqrstuvwxyz123456");
        value.references[0]
            .text
            .push_str(" password: sensitive-password");
        value.scripts[0]
            .text
            .push_str(" token=another-sensitive-token");
        let saved = save(
            &f.app,
            &f.scope(),
            None,
            value,
            0,
            "",
            Provenance {
                kind: "manual".into(),
                chat_id: Some(f.dm.clone()),
                message_ids: vec![source.id.clone()],
                note: "Private correction note".into(),
            },
        )
        .unwrap();
        let exported = export(&f.app, &f.scope(), saved["id"].as_str().unwrap()).unwrap();
        let text = exported.to_string();
        for secret in [
            "abcdef123456789",
            "sk-abcdefghijklmnopqrstuvwxyz123456",
            "sensitive-password",
            "another-sensitive-token",
            &source.id,
            &f.bot,
            &f.dm,
            "Private correction note",
            "Unrelated private client",
        ] {
            assert!(!text.contains(secret), "Export leaked {secret}");
        }
        assert_eq!(
            exported
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["content", "files", "format", "version"]
        );
        assert_eq!(exported["files"].as_array().unwrap().len(), 3);
        assert!(exported["files"][0]["text"]
            .as_str()
            .unwrap()
            .contains("name: report"));
        for bad in [
            "../outside",
            "/absolute",
            "references/../secret",
            "references/a\\b",
            "references/.env",
            "references//a",
        ] {
            let mut content = content("bad");
            content.references[0].path = bad.into();
            assert!(
                save(
                    &f.app,
                    &f.scope(),
                    None,
                    content,
                    0,
                    "",
                    Provenance::default()
                )
                .is_err(),
                "Accepted {bad}"
            );
        }
        let mut duplicate = content("duplicate");
        duplicate.references.push(duplicate.references[0].clone());
        assert!(duplicate.prepare().is_err());
        assert_eq!(
            crate::memory::scrub("API_KEY=«redacted 20 chars»"),
            "API_KEY=«redacted 20 chars»"
        );
    }

    #[test]
    fn capture_requires_selected_completed_scoped_evidence_and_repeated_corrections() {
        let f = Fixture::new();
        let one = f.message(Author::You, "Use the public example only");
        let two = f.message(Author::You, "Again, use the public example only");
        let reply = f.message(
            Author::Bot {
                bot_id: f.bot.clone(),
            },
            "Completed reproducible workflow",
        );
        assert!(selected_evidence(
            &f.app,
            &f.scope(),
            &f.bot,
            &f.dm,
            "corrections",
            &[one.id.clone()]
        )
        .is_err());
        assert!(selected_evidence(
            &f.app,
            &f.scope(),
            &f.bot,
            &f.dm,
            "corrections",
            &[one.id.clone(), two.id.clone()]
        )
        .is_ok());
        assert!(selected_evidence(
            &f.app,
            &f.scope(),
            &f.bot,
            &f.dm,
            "workflow",
            &[one.id.clone()]
        )
        .is_err());
        assert!(selected_evidence(
            &f.app,
            &f.scope(),
            &f.bot,
            &f.dm,
            "workflow",
            &[one.id.clone(), reply.id.clone()]
        )
        .is_ok());
        assert!(selected_evidence(
            &f.app,
            &Scope::project(&f.project),
            &f.bot,
            &f.dm,
            "workflow",
            &[reply.id.clone()]
        )
        .is_err());
        assert!(selected_evidence(
            &f.app,
            &f.scope(),
            &f.bot,
            &f.dm,
            "workflow",
            &[reply.id.clone(), reply.id.clone()]
        )
        .is_err());
        let other = f.message(
            Author::Bot {
                bot_id: "other-bot".into(),
            },
            "Other private work",
        );
        assert!(selected_evidence(
            &f.app,
            &f.scope(),
            &f.bot,
            &f.dm,
            "workflow",
            &[reply.id.clone(), other.id]
        )
        .is_err());
        let mut streaming = reply.clone();
        streaming.state = crate::model::MessageState::Streaming;
        f.app.upsert_message(streaming, false);
        assert!(
            selected_evidence(&f.app, &f.scope(), &f.bot, &f.dm, "workflow", &[reply.id]).is_err()
        );
        let invalid = Provenance {
            chat_id: Some(f.project.clone()),
            message_ids: vec![one.id],
            ..Default::default()
        };
        assert!(draft(&f.app, &f.scope(), content("bad-evidence"), invalid).is_err());
    }

    #[test]
    fn opaque_credential_markers_are_stable_through_export_and_new_scope_save() {
        let f = Fixture::new();
        let credential = "opaque-provider-credential-for-export";
        f.app.credentials.lock().unwrap().deepseek = Some(crate::credentials::ApiKeyCredential {
            api_key: credential.into(),
            base_url: None,
            connected_at: 1,
        });
        let mut authored = content("portable-report");
        authored.instructions = format!(
            "Use the provided value: {credential}\nAPI_KEY={credential}\nAPI_KEY=«redacted credential»\nPASSWORD='«redacted 23 chars»'"
        );
        authored.examples = "TOKEN=«redacted credential»\nAPI_KEY=\"«redacted 42 chars»\"".into();
        authored.references[0].text = "password: «redacted credential»".into();
        authored.scripts[0].text =
            "export API_KEY='«redacted credential»'\nTOKEN=«redacted 23 chars»".into();

        let source = save(
            &f.app,
            &f.scope(),
            None,
            authored,
            0,
            "",
            Provenance::default(),
        )
        .unwrap();
        let exported = export(&f.app, &f.scope(), source["id"].as_str().unwrap()).unwrap();
        let clean: PlaybookContent = serde_json::from_value(exported["content"].clone()).unwrap();
        assert!(!exported.to_string().contains(credential));
        assert!(clean
            .instructions
            .contains("Use the provided value: «redacted credential»"));
        assert!(clean.instructions.contains("API_KEY=«redacted credential»"));
        assert!(clean
            .instructions
            .contains("PASSWORD='«redacted 23 chars»'"));
        assert_eq!(
            clean.examples,
            "TOKEN=«redacted credential»\nAPI_KEY=\"«redacted 42 chars»\""
        );
        assert_eq!(clean.references[0].text, "password: «redacted credential»");
        assert_eq!(
            clean.scripts[0].text,
            "export API_KEY='«redacted credential»'\nTOKEN=«redacted 23 chars»"
        );

        let recipient = Scope::project(&f.project);
        let mut imported = save(
            &f.app,
            &recipient,
            None,
            clean.clone(),
            0,
            "",
            Provenance {
                kind: "template_import".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_ne!(imported["id"], source["id"]);
        assert_eq!(imported["content"], source["content"]);
        assert_eq!(imported["hash"], source["hash"]);
        for _ in 0..3 {
            let round_trip = export(&f.app, &recipient, imported["id"].as_str().unwrap()).unwrap();
            assert_eq!(
                round_trip, exported,
                "Repeated export must not grow redaction markers"
            );
            let content: PlaybookContent =
                serde_json::from_value(round_trip["content"].clone()).unwrap();
            imported = save(
                &f.app,
                &recipient,
                imported["id"].as_str(),
                content,
                imported["revision"].as_u64().unwrap(),
                imported["hash"].as_str().unwrap(),
                Provenance::default(),
            )
            .unwrap();
            assert_eq!(imported["content"], source["content"]);
            assert_eq!(
                imported["hash"], source["hash"],
                "Content hash must stay stable across scrubbing passes"
            );
        }
    }

    #[tokio::test]
    async fn api_requires_guards_and_never_blindly_overwrites() {
        let f = Fixture::new();
        let params = json!({"scope":f.scope(),"content":content("api-skill"),"expected_revision":0,"expected_hash":""});
        let saved = crate::api::dispatch(&f.app, "playbooks.save", params.clone())
            .await
            .unwrap();
        let mut unguarded = params;
        unguarded["id"] = saved["id"].clone();
        unguarded.as_object_mut().unwrap().remove("expected_hash");
        assert!(crate::api::dispatch(&f.app, "playbooks.save", unguarded)
            .await
            .unwrap_err()
            .contains("expected_hash"));
        assert!(crate::api::dispatch(&f.app, "playbooks.get", json!({"id":saved["id"]}))
            .await
            .is_err());
        assert_eq!(
            crate::api::dispatch(
                &f.app,
                "playbooks.get",
                json!({"scope":f.scope(),"id":saved["id"]})
            )
            .await
            .unwrap()["revision"],
            1
        );
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn drafting_model_sees_only_selected_evidence_and_review_activates_on_a_later_turn() {
        use crate::credentials::{CustomApi, CustomModel, CustomProvider};
        use axum::{routing::post, Json, Router};
        use std::sync::Mutex;

        let f = Fixture::new();
        let request = f.message(Author::You, "Compare the public report");
        let reply = f.message(
            Author::Bot {
                bot_id: f.bot.clone(),
            },
            "Finished the public comparison",
        );
        f.message(Author::You, "PRIVATE_UNSELECTED_CONTEXT");
        let opaque_key = "opaqueProviderCredential123";
        let source = f.message(
            Author::You,
            &format!("Another correction: use public examples. {opaque_key}"),
        );
        let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
        let recording = seen.clone();
        let generated = serde_json::to_string(&content("captured-report")).unwrap();
        let router = Router::new().route("/chat/completions", post(move |Json(body): Json<Value>| {
            let seen = recording.clone();
            let generated = generated.clone();
            async move {
                seen.lock().unwrap().push(body);
                let data = json!({"id":"draft-test","object":"chat.completion.chunk","model":"draft-model",
                    "choices":[{"index":0,"delta":{"role":"assistant","content":generated},"finish_reason":null}]});
                let end = json!({"id":"draft-test","object":"chat.completion.chunk","model":"draft-model",
                    "choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120}});
                ([("content-type","text/event-stream")],format!("data: {data}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        f.app.credentials.lock().unwrap().custom.insert(
            "custom:playbook-test".into(),
            CustomProvider {
                name: "Draft test".into(),
                api: CustomApi::ChatCompletions,
                base_url: root,
                api_key: opaque_key.into(),
                models: vec![CustomModel {
                    id: "draft-model".into(),
                    name: None,
                    context_window: Some(32000),
                    max_output: Some(4096),
                    images: None,
                }],
                created_at: 1,
            },
        );
        f.app
            .update_bot(&f.bot, |bot| {
                bot.provider = "custom:playbook-test".into();
            })
            .unwrap();
        let draft = crate::api::dispatch(
            &f.app,
            "playbooks.draft",
            json!({"scope":f.scope(),"bot_id":f.bot,"chat_id":f.dm,
            "kind":"workflow","message_ids":[request.id,reply.id,source.id]}),
        )
        .await
        .unwrap();
        server.abort();
        let body = seen.lock().unwrap()[0].clone();
        let prompt = body["messages"].to_string();
        assert!(prompt.contains("Finished the public comparison"));
        assert!(!prompt.contains("PRIVATE_UNSELECTED_CONTEXT"));
        assert!(!prompt.contains(opaque_key));
        assert!(body
            .get("tools")
            .is_none_or(|v| v.as_array().is_some_and(|a| a.is_empty())));
        assert_eq!(draft["status"], "draft");
        assert!(catalog(&f.app, &[f.scope()], "", PROMPT_BYTES)["items"]
            .as_array()
            .unwrap()
            .is_empty());
        let saved = save(
            &f.app,
            &f.scope(),
            draft["id"].as_str(),
            content("captured-report"),
            1,
            draft["hash"].as_str().unwrap(),
            Provenance {
                kind: "reviewed_edit".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            catalog(
                &f.app,
                &scopes_for_turn(&f.app, &f.bot, &f.dm),
                "captured-report",
                PROMPT_BYTES
            )["items"][0]["path"],
            saved["path"]
        );
        let scopes = scopes_for_turn(&f.app, &f.bot, &f.dm);
        assert!(
            read_for_turn(&f.app, &scopes, saved["path"].as_str().unwrap())
                .unwrap()
                .contains("BODY_ONLY")
        );
        assert_eq!(f.app.state.lock().unwrap().auto_review.rules.len(), 0);
    }
}
