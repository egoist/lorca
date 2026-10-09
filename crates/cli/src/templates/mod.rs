//! Private template files: explicit allowlisted selections, a reviewable preview, and an
//! independent recipient bot. Runtime data crosses Devices only through sealed requests and
//! the existing encrypted roster; creating an export is solely a local file operation.

mod files;
pub mod format;
pub mod links;
mod playbooks;
mod routines;
mod secrets;

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app::App;
use crate::model::{Bot, PluginStatus};
use format::{Profile, Requirement, Routine, Skill, Template};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    #[serde(default)]
    pub profile: bool,
    #[serde(default)]
    pub skill_ids: Vec<String>,
    #[serde(default)]
    pub memory_ids: Vec<String>,
    #[serde(default)]
    pub routine_ids: Vec<String>,
    #[serde(default)]
    pub requirement_ids: Vec<String>,
}

/// A piece of content the export sheet offers, as it would go in the file, with what its reader
/// should look at before sharing it (`format::flags`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item<T> {
    pub id: String,
    pub content: T,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
}

/// A plugin the bot's Runner has, offered as a requirement, with the name the apps show.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub service_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contents {
    pub profile: Item<Profile>,
    pub skills: Vec<Item<Skill>>,
    pub memories: Vec<Item<String>>,
    pub routines: Vec<Item<Routine>>,
    pub requirements: Vec<Service>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportPreview {
    pub template: Template,
    pub digest: String,
}

/// #71 supplies optional service_id on PluginStatus. Reading the wire field lets the adapter
/// understand named accounts after integration without requiring a second connection model.
fn service_id(status: &PluginStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v["service_id"].as_str().map(str::to_string))
        .unwrap_or_else(|| status.id.clone())
}

fn runner_plugins(app: &App, runner_id: &str) -> Result<Vec<PluginStatus>, String> {
    let runner = app.device(runner_id).ok_or("Unknown Runner")?;
    if !runner.is_runner() {
        return Err(format!("{} cannot run bots.", runner.name));
    }
    if app.this_device_id().as_deref() == Some(runner_id) {
        Ok(app.plugins.lock().unwrap().statuses())
    } else {
        Ok(runner.plugins)
    }
}

/// Everything the bot has that a template can carry, redacted as the file would be.
pub async fn contents(app: &Arc<App>, bot_id: &str) -> Result<Contents, String> {
    let mut bot = app.bot(bot_id).ok_or("Unknown bot")?;
    bot.normalize_description();
    let mut skills = vec![];
    for item in playbooks::list(app, bot_id) {
        let id = item["id"].as_str().ok_or("Invalid playbook id")?;
        skills.push(item_of(id, playbooks::export(app, bot_id, id)?));
    }
    let memories: Vec<Item<String>> = if app.this_device_id().as_deref() == Some(bot.runner_id.as_str()) {
        local_memories(app, bot_id)?
    } else {
        let value = crate::requests::ask(
            app,
            &bot.runner_id,
            "templates.memories",
            json!({ "bot_id": bot_id }),
        )
        .await?;
        serde_json::from_value(value)
            .map_err(|e| format!("Cannot read the Runner's template memories: {e}"))?
    };
    let routines = app
        .routines_of(bot_id)
        .into_iter()
        .map(|r| item_of(&r.id.clone(), routines::export(r)))
        .collect();
    let mut ids = HashSet::new();
    let requirements = runner_plugins(app, &bot.runner_id)?
        .into_iter()
        .filter_map(|status| {
            let id = service_id(&status);
            ids.insert(id.clone())
                .then_some(Service { service_id: id, name: status.name })
        })
        .collect();
    let profile = Profile {
        name: bot.name,
        description: bot.description,
        symbol_name: bot.symbol_name,
        accent: bot.accent,
    };
    let mut contents = Contents {
        profile: item_of("profile", profile),
        skills,
        memories,
        routines,
        requirements,
    };
    let known = secrets::local(app);
    let clean = |texts: Vec<&mut String>| {
        for text in texts {
            *text = format::scrub_text(&secrets::redact(text, &known));
        }
    };
    clean(contents.profile.content.texts_mut());
    contents.skills.iter_mut().for_each(|item| clean(item.content.texts_mut()));
    contents.memories.iter_mut().for_each(|item| clean(vec![&mut item.content]));
    contents.routines.iter_mut().for_each(|item| clean(item.content.texts_mut()));
    flag(&mut contents.profile, Profile::texts_mut);
    contents.skills.iter_mut().for_each(|item| flag(item, Skill::texts_mut));
    contents.memories.iter_mut().for_each(|item| flag(item, |text| vec![text]));
    contents.routines.iter_mut().for_each(|item| flag(item, Routine::texts_mut));
    Ok(contents)
}

fn item_of<T>(id: &str, content: T) -> Item<T> {
    Item { id: id.into(), content, flags: vec![] }
}

