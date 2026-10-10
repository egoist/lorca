//! The JSON API every app speaks: `{ method, params }` in, a result or an error out, with the
//! events on the App's bus beside it. The websocket server hands requests here; the phone
//! calls it directly.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::app::App;
use crate::model::*;
#[cfg(feature = "provider-auth")]
use crate::provider_auth;
use crate::{identity, pairing, requests, routines, runtime};

fn string(params: &Value, key: &str) -> Result<String, String> {
    params[key].as_str().map(str::to_string).filter(|s| !s.is_empty()).ok_or_else(|| format!("missing {key}"))
}

fn opt_string(params: &Value, key: &str) -> Option<String> {
    params[key].as_str().map(str::to_string).filter(|s| !s.is_empty())
}

fn parse_permissions(params: &Value) -> Result<Option<crate::permissions::BotPermissions>, String> {
    let Some(value) = params.get("permissions") else { return Ok(None) };
    let policy: crate::permissions::BotPermissions = serde_json::from_value(value.clone())
        .map_err(|error| format!("Invalid bot permissions: {error}. Use an explicit policy to change access."))?;
    policy.validate()?;
    Ok(Some(policy))
}

/// A Runner opens provider OAuth in its browser. A phone emits the URL to the Expo app,
/// whose in-app browser keeps the core alive for the localhost callback.
#[cfg(feature = "provider-auth")]
fn open_provider_auth(app: &Arc<App>, kind: &str, url: &str) -> Result<(), String> {
    #[cfg(feature = "runner")]
    {
        let _ = app;
        let _ = kind;
        open::that(url).map_err(|e| format!("Cannot open the browser: {e}"))
    }
    #[cfg(not(feature = "runner"))]
    {
        app.emit(crate::events::Event::ProviderAuth { kind: kind.to_string(), url: url.to_string() });
        Ok(())
    }
}

/// A bot's profile image from the `avatar` param: `None` when the param is absent (leave it),
/// `Some(None)` when it is null (remove it), and `Some(Some(_))` for a `{ path, … }` file,
/// which is copied into the store and queued as a `file` blob like a message attachment.
fn store_avatar(app: &Arc<App>, params: &Value) -> Result<Option<Option<Attachment>>, String> {
    match params.get("avatar") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(value) => {
            let file: crate::files::OutgoingFile = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
            let attachment = crate::files::store(app, &file).map_err(|e| e.to_string())?;
            if !attachment.is_image() {
                let _ = std::fs::remove_file(crate::files::local_path(app, &attachment.id));
                return Err(format!("{} is not an image", attachment.name));
            }
            crate::files::push_blob(app, None, &attachment).map_err(|e| e.to_string())?;
            Ok(Some(Some(attachment)))
        }
    }
}

/// How long `device.update` waits for another Runner, which reads the latest release's
/// manifest before it answers.
const UPDATE_WAIT: std::time::Duration = std::time::Duration::from_secs(45);