fn flag<T: Clone>(item: &mut Item<T>, texts: impl Fn(&mut T) -> Vec<&mut String>) {
    let mut copy = item.content.clone();
    let texts: Vec<&str> = texts(&mut copy).into_iter().map(|text| text.as_str()).collect();
    item.flags = format::flags(&texts);
}

/// Only curated MEMORY.md lines and direct Markdown topics, bounded and regular. This does
/// not adopt a workdir's MEMORY.md, traverse attachments, read logs, or follow symbolic links.
pub fn local_memories(app: &Arc<App>, bot_id: &str) -> Result<Vec<Item<String>>, String> {
    local_bot(app, bot_id)?;
    let dir = files::memory_dir(app, bot_id)?;
    let mut items = vec![];
    let index = dir.join("MEMORY.md");
    if index.exists() || std::fs::symlink_metadata(&index).is_ok() {
        let text = files::read(&index, crate::memory::MEMORY_FILE_MAX_BYTES)?;
        let mut seen = HashSet::new();
        for line in text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let id = format!("memory-{}", crate::memory::hash_text(line));
            if seen.insert(id.clone()) {
                items.push(item_of(&id, line.to_string()));
            }
        }
    }
    let topics = dir.join("memory");
    files::regular_directories(&dir, &topics)?;
    if topics.is_dir() {
        let mut entries = std::fs::read_dir(&topics)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.extension().and_then(|p| p.to_str()) != Some("md")
                || !entry.file_type().map_err(|e| e.to_string())?.is_file()
            {
                continue;
            }
            let text = files::read(&path, 64 * 1024)?;
            if !text.trim().is_empty() {
                let id = format!("topic-{}", crate::memory::hash_text(&format!("{name}\n{text}")));
                items.push(item_of(&id, text));
            }
        }
    }
    let known = secrets::local(app);
    for item in &mut items {
        item.content = secrets::redact(&item.content, &known);
    }
    // Credentials the Runner holds are redacted here; patterns and flags are the asker's.
    Ok(items)
}

pub async fn export_preview(
    app: &Arc<App>,
    bot_id: &str,
    selection: &Selection,
) -> Result<ExportPreview, String> {
    let bot = app.bot(bot_id).ok_or("Unknown bot")?;
    if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
        let value = crate::requests::ask(
            app,
            &bot.runner_id,
            "templates.export.preview",
            json!({ "bot_id": bot_id, "selection": selection }),
        )
        .await?;
        return serde_json::from_value(value)
            .map_err(|e| format!("Invalid Runner template preview: {e}"));
    }
    if !selection.profile
        && selection.skill_ids.is_empty()
        && selection.memory_ids.is_empty()
        && selection.routine_ids.is_empty()
        && selection.requirement_ids.is_empty()
    {
        return Err("Select reusable content before previewing a template.".into());
    }
    let catalog = contents(app, bot_id).await?;
    let bot = app.bot(bot_id).ok_or("Unknown bot")?;
    let skills = select(&catalog.skills, &selection.skill_ids, "skill")?;
    let memories = select(&catalog.memories, &selection.memory_ids, "memory")?;
    let routines = select(&catalog.routines, &selection.routine_ids, "routine")?;
    let mut selected = HashSet::new();
    let mut requirements = vec![];
    for id in &selection.requirement_ids {
        if !selected.insert(id) {
            return Err(format!("Requirement {id} is selected twice."));
        }
        let service = catalog
            .requirements
            .iter()
            .find(|r| &r.service_id == id)
            .ok_or_else(|| format!("{id} is no longer on this bot's Runner."))?;
        requirements.push(Requirement { service_id: service.service_id.clone() });
    }
    let mut template = Template {
        profile: selection.profile.then_some(catalog.profile.content),
        skills,
        memories,
        routines,
        requirements,
        ..Template::default()
    };
    let statuses = runner_plugins(app, &bot.runner_id)?;
    // Account IDs and labels are never included in the portable requirement or tool namespace.
    let mut mappings = BTreeMap::new();
    let mut instance_ids = BTreeMap::new();
    let mut seen_services = BTreeMap::new();
    let namespaces = template.namespaces();
    for status in &statuses {
        let source = format::namespace(&status.id);
        let service = service_id(status);
        if status.id == service
            && format::is_named_instance(&status.id)
            && (selected.iter().any(|id| **id == service)
                || namespaces.contains(&source)
                || template.contains_text(&status.id))
        {
            return Err("The source connection is missing service_id metadata. Update its Runner's CLI before exporting this content.".into());
        }
        let direct_instance = status.id != service && template.contains_text(&status.id);
        if namespaces.contains(&source) || direct_instance {
            if let Some(previous) = seen_services.insert(service.clone(), source.clone()) {
                if previous != source {
                    return Err(format!("Selected content uses several {service} accounts. A portable requirement maps one recipient account per service; split this content before exporting."));
                }
            }
            if !selected.iter().any(|id| **id == service) {
                return Err(format!("What you picked uses {}. Check it under Plugins too.", status.name));
            }
            mappings.insert(source, format::namespace(&service));
            if status.id != service {
                instance_ids.insert(status.id.clone(), format!("{{{{connection:{service}}}}}"));
            }
        }
    }
    template.map_namespaces(&mappings);
    template.map_connection_ids(&instance_ids);
    validate_namespaces(&template)?;
    template.scrub_known(&secrets::local(app));
    template.scrub();
    template.validate()?;
    let digest = template_digest(&template)?;
    Ok(ExportPreview { template, digest })
}

fn select<T: Clone>(items: &[Item<T>], ids: &[String], kind: &str) -> Result<Vec<T>, String> {
    if ids.len() > format::MAX_ITEMS {
        return Err(format!(
            "Select at most {} {kind} items.",
            format::MAX_ITEMS
        ));
    }
    let mut seen = HashSet::new();
    ids.iter()
        .map(|id| {
            if !seen.insert(id) {
                return Err(format!("The {kind} {id} is selected twice."));
            }
            items
                .iter()
                .find(|item| &item.id == id)
                .map(|item| item.content.clone())
                .ok_or_else(|| format!("The selected {kind} changed on its Runner. Open the export again."))
        })
        .collect()
}

fn validate_namespaces(template: &Template) -> Result<(), String> {
    let services: HashSet<_> = template
        .requirements
        .iter()
        .map(|r| r.service_id.clone())
        .collect();
    let mut missing: Vec<_> = template
        .connection_references()
        .difference(&services)
        .cloned()
        .collect();
    missing.sort();
    if !missing.is_empty() {
        return Err(format!(
            "Connection references need selected service requirements: {}.",
            missing.join(", ")
        ));
    }
    let required: HashSet<_> = template
        .requirements
        .iter()
        .map(|r| format::namespace(&r.service_id))
        .collect();
    let mut missing: Vec<_> = template
        .namespaces()
        .difference(&required)
        .cloned()
        .collect();
    missing.sort();
    if !missing.is_empty() {
        return Err(format!("This uses tools from {}, which isn't a plugin listed in the template.", missing.join(", ")));
    }
    Ok(())
}

fn template_digest(template: &Template) -> Result<String, String> {
    Ok(crate::memory::hash_text(
        &serde_json::to_string(template).map_err(|e| e.to_string())?,
    ))
}

pub async fn dispatch(app: &Arc<App>, method: &str, params: &Value) -> Result<Value, String> {
    match method {
        "templates.contents" => Ok(json!(contents(app, required(params, "bot_id")?).await?)),
        "templates.export.preview" | "templates.export" => {
            let selection: Selection = serde_json::from_value(params["selection"].clone())
                .map_err(|e| format!("Invalid export selection: {e}"))?;
            let preview = export_preview(app, required(params, "bot_id")?, &selection).await?;
            if method.ends_with(".preview") {
                return Ok(json!(preview));
            }
            review(params, &preview.digest)?;
            let path = Path::new(required(params, "path")?);
            files::export(
                app,
                path,
                &serde_json::to_string_pretty(&preview.template).map_err(|e| e.to_string())?,
                params["overwrite"].as_bool() == Some(true),
            )?;
            Ok(json!({ "path": path }))
        }
        "templates.share" => links::share(app, params).await,
        "templates.unshare" => links::revoke(app, required(params, "link_id")?).await,
        "templates.import.preview" | "templates.import" => {
            // A file this CLI reads, or a link whose template it fetches and opens.
            let text = match params["link"].as_str() {
                Some(link) => links::fetch(app, link).await?,
                None => files::read(Path::new(required(params, "path")?), format::MAX_BYTES)?,
            };
            let digest = crate::memory::hash_text(&text);
            let preview = import_preview(app, &text, params).await;
            if method.ends_with(".preview") {
                return Ok(preview);
            }
            review(params, &digest)?;
            if preview["can_import"] != true {
                return Err(issues_text(&preview));
            }
            let runner_id = required(params, "runner_id")?;
            if app.this_device_id().as_deref() == Some(runner_id) {
                import_text(app, &text, params).await
            } else {
                let mut body = params.clone();
                let fields = body.as_object_mut().ok_or("Invalid import params")?;
                fields.remove("path");
                fields.remove("link");
                body["text"] = json!(text);
                crate::requests::ask(app, runner_id, "templates.import", body).await
            }
        }
        _ => Err(format!("unknown method {method}")),
    }
}