pub async fn dispatch(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    if method.starts_with("attention.") {
        return crate::attention::dispatch(app, method, params, None);
    }
    match method {
        method if method.starts_with("templates.") => crate::templates::dispatch(app, method, &params).await,
        method if method.starts_with("playbooks.") => crate::playbooks::dispatch(app, method, params).await,
        method if method.starts_with("events.") => crate::event_triggers::dispatch(app, method, params).await,
        "projects.get" | "projects.save" | "projects.refresh" | "projects.asset" | "projects.asset_path" => {
            crate::project_context::dispatch(app, method, params).await
        }
        method if method.starts_with("budgets.") => crate::budgets::dispatch(app, method, &params).await,
        #[cfg(feature = "runner")]
        method if method.starts_with("connector_limits.") => crate::connector_limits::dispatch(app, method, &params).await,
        #[cfg(not(feature = "runner"))]
        method if method.starts_with("connector_limits.") => {
            let runner = string(&params, "runner_id")?;
            requests::ask(app, &runner, method, params).await
        }
        method if method.starts_with("reviews.") => crate::review_queue::dispatch(app, method, params).await,
        method if method.starts_with("tasks.") => crate::tasks::dispatch(app, method, params).await,
        method if method.starts_with("handoffs.") => crate::handoffs::dispatch(app, method, params),
        "hello" => Ok(json!({
            "version": crate::config::VERSION,
            "has_identity": app.has_identity(),
            "is_identity_device": app.is_identity_device(),
            "device_id": app.this_device_id(),
            "relay_url": app.relay_url(),
            "relay_connected": app.relay_connected.load(std::sync::atomic::Ordering::Relaxed),
            "relay_update_required": app.relay_update_required.load(std::sync::atomic::Ordering::Relaxed),
            "relay_error": app.relay_problem.lock().unwrap().clone(),
        })),
        // An app connecting checks for a newer model catalog and marketplace index, unless each
        // was checked within the hour.
        "bootstrap" => {
            crate::catalog::check_in_background(app);
            crate::marketplace::check_in_background(app);
            Ok(app.snapshot())
        }
        method if method.starts_with("browser.") => crate::browser::dispatch(app, method, params).await,
        // A Runner's saved secrets: listed without their values, replaced, or deleted, there when
        // this Device is the Runner, else sealed to it. A new value travels only sealed.
        "secrets.list" | "secrets.set" | "secrets.delete" => {
            let runner_id = string(&params, "runner_id")?;
            let mut body = params.clone();
            if let Some(fields) = body.as_object_mut() {
                fields.remove("runner_id");
            }
            if app.this_device_id().as_deref() == Some(runner_id.as_str()) {
                #[cfg(feature = "runner")]
                return crate::secrets::serve(app, method, &body);
            }
            requests::ask(app, &runner_id, method, body).await
        }

        "identity.create" => {
            let phrase = identity::create(app, opt_string(&params, "device_name")).map_err(|e| e.to_string())?;
            Ok(json!({ "phrase": phrase, "device_id": app.this_device_id() }))
        }
        "identity.restore" => {
            let phrase = string(&params, "phrase")?;
            identity::restore(app, &phrase, opt_string(&params, "device_name")).await.map_err(|e| e.to_string())?;
            Ok(json!({ "device_id": app.this_device_id() }))
        }

        "pair.start" => {
            let (nonce, pairing_string) = pairing::start(app.clone()).await.map_err(|e| e.to_string())?;
            Ok(json!({ "nonce": nonce, "pairing_string": pairing_string }))
        }
        "pair.status" => Ok(pairing::status(app, &string(&params, "nonce")?)),
        "pair.cancel" => {
            pairing::cancel(app, &string(&params, "nonce")?);
            Ok(Value::Null)
        }
        "pair.accept" => {
            let text = string(&params, "pairing_string")?;
            pairing::accept(app.clone(), &text, opt_string(&params, "device_name")).await.map_err(|e| e.to_string())
        }
        "pair.abort" => {
            pairing::abort(app);
            Ok(Value::Null)
        }

        "device.rename" => {
            let name = string(&params, "name")?;
            app.rename_device(&name).map_err(|e| e.to_string())?;
            Ok(json!({ "name": name }))
        }
        // Another Device by id, or this one: the latter is `identity.forget`.
        "device.unpair" => {
            let id = string(&params, "id")?;
            if app.this_device_id().as_deref() == Some(id.as_str()) {
                crate::sync::revoke_self(app).await;
                app.forget_identity().map_err(|e| e.to_string())?;
            } else {
                crate::sync::unpair_device(app, &id).await?;
            }
            Ok(Value::Null)
        }
        // Another Device's CLI by id, or this one's (no id: also before it pairs): installs the
        // latest release, which the CLI restarts into once no bot is at work there.
        "device.update" => {
            let id = opt_string(&params, "id").filter(|id| app.this_device_id().as_deref() != Some(id.as_str()));
            let Some(id) = id else {
                #[cfg(feature = "cli")]
                return crate::update::install_now(app).await;
                #[cfg(not(feature = "cli"))]
                return Err("This Device's Lorca updates with its app.".into());
            };
            crate::requests::ask_within(app, &id, "update.install", json!({}), UPDATE_WAIT).await
        }
        "device.auto_update" => {
            let id = opt_string(&params, "id").filter(|id| app.this_device_id().as_deref() != Some(id.as_str()));
            let on = params["on"].as_bool().ok_or("missing on")?;
            let Some(id) = id else {
                #[cfg(feature = "cli")]
                return crate::update::set_auto(app, on);
                #[cfg(not(feature = "cli"))]
                return Err("This Device's Lorca updates with its app.".into());
            };
            crate::requests::ask(app, &id, "update.auto", json!({ "on": on })).await
        }
        "identity.forget" => {
            crate::sync::revoke_self(app).await;
            app.forget_identity().map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        // The account goes from the relay first; a relay that cannot be told leaves it in place.
        "identity.delete" => {
            crate::sync::delete_identity(app).await?;
            app.forget_identity().map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        // Onboarding after a pair or a restore: the account's providers, once the first pull
        // has brought its credentials.
        "sync.account" => {
            crate::sync::wait_for_account(app, crate::sync::ACCOUNT_WAIT).await;
            Ok(json!({ "providers": app.credentials.lock().unwrap().statuses() }))
        }
        // The app came back to the foreground: ask the relay again now, not after the backoff.
        "sync.wake" => {
            app.wake_sync();
            Ok(Value::Null)
        }

        // The desktop app names the chat on screen while it is frontmost, null otherwise; a
        // reply the user is watching arrive is not pushed to their phone, from this Runner or
        // another.
        "ui.watching" => {
            app.watch_chat(opt_string(&params, "chat_id"));
            Ok(Value::Null)
        }
        // A phone's APNs or FCM device token, registered with the relay under this machine.
        "push.register" => {
            let (platform, device_token) = (string(&params, "platform")?, string(&params, "token")?);
            let url = app.relay_url().ok_or("No relay configured")?;
            let machine = app.machine_file().and_then(|m| m.machine().ok()).ok_or("Not paired")?;
            let token = crate::sync::token_or_register(app, &url, &machine).await.map_err(|e| e.to_string())?;
            app.relay.put_push_token(&url, &token, &platform, &device_token, params["environment"].as_str()).await.map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "push.unregister" => {
            let url = app.relay_url().ok_or("No relay configured")?;
            let machine = app.machine_file().and_then(|m| m.machine().ok()).ok_or("Not paired")?;
            let token = crate::sync::token_or_register(app, &url, &machine).await.map_err(|e| e.to_string())?;
            app.relay.delete_push_token(&url, &token).await.map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }

        "config.set" => {
            if params.get("relay_url").is_some() {
                let url = params["relay_url"].as_str().map(str::to_string);
                app.set_relay_url(url).map_err(|e| e.to_string())?;
            }
            Ok(json!({ "relay_url": app.relay_url() }))
        }

        method if method.starts_with("workflows.") => crate::workflows::handle(app, method, &params).await,

        "bots.create" => {
            // A bot from the marketplace starts from its template's profile, with the routines
            // and the first turn `marketplace::welcome` gives it.
            let template = match opt_string(&params, "template_id") {
                Some(id) => Some(crate::marketplace::template(app, &id).await?),
                None => None,
            };
            let from_template = |pick: fn(&crate::marketplace::BotTemplate) -> &String| template.as_ref().map(|(t, _)| pick(t).clone());
            let bot = Bot {
                id: opt_string(&params, "id").unwrap_or_default(),
                name: opt_string(&params, "name").or_else(|| from_template(|t| &t.name)).ok_or("missing name")?,
                description: opt_string(&params, "description").or_else(|| from_template(|t| &t.description)).unwrap_or_default(),
                symbol_name: opt_string(&params, "symbol_name").or_else(|| from_template(|t| &t.symbol_name)).unwrap_or_else(|| "sparkles".into()),
                accent: opt_string(&params, "accent").or_else(|| from_template(|t| &t.accent)).unwrap_or_else(|| "indigo".into()),
                avatar: store_avatar(app, &params)?.flatten(),
                runner_id: string(&params, "runner_id")?,
                provider: opt_string(&params, "provider").unwrap_or_else(|| "deepseek".into()),
                model: opt_string(&params, "model"),
                thinking: opt_string(&params, "thinking"),
                // Older apps sent a second behavioral field. `insert_bot` folds it into the
                // description, then clears this rolling-upgrade slot.
                legacy_instructions: opt_string(&params, "instructions").unwrap_or_default(),
                workdir: opt_string(&params, "workdir"),
                permissions: parse_permissions(&params)?,
                created_at: 0.0,
            };
            // Every bot has one direct chat; both land in a single roster change.
            let (bot, chat) = app.create_bot_with_dm(bot, opt_string(&params, "chat_id")).map_err(|e| e.to_string())?;
            if let Some((template, plugins)) = template {
                crate::marketplace::welcome(app, &bot, &chat.meta.id, &template, plugins, opt_string(&params, "greeting"));
            }
            Ok(json!({ "bot": bot, "chat_id": chat.meta.id }))
        }
        "bots.update" => {
            let id = string(&params, "id")?;
            let permissions = parse_permissions(&params)?;
            let access_changed = permissions.is_some();
            // The image is copied and queued before the roster names it, so every Device can
            // fetch the blob by the time it reads the profile.
            let avatar = store_avatar(app, &params)?;
            let bot = app.update_bot(&id, |bot| {
                if let Some(v) = opt_string(&params, "name") { bot.name = v; }
                if let Some(v) = opt_string(&params, "symbol_name") { bot.symbol_name = v; }
                if let Some(v) = opt_string(&params, "accent") { bot.accent = v; }
                if let Some(v) = avatar { bot.avatar = v; }
                if let Some(v) = params["description"].as_str() {
                    bot.description = v.trim().to_string();
                } else if let Some(v) = params["instructions"].as_str() {
                    // Compatibility for an older app updating the retired field.
                    bot.description = v.trim().to_string();
                }
                bot.legacy_instructions.clear();
                if let Some(v) = opt_string(&params, "provider") { bot.provider = v; }
                if let Some(v) = params["model"].as_str() { bot.model = Some(v.trim().to_string()).filter(|m| !m.is_empty()); }
                if let Some(v) = params["thinking"].as_str() { bot.thinking = Some(v.trim().to_string()).filter(|t| !t.is_empty()); }
                if let Some(v) = opt_string(&params, "runner_id") { bot.runner_id = v; }
                if let Some(v) = params["workdir"].as_str() { bot.workdir = Some(v.to_string()).filter(|w| !w.trim().is_empty()); }
                if let Some(v) = permissions { bot.permissions = Some(v); }
                // A draft card's Send Directly changes only this, whatever else the policy says.
                if let Some(v) = params["drafts"].as_bool() {
                    bot.permissions.get_or_insert_with(Default::default).drafts = v;
                }
            })
            .map_err(|e| e.to_string())?;
            if access_changed { crate::permissions::dismiss_requests(app, &id); }
            Ok(json!({ "bot": bot }))
        }
        // The plugins the bot's Runner has and their tools, for its Access sheet.
        "bots.permissions" => {
            let bot = app.bot(&string(&params, "id")?).ok_or("Unknown bot")?;
            let catalog = crate::plugins::on_runner(app, &bot.runner_id, "permissions.catalog", json!({})).await?;
            Ok(json!({ "connections": catalog }))
        }
        "bots.delete" => {
            app.delete_bot(&string(&params, "id")?).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }

        "chats.create" => {
            let bot_ids: Vec<String> = serde_json::from_value(params["bot_ids"].clone()).unwrap_or_default();
            let kind = opt_string(&params, "kind").unwrap_or_else(|| "group".into());
            let chat = if kind == "dm" {
                let bot_id = bot_ids.first().cloned().ok_or("missing bot_ids")?;
                app.dm_with(&bot_id, opt_string(&params, "id")).map_err(|e| e.to_string())?
            } else {
                app.create_chat(ChatMeta {
                    id: opt_string(&params, "id").unwrap_or_default(),
                    kind,
                    title: opt_string(&params, "title"),
                    owner_bot_id: opt_string(&params, "owner_bot_id").or_else(|| bot_ids.first().cloned()),
                    description: opt_string(&params, "description").map(|text| text.trim().to_string()),
                    bot_ids,
                    is_pinned: false,
                    section_id: None,
                    is_hidden: false,
                    mute: None,
                    created_at: 0.0,
                    channel: None,
                })
                .map_err(|e| e.to_string())?
            };
            Ok(json!({ "chat": chat }))
        }
        "chats.dm" => {
            let chat = app.dm_with(&string(&params, "bot_id")?, opt_string(&params, "id")).map_err(|e| e.to_string())?;
            Ok(json!({ "chat": chat }))
        }
        "chats.send" => {
            let files: Vec<crate::files::OutgoingFile> = serde_json::from_value(params["attachments"].clone()).unwrap_or_default();
            let mut attachments = Vec::new();
            for file in &files {
                attachments.push(crate::files::store(app, file).map_err(|e| e.to_string())?);
            }
            let text = opt_string(&params, "text").unwrap_or_default();
            let mentions: Vec<String> = serde_json::from_value(params["mentions"].clone()).unwrap_or_default();
            let message =
                runtime::send_user_message(
                    app.clone(),
                    &string(&params, "chat_id")?,
                    &text,
                    opt_string(&params, "message_id"),
                    attachments,
                    mentions,
                    opt_string(&params, "reply_to"),
                )
                    .map_err(|e| e.to_string())?;
            Ok(json!({ "message": message }))
        }
        "outputs.list" => {
            let chat_id = string(&params, "chat_id")?;
            let task_id = opt_string(&params, "task_id");
            let messages = crate::outputs::list(app, &chat_id, task_id.as_deref())?;
            Ok(json!({ "outputs": messages.into_iter().map(|message| message.for_app()).collect::<Vec<_>>() }))
        }
        #[cfg(feature = "runner")]
        "outputs.publish" => {
            let chat_id = string(&params, "chat_id")?;
            let bot_id = string(&params, "bot_id")?;
            let bot = app.bot(&bot_id).ok_or("Unknown producing bot")?;
            let workdir = bot.working_directory(&app.config.home);
            let mut payload = params;
            payload.as_object_mut().ok_or("Output parameters must be an object")?.remove("chat_id");
            payload.as_object_mut().unwrap().remove("bot_id");
            let request = serde_json::from_value(payload).map_err(|error| format!("Invalid output: {error}"))?;
            let app = app.clone();
            let message = tokio::task::spawn_blocking(move || crate::outputs::publish(&app, &chat_id, &bot_id, &workdir, request)).await
                .map_err(|error| error.to_string())??;
            Ok(json!({ "message": message.for_app(), "task_evidence": message.output.as_ref().map(|output| output.task_evidence(&message.id)) }))
        }
        "files.path" => {
            // Where the attachment's bytes are on this machine, fetched from the relay first
            // when another Device sent it.
            let attachment: Attachment = serde_json::from_value(params["attachment"].clone()).map_err(|e| e.to_string())?;
            let mut path = crate::files::ensure_local(app, &attachment).await.map_err(|e| e.to_string())?;
            if params["named"].as_bool() == Some(true) {
                path = crate::files::named_local_path(app, &attachment).map_err(|e| e.to_string())?;
            }
            Ok(json!({ "path": path }))
        }
        "chats.stop" => {
            runtime::cancel_chat(app, &string(&params, "chat_id")?);
            Ok(Value::Null)
        }
        // A message the direct chat's turn holds for its next step: read now. Here when the bot
        // runs here, else sealed to its Runner.
        "chats.send_now" => {
            let chat_id = string(&params, "chat_id")?;
            let message_id = string(&params, "message_id")?;
            let chat = app.chat(&chat_id).ok_or("Unknown chat")?;
            if chat.meta.is_group() {
                return Err("Send now is for a direct chat".into());
            }
            let bot = chat.meta.bot_ids.first().and_then(|id| app.bot(id)).ok_or("The chat has no bot")?;
            if app.this_device_id().as_deref() == Some(bot.runner_id.as_str()) {
                #[cfg(feature = "runner")]
                return crate::turns::send_now(app, &chat_id, &message_id).map(|sent| json!({ "sent": sent }));
            }
            requests::ask(app, &bot.runner_id, "chats.send_now", json!({ "chat_id": chat_id, "message_id": message_id })).await
        }
        #[cfg(feature = "runner")]
        "chats.compact" => {
            let chat_id = string(&params, "chat_id")?;
            let tokens_before = crate::turns::compact_now(app, &chat_id, opt_string(&params, "bot_id").as_deref()).await?;
            Ok(json!({ "tokens_before": tokens_before }))
        }
        // A bot's memory lives on its Runner: read and written here for the bots this Device
        // runs, and through a sealed request to the Runner for the others.
        "bots.memory" => {
            let bot = app.bot(&string(&params, "bot_id")?).ok_or("Unknown bot")?;
            let runner = app.device(&bot.runner_id).map(|d| d.name).unwrap_or_else(|| "its Runner".into());
            let here = app.this_device_id().as_deref() == Some(bot.runner_id.as_str());
            let mut overview = if here {
                requests::memory_read(app, &bot.id)?
            } else {
                requests::ask(app, &bot.runner_id, "memory.read", json!({ "bot_id": bot.id })).await?
            };
            overview["bot_id"] = json!(bot.id);
            overview["here"] = json!(here);
            overview["runner"] = json!(runner);
            Ok(overview)
        }
        "bots.memory.write" => {
            let bot = app.bot(&string(&params, "bot_id")?).ok_or("Unknown bot")?;
            let text = params["text"].as_str().ok_or("missing text")?;
            let expected_hash = opt_string(&params, "expected_hash");
            if app.this_device_id().as_deref() == Some(bot.runner_id.as_str()) {
                requests::memory_write(app, &bot.id, text, expected_hash.as_deref())
            } else {
                requests::ask(app, &bot.runner_id, "memory.write", json!({ "bot_id": bot.id, "text": text, "expected_hash": expected_hash })).await
            }
        }
        "chats.delete" => {
            app.delete_chat(&string(&params, "chat_id")?);
            Ok(Value::Null)
        }
        "chats.rename" => {
            let title = params["title"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            app.rename_chat(&string(&params, "chat_id")?, title).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "chats.set_description" => {
            let description = params["description"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            app.describe_chat(&string(&params, "chat_id")?, description).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "chats.pin" => {
            app.pin_chat(&string(&params, "chat_id")?, params["pinned"].as_bool()).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "chats.hide" => {
            app.hide_chat(&string(&params, "chat_id")?, params["hidden"].as_bool().unwrap_or(true)).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        // `until` is unix seconds; without it the chat stays muted until unmuted.
        "chats.mute" => {
            let mute = params["muted"].as_bool().unwrap_or(true).then(|| Mute { until: params["until"].as_f64() });
            app.mute_chat(&string(&params, "chat_id")?, mute).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        // A null `section_id` lists the chat with the chats in no section.
        "chats.set_section" => {
            app.set_chat_section(&string(&params, "chat_id")?, opt_string(&params, "section_id")).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "sections.create" => {
            let section = app
                .create_section(opt_string(&params, "id"), &string(&params, "name")?, opt_string(&params, "chat_id").as_deref())
                .map_err(|e| e.to_string())?;
            Ok(json!({ "section": section }))
        }
        "sections.rename" => {
            app.rename_section(&string(&params, "id")?, &string(&params, "name")?).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "sections.collapse" => {
            let collapsed = params["collapsed"].as_bool().unwrap_or(true);
            app.update_section(&string(&params, "id")?, |section| section.collapsed = collapsed).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "sections.reorder" => {
            let ids: Vec<String> = serde_json::from_value(params["ids"].clone()).map_err(|_| "ids must be a list of section ids")?;
            app.reorder_sections(&ids);
            Ok(Value::Null)
        }
        "sections.delete" => {
            app.delete_section(&string(&params, "id")?).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "chats.add_bot" => {
            let chat_id = string(&params, "chat_id")?;
            let bot_id = string(&params, "bot_id")?;
            let bot = app.bot(&bot_id).ok_or("Unknown bot")?;
            app.update_chat_meta(&chat_id, |meta| {
                if meta.is_group() && meta.bot_ids.len() < MAX_GROUP_BOTS && !meta.bot_ids.contains(&bot_id) {
                    meta.bot_ids.push(bot_id.clone());
                }
            })
            .map_err(|e| e.to_string())?;
            app.notice(&chat_id, format!("{} joined the chat.", bot.name));
            Ok(Value::Null)
        }
        "chats.remove_bot" => {
            let chat_id = string(&params, "chat_id")?;
            let bot_id = string(&params, "bot_id")?;
            app.update_chat_meta(&chat_id, |meta| {
                if meta.is_group() && meta.bot_ids.len() > 1 {
                    meta.bot_ids.retain(|id| id != &bot_id);
                    if meta.owner_bot_id.as_deref() == Some(bot_id.as_str()) {
                        meta.owner_bot_id = meta.bot_ids.first().cloned();
                    }
                }
            })
            .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "chats.set_owner" => {
            let chat_id = string(&params, "chat_id")?;
            let bot_id = string(&params, "bot_id")?;
            let chat = app.chat(&chat_id).ok_or("Unknown chat")?;
            if !chat.meta.is_group() {
                return Err("Only a group has an owner".into());
            }
            if !chat.meta.bot_ids.contains(&bot_id) {
                return Err(format!("{} is not in {}", crate::runtime::name_of(app, &bot_id), app.chat_title(&chat.meta)));
            }
            app.update_chat_meta(&chat_id, |meta| meta.owner_bot_id = Some(bot_id.clone())).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "chats.search" => {
            let query = string(&params, "query")?;
            let limit = params["limit"].as_u64().unwrap_or(20).clamp(1, 50) as usize;
            let terms = crate::local_store::search_terms(&query);
            if terms.is_empty() {
                return Ok(json!({ "chats": [], "messages": [] }));
            }
            let (chat_ids, chats) = {
                let state = app.state.lock().unwrap();
                let bots: std::collections::HashMap<&str, &Bot> =
                    state.bots.iter().map(|bot| (bot.id.as_str(), bot)).collect();
                let ids: std::collections::HashSet<String> =
                    state.chats.iter().map(|chat| chat.meta.id.clone()).collect();
                let chats = state
                    .chats
                    .iter()
                    .filter_map(|chat| {
                        let mut parts = chat.meta.title.clone().into_iter().collect::<Vec<_>>();
                        for id in &chat.meta.bot_ids {
                            if let Some(bot) = bots.get(id.as_str()) {
                                parts.extend([bot.name.clone(), bot.description.clone()]);
                            }
                        }
                        let text = parts.join(" ");
                        crate::local_store::search_matches(&text, &terms).then(|| {
                            json!({
                                "chat_id": chat.meta.id,
                                "snippet": crate::local_store::search_snippet(&text, &terms),
                            })
                        })
                    })
                    .take(limit)
                    .collect::<Vec<_>>();
                (ids, chats)
            };
            let messages: Vec<Value> = app
                .store
                .search_messages(&query, limit)
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter(|hit| chat_ids.contains(&hit.chat_id))
                .map(|hit| {
                    json!({
                        "chat_id": hit.chat_id,
                        "message_id": hit.message_id,
                        "snippet": hit.snippet,
                        "author": hit.author,
                        "created_at": hit.created_at,
                    })
                })
                .collect();
            Ok(json!({ "chats": chats, "messages": messages }))
        }
        // Older messages than the snapshot carried, a page at a time, oldest first.
        "chats.messages" => {
            let chat_id = string(&params, "chat_id")?;
            if app.chat(&chat_id).is_none() {
                return Err("No such chat".into());
            }
            let limit = params["limit"].as_u64().unwrap_or(SNAPSHOT_MESSAGES as u64).clamp(1, 200) as usize;
            let before = params["before"].as_str();
            let mut page = app.message_page(&chat_id, before, limit);
            // The end of what is here, with more on the relay: read a page back and look again.
            // A relay that cannot be reached ends the chat here for now.
            if page.0.len() < limit && app.history_is_partial(&chat_id) {
                match crate::sync::older_messages(app, &chat_id).await {
                    Ok(_) => page = app.message_page(&chat_id, before, limit),
                    Err(error) => {
                        tracing::warn!(%error, %chat_id, "fetching older messages");
                        page.1 = false;
                    }
                }
            }
            let (messages, has_more) = page;
            Ok(json!({ "messages": messages, "has_more": has_more }))
        }
        "chats.mark_read" => {
            app.mark_read(&string(&params, "chat_id")?, true);
            Ok(Value::Null)
        }

        // Routines live in the roster; any Device edits them, the bot's Runner runs them. A
        // routine's check is saved on its assigned Runner, including a private template import.
        "routines.create" => {
            let bot_id = string(&params, "bot_id")?;
            let check = params["check"].as_str().filter(|s| !s.trim().is_empty());
            if check.is_some() {
                let bot = app.bot(&bot_id).ok_or("Unknown bot")?;
                if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
                    return Err("Save routine checks on the bot's assigned Runner.".into());
                }
            }
            let triggers = routines::Triggers { pull_request: params["pull_request"].as_str(), calendar: params["calendar"].as_str(), event_match: params["event_match"].as_str() };
            let routine = routines::create_routine(
                app,
                &bot_id,
                &string(&params, "name")?,
                params["schedule"].as_str().unwrap_or(""),
                params["prompt"].as_str().unwrap_or(""),
                check,
                params["enabled"].as_bool().unwrap_or(true),
                params["timezone"].as_str(),
                params["missed_run_policy"].as_str(),
                triggers,
            )?;
            Ok(json!({ "routine": app.routine_out(&routine) }))
        }
        "routines.update" => {
            let id = string(&params, "id")?;
            let mut routine = app.routine(&id).ok_or("Unknown routine")?;
            if ["name", "schedule", "prompt", "timezone", "missed_run_policy", "pull_request", "calendar", "event_match"].iter().any(|field| params.get(field).is_some()) {
                let triggers = routines::Triggers { pull_request: params["pull_request"].as_str(), calendar: params["calendar"].as_str(), event_match: params["event_match"].as_str() };
                routine = routines::edit_routine(app, &id, opt_string(&params, "name").as_deref(), opt_string(&params, "schedule").as_deref(), params["prompt"].as_str(), None, params["timezone"].as_str(), params["missed_run_policy"].as_str(), triggers)?;
            }
            if let Some(enabled) = params["enabled"].as_bool() {
                routine = routines::set_enabled(app, &id, enabled)?;
            }
            Ok(json!({ "routine": app.routine_out(&routine) }))
        }
        "routines.delete" => {
            routines::delete(app, &string(&params, "id")?)?;
            Ok(Value::Null)
        }
        "routines.run" => {
            routines::run_now(app, &string(&params, "id")?)?;
            Ok(Value::Null)
        }
        "routines.describe" => routines::describe(&string(&params, "schedule")?, params["timezone"].as_str(), params["missed_run_policy"].as_str()),
        "device.service_status" => {
            let runner = opt_string(&params, "id").or_else(|| app.this_device_id()).ok_or("No identity on this Device")?;
            if app.this_device_id().as_deref() == Some(&runner) {
                #[cfg(feature = "cli")]
                { crate::service::status_out(&app.config) }
                #[cfg(not(feature = "cli"))]
                { Err("This Device does not run a CLI service.".into()) }
            } else {
                crate::requests::ask(app, &runner, "service.status", json!({})).await
            }
        }

        method if method.starts_with("feedback.") => crate::feedback::on_runner(app, method, params).await,

        // The marketplace: plugins, each with the Runners that have it, and bots to add from a
        // template (`bots.create { template_id }`).
        "marketplace" => {
            let query = opt_string(&params, "query").unwrap_or_default();
            let index = crate::marketplace::index(app).await;
            let installed_on: Vec<(String, Vec<crate::model::PluginStatus>)> =
                app.state.lock().unwrap().devices.iter().map(|d| (d.id.clone(), d.plugins.clone())).collect();
            let plugins: Vec<Value> = crate::marketplace::search_plugins(&index.plugins, &query)
                .into_iter()
                .map(|m| {
                    let mut out = serde_json::to_value(m).unwrap_or_default();
                    // A server from a Runner's mcp.json that happens to share the id is not this plugin.
                    out["installed_on"] = json!(installed_on.iter().filter(|(_, p)| p.iter().any(|s| s.service_id.as_deref().unwrap_or(&s.id) == m.id && s.source.is_none())).map(|(id, _)| id.clone()).collect::<Vec<_>>());
                    out
                })
                .collect();
            let bots: Vec<Value> = crate::marketplace::search_bots(&index.bots, &query)
                .into_iter()
                .map(|t| {
                    let mut out = serde_json::to_value(t).unwrap_or_default();
                    // The apps word a routine's schedule from the CLI's sentence, as a bot's own.
                    for routine in out["routines"].as_array_mut().into_iter().flatten() {
                        let text = routine["schedule"].as_str().and_then(|s| crate::schedule::parse(s).ok()).map(|s| s.describe());
                        routine["schedule_text"] = json!(text);
                    }
                    out
                })
                .collect();
            let packs: Vec<_> = index.packs.iter().filter(|p| {
                query.split_whitespace().all(|word| format!("{} {} {}", p.name, p.outcome, p.description).to_lowercase().contains(&word.to_lowercase()))
            }).collect();
            Ok(json!({ "plugins": plugins, "bots": bots, "packs": packs }))
        }
        // Asks lorca.app for a newer marketplace index now, even within the hour of the last check.
        "marketplace.reload" => {
            let changed = crate::marketplace::check(app, true).await?;
            Ok(json!({ "updated": crate::marketplace::current(app).updated, "changed": changed }))
        }
        // Plugins are installed per Runner, here or through a sealed request to that Runner.
        "plugins.install" => {
            let runner_id = string(&params, "runner_id")?;
            let manifest = match params.get("manifest") {
                Some(value) => crate::plugins::Manifest::parse(value)?,
                None => {
                    let id = string(&params, "plugin_id")?;
                    crate::marketplace::index(app).await.plugin(&id).cloned().ok_or_else(|| format!("No plugin {id} in the marketplace"))?
                }
            };
            let source = if params.get("plugin_id").is_some() { "marketplace" } else { "inline" };
            let body = json!({ "manifest": manifest, "source": source, "account_name": params["account_name"] });
            let status = crate::plugins::on_runner(app, &runner_id, "plugins.install", body).await?;
            Ok(json!({ "status": status }))
        }
        "plugins.uninstall" => {
            let runner_id = string(&params, "runner_id")?;
            crate::plugins::on_runner(app, &runner_id, "plugins.uninstall", json!({ "plugin_id": string(&params, "plugin_id")? })).await
        }
        "plugins.rename" => {
            let runner_id = string(&params, "runner_id")?;
            let body = json!({ "plugin_id": string(&params, "plugin_id")?, "account_name": string(&params, "account_name")? });
            let status = crate::plugins::on_runner(app, &runner_id, "plugins.rename", body).await?;
            Ok(json!({ "status": status }))
        }
        "plugins.set_variables" => {
            let runner_id = string(&params, "runner_id")?;
            let body = json!({ "plugin_id": string(&params, "plugin_id")?, "variables": params["variables"] });
            let status = crate::plugins::on_runner(app, &runner_id, "plugins.variables", body).await?;
            Ok(json!({ "status": status }))
        }
        // On another Runner, the sign-in page opens here (`plugins::sign_in`).
        "plugins.connect" => {
            let runner_id = string(&params, "runner_id")?;
            let plugin_id = string(&params, "plugin_id")?;
            let body = json!({ "plugin_id": plugin_id, "server": opt_string(&params, "server") });
            if app.this_device_id().as_deref() != Some(runner_id.as_str()) {
                let plugin = app.device(&runner_id).and_then(|device| device.plugins.into_iter().find(|p| p.id == plugin_id));
                let name = plugin.map(|p| p.name).unwrap_or_else(|| plugin_id.clone());
                return crate::plugins::sign_in::from_here(app, &runner_id, "plugins.connect", body, &plugin_id, &name).await;
            }
            crate::plugins::on_runner(app, &runner_id, "plugins.connect", body).await
        }
        // The phone's page for sign-in `sign_in` closed before the browser came back.
        "plugins.auth.cancel" => {
            app.cancel_plugin_sign_in(opt_string(&params, "sign_in").as_deref());
            Ok(Value::Null)
        }
        "plugins.detail" => {
            let runner_id = string(&params, "runner_id")?;
            crate::plugins::on_runner(app, &runner_id, "plugins.detail", json!({ "plugin_id": string(&params, "plugin_id")? })).await
        }
        // Forgets a plugin's sign-in on its Runner: every OAuth server's, or `server`'s.
        "plugins.sign_out" => {
            let runner_id = string(&params, "runner_id")?;
            crate::plugins::on_runner(app, &runner_id, "plugins.sign_out", json!({ "plugin_id": string(&params, "plugin_id")?, "server": params["server"] })).await
        }
        // A Runner's mcp.json: its servers, here or through a sealed request to that Runner.
        // `mcp.parse` reads pasted JSON on this Device.
        "mcp.parse" => crate::plugins::mcp_json::parse_reply(params["text"].as_str().unwrap_or_default()),
        "mcp.list" | "mcp.get" | "mcp.save" | "mcp.remove" | "mcp.set_enabled" | "mcp.hide_tool" | "mcp.reconnect" | "mcp.sign_in" | "mcp.sign_out" | "mcp.reload" => {
            let runner_id = opt_string(&params, "runner_id");
            // Boxed: a connection's future is large, and a caller may hold this one on its stack.
            Box::pin(crate::plugins::mcp_json::on_runner(app, runner_id.as_deref(), method, params)).await
        }
        // Auto-review: the check on plugin and shell actions, shared through the roster.
        // `rules` replaces the list; a rule without an id gets one. `provider` picks a
        // connected provider to review with (empty or null for the bot's own). `models` sets a
        // provider's review model by kind, or with null or "" puts back its default; the
        // providers it leaves out keep theirs.
        "auto_review.set" => {
            let mut auto_review = app.auto_review();
            if let Some(enabled) = params["is_enabled"].as_bool() {
                auto_review.is_enabled = enabled;
            }
            if let Some(provider) = params.get("provider") {
                let provider = provider.as_str().map(str::trim).filter(|kind| !kind.is_empty());
                if let Some(kind) = provider {
                    let credentials = app.credentials.lock().unwrap();
                    if !credentials.connected_kinds().iter().any(|connected| connected == kind) {
                        return Err(format!("{} is not connected", credentials.label(kind)));
                    }
                }
                auto_review.provider = provider.map(str::to_string);
            }
            if let Some(models) = params["models"].as_object() {
                for (kind, model) in models {
                    match model.as_str().map(str::trim).filter(|model| !model.is_empty()) {
                        Some(model) => auto_review.models.insert(kind.clone(), model.to_string()),
                        None => auto_review.models.remove(kind),
                    };
                }
            }
            if let Some(rules) = params["rules"].as_array() {
                auto_review.rules = rules
                    .iter()
                    .filter_map(|r| {
                        let text = r["text"].as_str()?.trim().to_string();
                        if text.is_empty() {
                            return None;
                        }
                        let behavior = if r["behavior"].as_str() == Some("ask") { "ask" } else { "allow" };
                        Some(AutoReviewRule {
                            id: r["id"].as_str().filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                            text,
                            behavior: behavior.into(),
                            tool: r["tool"].as_str().map(str::to_string),
                        })
                    })
                    .collect();
            }
            app.set_auto_review(auto_review.clone());
            Ok(json!({ "auto_review": auto_review }))
        }
        // The user answered a permission card: here when the bot runs here, else sealed to
        // its Runner.
        "chats.permission" => {
            let chat_id = string(&params, "chat_id")?;
            let message_id = string(&params, "message_id")?;
            let decision = string(&params, "decision")?;
            let message = app.message(&chat_id, &message_id).ok_or("Unknown message")?;
            let Author::Bot { bot_id } = &message.author else { return Err("Not a permission request".into()) };
            let bot = app.bot(bot_id).ok_or("Unknown bot")?;
            // A secret request's values go along, here or sealed to the Runner, and nowhere else.
            let mut body = json!({ "chat_id": chat_id, "message_id": message_id, "decision": decision });
            if let Some(values) = params.get("values").filter(|values| values.is_object()) {
                body["values"] = values.clone();
            }
            // Sign in on a card for a bot on another Runner: the sign-in page opens here.
            if let Body::Permission { tool, plugin_id, plugin_name, decision: current, .. } = &message.body {
                let signs_in = tool == "connect" && current == "pending" && decision != "deny";
                if signs_in && app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
                    return crate::plugins::sign_in::from_here(app, &bot.runner_id, "permission.answer", body, plugin_id, plugin_name).await;
                }
            }
            crate::plugins::on_runner(app, &bot.runner_id, "permission.answer", body).await
        }

        // A command running in its terminal: the user's answer goes to it from its card, or it
        // stops. Here when the bot runs here, else sealed to its Runner. The text is never kept.
        "bash.stdin" | "bash.stop" | "bash.background" => {
            let chat_id = string(&params, "chat_id")?;
            let message_id = string(&params, "message_id")?;
            let message = app.message(&chat_id, &message_id).ok_or("Unknown message")?;
            let Author::Bot { bot_id } = &message.author else { return Err("That row has no command waiting".into()) };
            let bot = app.bot(bot_id).ok_or("Unknown bot")?;
            let body = json!({ "chat_id": chat_id, "message_id": message_id, "text": params["text"], "enter": params["enter"] });
            if app.this_device_id().as_deref() == Some(bot.runner_id.as_str()) {
                #[cfg(feature = "runner")]
                return crate::shell::serve(app, method, &body).await;
            }
            requests::ask(app, &bot.runner_id, method, body).await
        }

        // A coding agent's card: its transcript, Stop, an answer to what its pane asks, and its
        // pane brought forward on the Runner. Here when the bot runs here, else sealed to its
        // Runner, except showing the pane, which is the Runner's own.
        "coding.transcript" | "coding.stop" | "coding.answer" | "coding.show" => {
            let chat_id = string(&params, "chat_id")?;
            let message_id = string(&params, "message_id")?;
            let message = app.message(&chat_id, &message_id).ok_or("Unknown message")?;
            let Author::Bot { bot_id } = &message.author else { return Err("That row has no coding agent".into()) };
            let bot = app.bot(bot_id).ok_or("Unknown bot")?;
            let body = json!({ "chat_id": chat_id, "message_id": message_id, "choice": params["choice"], "text": params["text"] });
            if app.this_device_id().as_deref() == Some(bot.runner_id.as_str()) {
                #[cfg(feature = "runner")]
                return crate::coding::serve(app, method, &body).await;
            }
            if method == "coding.show" {
                return Err("Its pane is on the Runner".into());
            }
            requests::ask(app, &bot.runner_id, method, body).await
        }
        // `lorca coding hook` in the pane of a coding agent this CLI runs asks whether its next
        // command may run.
        #[cfg(feature = "runner")]
        "coding.review" => {
            let id = string(&params, "agent_id")?;
            let command = string(&params, "command")?;
            crate::coding::review_for_hook(app, &id, &command).await
        }

        // Read one API key on demand for the local settings editor. Snapshots and events
        // continue to carry masked provider statuses.
        "providers.api_key" => {
            let kind = string(&params, "kind")?;
            let credentials = app.credentials.lock().unwrap();
            if crate::credentials::is_custom(&kind) {
                let provider = credentials.custom.get(&kind).ok_or("Unknown provider")?;
                return Ok(json!({ "api_key": provider.api_key, "base_url": provider.base_url }));
            }
            if !matches!(kind.as_str(), "deepseek" | "anthropic" | "opencode" | "opencode-go") {
                return Err("Not an API-key provider".into());
            }
            let credential = credentials.api_key(&kind);
            Ok(json!({
                "api_key": credential.map(|c| &c.api_key),
                "base_url": credential.and_then(|c| c.base_url.as_ref()),
            }))
        }
        #[cfg(feature = "provider-auth")]
        "providers.connect_deepseek" => {
            provider_auth::connect_deepseek(app, &string(&params, "api_key")?, opt_string(&params, "base_url").as_deref()).await?;
            Ok(json!({ "providers": app.credentials.lock().unwrap().statuses() }))
        }
        #[cfg(feature = "provider-auth")]
        "providers.connect_anthropic" => {
            provider_auth::connect_anthropic(app, &string(&params, "api_key")?, opt_string(&params, "base_url").as_deref()).await?;
            Ok(json!({ "providers": app.credentials.lock().unwrap().statuses() }))
        }
        #[cfg(feature = "provider-auth")]
        "providers.connect_opencode" => {
            provider_auth::connect_opencode(app, &string(&params, "api_key")?, opt_string(&params, "base_url").as_deref()).await?;
            Ok(json!({ "providers": app.credentials.lock().unwrap().statuses() }))
        }
        #[cfg(feature = "provider-auth")]
        "providers.connect_opencode_go" => {
            provider_auth::connect_opencode_go(app, &string(&params, "api_key")?, opt_string(&params, "base_url").as_deref()).await?;
            Ok(json!({ "providers": app.credentials.lock().unwrap().statuses() }))
        }
        // The chat models a custom provider's server lists, for the model picker.
        #[cfg(feature = "provider-auth")]
        "providers.list_models" => {
            let str_param = |key: &str| params[key].as_str().unwrap_or_default().to_string();
            let listed = provider_auth::list_custom_models(app, &str_param("name"), &str_param("api"), &str_param("base_url"), &str_param("api_key")).await?;
            Ok(json!({ "listed": listed.is_some(), "models": listed.unwrap_or_default() }))
        }
        // Adds a custom provider, or saves one with `kind`, once its server answers.
        #[cfg(feature = "provider-auth")]
        "providers.connect_custom" => {
            let input = provider_auth::CustomInput {
                kind: opt_string(&params, "kind"),
                name: params["name"].as_str().unwrap_or_default().to_string(),
                api: opt_string(&params, "api").unwrap_or_else(|| "chat-completions".into()),
                base_url: params["base_url"].as_str().unwrap_or_default().to_string(),
                api_key: params["api_key"].as_str().unwrap_or_default().to_string(),
                models: params["models"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect(),
            };
            let kind = provider_auth::connect_custom(app, input).await?;
            Ok(json!({ "kind": kind, "providers": app.credentials.lock().unwrap().statuses() }))
        }
        #[cfg(feature = "provider-auth")]
        "providers.connect_chatgpt" => {
            let cancel = app.begin_provider_auth();
            let opener = app.clone();
            let login = provider_auth::connect_chatgpt(app, move |url| open_provider_auth(&opener, "chatgpt", url));
            let tokens = tokio::select! {
                result = login => result?,
                _ = cancel.cancelled() => return Err("Sign-in cancelled".into()),
            };
            Ok(json!({ "email": tokens.email, "providers": app.credentials.lock().unwrap().statuses() }))
        }
        #[cfg(feature = "provider-auth")]
        "providers.connect_grok" => {
            let cancel = app.begin_provider_auth();
            let opener = app.clone();
            let login = provider_auth::connect_grok(app, move |url| open_provider_auth(&opener, "grok", url));
            let tokens = tokio::select! {
                result = login => result?,
                _ = cancel.cancelled() => return Err("Sign-in cancelled".into()),
            };
            Ok(json!({ "email": tokens.email, "providers": app.credentials.lock().unwrap().statuses() }))
        }
        #[cfg(feature = "provider-auth")]
        "providers.auth.cancel" => {
            app.cancel_provider_auth();
            Ok(Value::Null)
        }
        #[cfg(feature = "provider-auth")]
        "providers.disconnect" => {
            app.cancel_provider_auth();
            provider_auth::disconnect(app, &string(&params, "kind")?)?;
            Ok(json!({ "providers": app.credentials.lock().unwrap().statuses() }))
        }
        // Asks lorca.app for a newer model catalog now, even within the hour of the last check.
        "models.reload" => {
            let changed = crate::catalog::check(app, true).await?;
            Ok(json!({ "updated": lorca_models::updated(), "changed": changed }))
        }

        other => Err(format!("unknown method {other}")),
    }
}