/// Import preview is structured even for a future/unsupported file: it explains the problem
/// before any account mutation. Each requirement lists the recipient Runner's own connections for
/// its service and the one picked: the `mappings` choice, else the only ready one there. A
/// requirement without a ready pick blocks the import without an issue of its own; the apps say
/// what it needs on its row.
pub async fn import_preview(app: &Arc<App>, text: &str, params: &Value) -> Value {
    let digest = crate::memory::hash_text(text);
    let mut template = match Template::parse(text) {
        Ok(template) => template,
        Err(error) => {
            return json!({ "digest": digest, "can_import": false, "issues": [error], "requirements": [] })
        }
    };
    let mut issues = vec![];
    // The preview never shows the credential it found, and the import refuses the file.
    if template.scrub_known(&secrets::local(app)) | template.scrub() {
        issues.push("The file contains a password or key. Remove it before importing.".to_string());
    }
    if let Err(error) = validate_namespaces(&template) {
        issues.push(error);
    }
    let chosen = mappings(params).unwrap_or_else(|error| {
        issues.push(error);
        BTreeMap::new()
    });
    let statuses = match required(params, "runner_id").and_then(|id| runner_plugins(app, id)) {
        Ok(statuses) => statuses,
        Err(error) => {
            issues.push(error);
            vec![]
        }
    };
    let market = crate::marketplace::current(app);
    let mut ready = true;
    let mut resolved = BTreeMap::new();
    let mut requirements = vec![];
    for requirement in &template.requirements {
        let id = &requirement.service_id;
        let candidates: Vec<_> = statuses.iter().filter(|s| service_id(s) == *id).cloned().collect();
        let mut ready_ones = candidates.iter().filter(|s| s.state == "ready");
        let only_ready = match (ready_ones.next(), ready_ones.next()) {
            (Some(status), None) => Some(status.id.clone()),
            _ => None,
        };
        let selected = chosen.get(id).cloned().or(only_ready);
        match selected.as_ref().and_then(|picked| candidates.iter().find(|s| &s.id == picked)) {
            Some(status) if status.state == "ready" => {
                resolved.insert(id.clone(), status.id.clone());
            }
            Some(_) => ready = false,
            None if selected.is_some() => issues.push(format!("The connection picked for {id} is not one of this Runner's.")),
            None => ready = false,
        }
        let name = market.plugin(id).map(|m| m.name.clone()).or_else(|| candidates.first().map(|s| s.name.clone())).unwrap_or_else(|| id.clone());
        requirements.push(json!({ "service_id": id, "name": name, "candidates": candidates, "selected": selected }));
    }
    for key in chosen.keys() {
        if !template.requirements.iter().any(|r| &r.service_id == key) {
            issues.push(format!("Mapping {key} has no template requirement."));
        }
    }
    let mut mapped = template.clone();
    mapped.map_namespaces(
        &resolved
            .iter()
            .map(|(source, target)| (format::namespace(source), format::namespace(target)))
            .collect(),
    );
    mapped.resolve_connections(&resolved);
    if let Err(error) = mapped.validate() {
        issues.push(error);
    }
    for routine in &mapped.routines {
        if let Err(error) = routines::validate(app, routine).await {
            issues.push(error);
        }
    }
    if let Some(name) = params["name"].as_str() {
        if let Err(error) = bot_name(name) {
            issues.push(error);
        }
    }
    // How each schedule reads, for the apps; it is not part of the file.
    let mut shown = json!(template);
    for routine in shown["routines"].as_array_mut().into_iter().flatten() {
        let text = routine["schedule"].as_str().and_then(|s| crate::schedule::parse(s).ok()).map(|s| s.describe());
        routine["schedule_text"] = json!(text);
    }
    json!({ "digest": digest, "can_import": issues.is_empty() && ready, "issues": issues, "template": shown, "requirements": requirements })
}

fn bot_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 200 || name.contains('\0') {
        return Err("Give the new bot a name of at most 200 bytes.".into());
    }
    Ok(name.into())
}

/// Receives either the local file's bytes or the same reviewed bytes in a sealed request.
/// The recipient revalidates current connections; no plugin is installed or signed in here.
pub async fn import_text(app: &Arc<App>, text: &str, params: &Value) -> Result<Value, String> {
    let digest = crate::memory::hash_text(text);
    review(params, &digest)?;
    let runner_id = required(params, "runner_id")?;
    if app.this_device_id().as_deref() != Some(runner_id) {
        return Err("This import must run on its selected Runner.".into());
    }
    let preview = import_preview(app, text, params).await;
    if preview["can_import"] != true {
        return Err(issues_text(&preview));
    }
    let mut template = Template::parse(text)?;
    let mappings: BTreeMap<String, String> = preview["requirements"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| Some((r["service_id"].as_str()?.to_string(), r["selected"].as_str()?.to_string())))
        .collect();
    let name = bot_name(params["name"].as_str().or(template.profile.as_ref().map(|p| p.name.as_str())).unwrap_or(""))?;
    template.map_namespaces(
        &mappings
            .iter()
            .map(|(source, target)| (format::namespace(source), format::namespace(target)))
            .collect(),
    );
    template.resolve_connections(&mappings);
    // Mapping long account namespaces may expand content past a routine/playbook limit.
    template.validate()?;
    let profile = template.profile.unwrap_or(Profile {
        name: String::new(),
        description: String::new(),
        symbol_name: "sparkles".into(),
        accent: "indigo".into(),
    });
    let provider = params["provider"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("deepseek");
    let bot = Bot {
        id: format!("bot-{}", uuid::Uuid::new_v4().simple()),
        name,
        description: profile.description,
        symbol_name: profile.symbol_name,
        accent: profile.accent,
        avatar: None,
        runner_id: runner_id.into(),
        provider: provider.into(),
        model: None,
        thinking: None,
        legacy_instructions: String::new(),
        workdir: None,
        permissions: None,
        created_at: 0.0,
    };
    let memory = template.memories.join("\n");
    let dir = files::memory_dir(app, &bot.id)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    crate::config::set_private(&dir).map_err(|e| e.to_string())?;
    if let Err(error) = crate::memory::MemoryStore::new(dir.clone()).write_index(&memory, None) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(error.to_string());
    }
    crate::config::set_private(&dir.join("MEMORY.md")).map_err(|e| e.to_string())?;
    let (bot, chat) = match app.create_bot_with_dm(bot, None) {
        Ok(created) => created,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(error.to_string());
        }
    };
    let mut skill_receipts = vec![];
    let result = async {
        for skill in &template.skills {
            skill_receipts.push(playbooks::save(app, &bot.id, skill)?);
        }
        for routine in &template.routines {
            routines::create_paused(app, &bot.id, routine).await?;
        }
        Ok::<_, String>(json!({ "bot": bot, "chat_id": chat.meta.id }))
    }
    .await;
    if result.is_err() {
        // A bot with incomplete setup must never be left behind as a successful import.
        for receipt in skill_receipts.iter().rev() {
            if let Err(error) = playbooks::remove(app, &bot.id, receipt) {
                tracing::warn!(%error, "removing a skill from a failed template import");
            }
        }
        let _ = app.delete_bot(&bot.id);
        let _ = std::fs::remove_dir_all(&dir);
    }
    result
}

fn local_bot(app: &App, bot_id: &str) -> Result<Bot, String> {
    let bot = app.bot(bot_id).ok_or("Unknown bot")?;
    if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
        return Err("The requested memory belongs to another Runner.".into());
    }
    Ok(bot)
}

fn required<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing {key}"))
}

fn mappings(params: &Value) -> Result<BTreeMap<String, String>, String> {
    match params.get("mappings") {
        None => Ok(BTreeMap::new()),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|e| format!("Invalid recipient connection mappings: {e}")),
    }
}

fn review(params: &Value, digest: &str) -> Result<(), String> {
    if params["reviewed"].as_bool() != Some(true) {
        return Err("Review the contents and potentially personal text before continuing.".into());
    }
    if params["expected_digest"].as_str() != Some(digest) {
        return Err(
            "The contents changed since the preview. Preview and review them again.".into(),
        );
    }
    Ok(())
}

fn issues_text(preview: &Value) -> String {
    let issues: Vec<_> = preview["issues"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if issues.is_empty() {
        return "Each plugin this bot uses needs a ready connection on its Runner.".into();
    }
    issues.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Account {
        root: std::path::PathBuf,
        app: Arc<App>,
    }
    impl Account {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("lorca-template-test-{}", uuid::Uuid::new_v4()));
            let app = App::load(crate::config::Config {
                home: root.join("account"),
                port: 0,
            })
            .unwrap();
            crate::identity::create(&app, Some("Recipient".into())).unwrap();
            Self { root, app }
        }
        fn bot(&self) -> Bot {
            self.app.state.lock().unwrap().bots[0].clone()
        }
    }
    impl Drop for Account {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[tokio::test]
    async fn private_round_trip_includes_only_selected_content_and_pauses_routines() {
        let source = Account::new();
        let bot = source.bot();
        source
            .app
            .update_bot(&bot.id, |b| {
                b.description = "Work carefully. API_KEY=abcdefghijklmnop123456789".into();
                b.workdir = Some("/Users/source/private".into());
                b.provider = "source-provider".into();
            })
            .unwrap();
        let memory = crate::memory::MemoryStore::for_bot(&source.app.config.home, &bot);
        memory
            .write_index("- reusable fact\n- private unselected fact\n", None)
            .unwrap();
        memory
            .append_log("PRIVATE DIARY", None, crate::config::now_unix())
            .unwrap();
        std::fs::write(memory.dir().join("credentials.txt"), "UNRELATED FILE").unwrap();
        let routine = crate::routines::create(
            &source.app,
            &bot.id,
            "Morning",
            "every 2h",
            "Summarize",
            Some("return 'look';"),
            true,
        )
        .unwrap();
        let catalog = contents(&source.app, &bot.id).await.unwrap();
        assert_eq!(catalog.memories.len(), 2);
        assert!(!catalog.profile.content.description.contains("abcdefghijklmnop"));
        assert!(catalog.profile.flags.contains(&"credential".to_string()));
        let selection = Selection {
            profile: true,
            memory_ids: vec![catalog.memories[0].id.clone()],
            routine_ids: vec![routine.id.clone()],
            ..Selection::default()
        };
        let preview = export_preview(&source.app, &bot.id, &selection)
            .await
            .unwrap();
        assert!(preview.template.profile.as_ref().unwrap().description.contains("«redacted"));
        let path = source.root.join("private.lorca-template");
        let mut params = json!({ "bot_id": bot.id, "selection": selection, "path": path, "reviewed": true, "expected_digest": "stale" });
        assert!(dispatch(&source.app, "templates.export", &params)
            .await
            .unwrap_err()
            .contains("changed"));
        assert!(!path.exists());
        params["expected_digest"] = json!(preview.digest);
        dispatch(&source.app, "templates.export", &params)
            .await
            .unwrap();
        let mut export_params = params.clone();
        let text = std::fs::read_to_string(&path).unwrap();
        for excluded in [
            "source-provider",
            "/Users/source/private",
            "PRIVATE DIARY",
            "private unselected fact",
            "UNRELATED FILE",
            "abcdefghijklmnop",
            &bot.id,
            &bot.runner_id,
            "last_scheduled_at",
            "last_run_at",
            "last_outcome",
            "health",
        ] {
            assert!(!text.contains(excluded), "{excluded} must stay private");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(dispatch(&source.app, "templates.export", &params)
            .await
            .unwrap_err()
            .contains("exists"));
        let recipient = Account::new();
        let params = json!({ "path": path, "runner_id": recipient.app.this_device_id(), "name": "Independent", "provider": "recipient-provider", "expected_digest": crate::memory::hash_text(&text), "reviewed": true });
        let preview = dispatch(&recipient.app, "templates.import.preview", &params)
            .await
            .unwrap();
        assert_eq!(preview["can_import"], true, "{}", issues_text(&preview));
        let out = dispatch(&recipient.app, "templates.import", &params)
            .await
            .unwrap();
        let imported = recipient
            .app
            .bot(out["bot"]["id"].as_str().unwrap())
            .unwrap();
        assert_ne!(imported.id, bot.id);
        assert_eq!(imported.name, "Independent");
        assert_eq!(imported.provider, "recipient-provider");
        assert!(imported.workdir.is_none() && imported.avatar.is_none());
        assert_eq!(
            crate::memory::MemoryStore::for_bot(&recipient.app.config.home, &imported).read_index(),
            "- reusable fact\n"
        );
        let imported_routines = recipient.app.routines_of(&imported.id);
        assert_eq!(imported_routines.len(), 1);
        assert!(!imported_routines[0].is_enabled);
        assert_eq!(
            imported_routines[0].check.as_deref(),
            Some("return 'look';")
        );
        assert!(
            imported_routines[0].last_run_at.is_none()
                && imported_routines[0].last_outcome.is_none()
        );
        assert!(source.app.routine(&routine.id).unwrap().is_enabled);
        assert!(recipient
            .app
            .store
            .page(out["chat_id"].as_str().unwrap(), None, 10)
            .unwrap()
            .0
            .is_empty());
        source
            .app
            .update_bot(&bot.id, |b| b.description.push_str(" changed"))
            .unwrap();
        export_params["overwrite"] = json!(true);
        assert!(dispatch(&source.app, "templates.export", &export_params)
            .await
            .unwrap_err()
            .contains("changed"));
    }

    #[tokio::test]
    async fn invalid_unreviewed_and_changed_files_never_create_a_bot() {
        let account = Account::new();
        let path = account.root.join("invalid.lorca-template");
        let before = account.app.state.lock().unwrap().bots.len();
        let mut value = json!(Template {
            profile: Some(Profile {
                name: "New".into(),
                description: "".into(),
                symbol_name: "sparkles".into(),
                accent: "indigo".into()
            }),
            ..Template::default()
        });
        value["provider"] = json!("must-not-import");
        std::fs::write(&path, value.to_string()).unwrap();
        let mut params = json!({ "path": path, "runner_id": account.app.this_device_id() });
        let preview = dispatch(&account.app, "templates.import.preview", &params)
            .await
            .unwrap();
        assert_eq!(preview["can_import"], false);
        assert!(issues_text(&preview).contains("provider"));
        params["reviewed"] = json!(true);
        params["expected_digest"] = preview["digest"].clone();
        assert!(dispatch(&account.app, "templates.import", &params)
            .await
            .is_err());
        value.as_object_mut().unwrap().remove("provider");
        std::fs::write(&path, value.to_string()).unwrap();
        assert!(dispatch(&account.app, "templates.import", &params)
            .await
            .unwrap_err()
            .contains("changed"));
        let preview = dispatch(&account.app, "templates.import.preview", &params)
            .await
            .unwrap();
        params["expected_digest"] = preview["digest"].clone();
        params["reviewed"] = json!(false);
        assert!(dispatch(&account.app, "templates.import", &params)
            .await
            .unwrap_err()
            .contains("Review"));
        assert_eq!(account.app.state.lock().unwrap().bots.len(), before);
    }

    #[tokio::test]
    async fn plugin_requirements_need_recipient_ready_connections_and_selected_namespaces() {
        let account = Account::new();
        let mut template = Template {
            requirements: vec![Requirement {
                service_id: "test-service".into(),
            }],
            memories: vec!["tools.test_service__read()".into()],
            ..Template::default()
        };
        let text = serde_json::to_string(&template).unwrap();
        let mut params = json!({ "name": "New", "runner_id": account.app.this_device_id() });
        let missing = import_preview(&account.app, &text, &params).await;
        assert_eq!(missing["can_import"], false);
        assert_eq!(missing["requirements"][0]["candidates"], json!([]));
        assert_eq!(missing["requirements"][0]["name"], "test-service");
        let manifest = crate::plugins::Manifest::parse(&json!({ "id": "test-service", "name": "Test service", "description": "Test", "servers": { "test": { "type": "stdio", "command": "true" } } })).unwrap();
        crate::plugins::install(&account.app, manifest, "inline").unwrap();
        // The test supplies a ready advertisement without invoking a server or provider.
        account
            .app
            .plugins
            .lock()
            .unwrap()
            .notes
            .insert("test-service".into(), ("ready".into(), "Ready".into()));
        // The only ready connection is picked, and shown as picked.
        let picked = import_preview(&account.app, &text, &params).await;
        assert_eq!(picked["can_import"], true, "{}", issues_text(&picked));
        assert_eq!(picked["requirements"][0]["selected"], "test-service");
        assert_eq!(picked["requirements"][0]["name"], "Test service");
        params["mappings"] = json!({ "test-service": "someone-elses-account" });
        assert!(issues_text(&import_preview(&account.app, &text, &params).await).contains("not one of this Runner's"));
        params["mappings"] = json!({ "test-service": "test-service" });
        assert_eq!(import_preview(&account.app, &text, &params).await["can_import"], true);
        account.app.plugins.lock().unwrap().notes.insert(
            "test-service".into(),
            ("needs_auth".into(), "Sign in".into()),
        );
        let unready = import_preview(&account.app, &text, &params).await;
        assert_eq!(unready["can_import"], false);
        assert_eq!(unready["requirements"][0]["candidates"][0]["state"], "needs_auth");
        template.requirements.clear();
        assert!(validate_namespaces(&template)
            .unwrap_err()
            .contains("test_service"));
    }

    #[tokio::test]
    async fn skills_are_saved_in_the_new_bots_scope() {
        let account = Account::new();
        let text = serde_json::to_string(&Template {
            skills: vec![Skill {
                name: "review".into(),
                description: "Review code".into(),
                instructions: "Read first".into(),
                examples: String::new(),
                references: vec![],
                scripts: vec![],
            }],
            ..Template::default()
        })
        .unwrap();
        let mut params = json!({ "name": "New", "runner_id": account.app.this_device_id() });
        let preview = import_preview(&account.app, &text, &params).await;
        assert_eq!(preview["can_import"], true, "{}", issues_text(&preview));
        params["expected_digest"] = preview["digest"].clone();
        params["reviewed"] = json!(true);
        let imported = import_text(&account.app, &text, &params).await.unwrap();
        let bot_id = imported["bot"]["id"].as_str().unwrap();
        let skills = playbooks::list(&account.app, bot_id);
        assert_eq!(skills.len(), 1);
        let content = playbooks::export(&account.app, bot_id, skills[0]["id"].as_str().unwrap()).unwrap();
        assert_eq!(content.name, "review");
        assert!(playbooks::list(&account.app, &account.bot().id).is_empty());

        // The skill comes back out of the bot it went into.
        let contents = contents(&account.app, bot_id).await.unwrap();
        assert_eq!(contents.skills.iter().map(|item| item.content.name.as_str()).collect::<Vec<_>>(), ["review"]);
    }

    #[tokio::test]
    async fn explicit_routine_configuration_is_preserved_or_blocked_before_setup() {
        let account = Account::new();
        let text = json!({ "format": format::FORMAT, "version": format::VERSION,
            "routines": [{ "name": "Morning", "schedule": "0 9 * * *", "prompt": "Read inbox",
                "check": "return null;", "timezone": "America/New_York", "missed_run_policy": "skip" }]
        }).to_string();
        let mut params = json!({ "name": "New", "runner_id": account.app.this_device_id() });
        let preview = import_preview(&account.app, &text, &params).await;
        let capability = Box::pin(crate::api::dispatch(
            &account.app,
            "routines.describe",
            json!({ "schedule": "every 2h" }),
        ))
        .await
        .unwrap();
        if capability.get("timezone").is_none() {
            assert_eq!(preview["can_import"], false);
            assert!(issues_text(&preview).contains("unsupported scheduling field timezone"));
            assert_eq!(account.app.state.lock().unwrap().bots.len(), 1);
        } else {
            assert_eq!(preview["can_import"], true, "{}", issues_text(&preview));
            params["expected_digest"] = preview["digest"].clone();
            params["reviewed"] = json!(true);
            let imported = import_text(&account.app, &text, &params).await.unwrap();
            let stored = account
                .app
                .routines_of(imported["bot"]["id"].as_str().unwrap());
            let wire = json!(stored[0]);
            assert_eq!(wire["timezone"], "America/New_York");
            assert_eq!(wire["missed_run_policy"], "skip");
            assert_eq!(wire["is_enabled"], false);
            assert_eq!(wire["check"], "return null;");
            let portable = json!(routines::export(stored[0].clone()));
            assert!(
                portable.get("last_scheduled_at").is_none() && portable.get("health").is_none()
            );
        }
    }

    #[tokio::test]
    async fn cannot_create_a_check_on_another_runner() {
        let account = Account::new();
        let remote = crate::model::Device {
            id: "remote-runner".into(),
            name: "Remote".into(),
            os: "linux".into(),
            ..Default::default()
        };
        account.app.state.lock().unwrap().devices.push(remote);
        let out = crate::api::dispatch(
            &account.app,
            "bots.create",
            json!({ "name": "Remote bot", "runner_id": "remote-runner" }),
        )
        .await
        .unwrap();
        let bot_id = out["bot"]["id"].as_str().unwrap();
        let request = json!({ "bot_id": bot_id, "name": "Check", "schedule": "every 2h", "prompt": "Read", "check": "return null;", "enabled": false });
        assert!(
            crate::api::dispatch(&account.app, "routines.create", request)
                .await
                .unwrap_err()
                .contains("assigned Runner")
        );
        assert!(account.app.routines_of(bot_id).is_empty());
    }

    #[tokio::test]
    async fn opaque_saved_provider_and_plugin_credentials_are_redacted_from_selected_text() {
        let account = Account::new();
        let provider_secret = "OpaqueProviderValueWithoutKnownPrefix";
        let plugin_secret = "OpaqueIntegrationValueWithoutKnownPrefix";
        account.app.credentials.lock().unwrap().deepseek =
            Some(crate::credentials::ApiKeyCredential {
                api_key: provider_secret.into(),
                base_url: None,
                connected_at: 1,
            });
        let manifest = crate::plugins::Manifest::parse(&json!({ "id": "test-secret", "name": "Test", "description": "Test", "servers": { "test": { "type": "stdio", "command": "true" } }, "variables": [{ "name": "OPAQUE", "secret": true }] })).unwrap();
        crate::plugins::install(&account.app, manifest, "inline").unwrap();
        crate::plugins::set_variables(
            &account.app,
            "test-secret",
            &BTreeMap::from([("OPAQUE".into(), plugin_secret.into())]),
        )
        .unwrap();
        let bot = account.bot();
        account
            .app
            .update_bot(&bot.id, |b| {
                b.description = format!("{provider_secret} {plugin_secret}")
            })
            .unwrap();
        crate::memory::MemoryStore::for_bot(&account.app.config.home, &bot)
            .write_index(&format!("- {plugin_secret}\n"), None)
            .unwrap();
        let catalog = contents(&account.app, &bot.id).await.unwrap();
        assert!(!catalog.memories[0].content.contains(plugin_secret));
        assert!(!catalog.profile.content.description.contains(provider_secret));
        assert_eq!(catalog.memories[0].flags, ["credential"]);
        let selection = Selection {
            profile: true,
            memory_ids: vec![catalog.memories[0].id.clone()],
            ..Default::default()
        };
        let preview = export_preview(&account.app, &bot.id, &selection)
            .await
            .unwrap();
        let text = serde_json::to_string(&preview.template).unwrap();
        assert!(!text.contains(provider_secret) && !text.contains(plugin_secret));
        let receiver = Account::new();
        let params = json!({ "runner_id": receiver.app.this_device_id(), "name": "New" });
        assert_eq!(
            import_preview(&receiver.app, &text, &params).await["can_import"],
            true
        );
        let raw = serde_json::to_string(&Template {
            memories: vec![
                provider_secret.into(),
                "API_KEY=abcdefghijklmnop123456789".into(),
            ],
            ..Default::default()
        })
        .unwrap();
        let params = json!({ "runner_id": account.app.this_device_id(), "name": "New" });
        let blocked = import_preview(&account.app, &raw, &params).await;
        assert_eq!(blocked["can_import"], false);
        assert!(issues_text(&blocked).contains("password or key"));
        assert!(!blocked["template"].to_string().contains(provider_secret));
        assert!(!blocked["template"]
            .to_string()
            .contains("abcdefghijklmnop123456789"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn source_links_and_runtime_export_destinations_are_refused() {
        use std::os::unix::fs::symlink;
        let account = Account::new();
        let bot = account.bot();
        let dir = files::memory_dir(&account.app, &bot.id).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let secret = account.root.join("secret.txt");
        std::fs::write(&secret, "private secret").unwrap();
        symlink(&secret, dir.join("MEMORY.md")).unwrap();
        assert!(contents(&account.app, &bot.id)
            .await
            .unwrap_err()
            .contains("symbolic link"));
        std::fs::remove_file(dir.join("MEMORY.md")).unwrap();
        symlink(&account.root, dir.join("memory")).unwrap();
        assert!(contents(&account.app, &bot.id)
            .await
            .unwrap_err()
            .contains("directory"));
        assert!(files::export(
            &account.app,
            &account.app.config.home.join("test.lorca-template"),
            "{}",
            false
        )
        .unwrap_err()
        .contains("data folder"));
    }
}
