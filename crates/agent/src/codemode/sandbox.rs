//! One script in a QuickJS VM of its own, on a thread of its own, after pi's codemode worker.
//!
//! The VM's only way out is the bridge the prelude holds: it posts calls and output to the
//! host as [`Event`]s, and the host answers calls with [`Worker::reply`]. QuickJS runs
//! synchronously, so the thread keeps a spinning script off the async runtime; stopping sets
//! an interrupt flag the VM polls and drops the reply channel, which ends the thread whether
//! the script spins or waits.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc as std_mpsc, Arc};

use rquickjs::context::EvalOptions;
use rquickjs::function::Opt;
use rquickjs::{CatchResultExt, Context, Ctx, Function, Object, Persistent, Runtime};
use serde_json::json;
use tokio::sync::mpsc;

const PRELUDE: &str = include_str!("prelude.js");
/// The native stack the VM may use before deep recursion throws a `RangeError`; the thread
/// gets twice that.
const MAX_STACK_BYTES: usize = 8 * 1024 * 1024;
/// The most one `store()` value and all values together may take, in characters of JSON.
pub const MAX_STORE_VALUE_CHARS: usize = 256 * 1024;
pub const MAX_STORE_TOTAL_CHARS: usize = 1024 * 1024;
/// Events the script may have sent that the host has not taken yet: past this, the script's
/// thread waits, so a script that writes faster than the host reads never grows the host.
const EVENT_BUFFER: usize = 64;

/// A tool as the script sees it: `tools[js_name]` and `tools[name]`, listed in `ALL_TOOLS`.
pub(crate) struct ScriptTool {
    pub name: String,
    pub js_name: String,
    pub description: String,
}

/// A host function called as `name(...)`, or `ns.member(...)` for a dotted name. `spread`
/// hands the host every argument as an array instead of the first one.
pub(crate) struct ScriptGlobal {
    pub name: String,
    pub spread: bool,
}

pub(crate) struct Script {
    pub code: String,
    pub tools: Vec<ScriptTool>,
    pub globals: Vec<ScriptGlobal>,
    /// Key to JSON text, for `load()`.
    pub store: BTreeMap<String, String>,
    pub memory_limit: usize,
}

/// What the script sends the host.
#[derive(Debug)]
pub(crate) enum Event {
    /// A tool call (`global` false) or a host global, with its arguments as JSON.
    Call { id: u64, global: bool, name: String, args: Option<String> },
    Text(String),
    Image { data: String, mime_type: String },
    /// The script returned: its value as JSON, and the keys it stored as a JSON array of
    /// `[key, json]` and `[key]` for a deletion.
    Done { value: Option<String>, writes: String },
    /// The script threw or failed to compile: `{ name?, message, stack? }` as JSON.
    Failed { error: String },
    /// The VM failed outside the script's control.
    Crash(String),
}

struct Reply {
    id: u64,
    ok: bool,
    payload: Option<String>,
}

/// A running script. Dropping it stops the script.
pub(crate) struct Worker {
    events: mpsc::Receiver<Event>,
    replies: Option<std_mpsc::Sender<Reply>>,
    interrupt: Arc<AtomicBool>,
}

impl Worker {
    pub fn start(script: Script) -> Result<Worker, String> {
        let (event_tx, events) = mpsc::channel(EVENT_BUFFER);
        let (replies, reply_rx) = std_mpsc::channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        let flag = interrupt.clone();
        std::thread::Builder::new()
            .name("codemode".into())
            .stack_size(MAX_STACK_BYTES * 2)
            .spawn(move || {
                let crashed = event_tx.clone();
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drive(script, event_tx, reply_rx, &flag)));
                match outcome {
                    Ok(Ok(())) => {}
                    Ok(Err(message)) => {
                        let _ = crashed.blocking_send(Event::Crash(message));
                    }
                    Err(_) => {
                        let _ = crashed.blocking_send(Event::Crash("the script's VM panicked".into()));
                    }
                }
            })
            .map_err(|error| format!("Cannot start the script's thread: {error}"))?;
        Ok(Worker { events, replies: Some(replies), interrupt })
    }

    /// The script's next event; `None` once its thread is gone.
    pub async fn next(&mut self) -> Option<Event> {
        self.events.recv().await
    }

    /// Answers call `id`: its result as JSON, or the error message the promise rejects with.
    pub fn reply(&self, id: u64, ok: bool, payload: Option<String>) {
        if let Some(replies) = &self.replies {
            let _ = replies.send(Reply { id, ok, payload });
        }
    }

    /// Ends the script where it stands: a spinning VM throws at its next interrupt check, one
    /// waiting on a call wakes to a closed channel, and one waiting to send wakes when the
    /// events close.
    pub fn stop(&mut self) {
        self.interrupt.store(true, Ordering::Relaxed);
        self.replies = None;
        self.events.close();
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn eval_options(filename: &str) -> EvalOptions {
    let mut options = EvalOptions::default();
    options.strict = false;
    options.filename = Some(filename.into());
    options
}

/// The thread's work: sets up the VM, starts the script, then runs its jobs and delivers the
/// host's answers until it settles or the host stops it.
fn drive(script: Script, events: mpsc::Sender<Event>, replies: std_mpsc::Receiver<Reply>, interrupt: &Arc<AtomicBool>) -> Result<(), String> {
    let runtime = Runtime::new().map_err(|error| format!("Cannot start QuickJS: {error}"))?;
    runtime.set_memory_limit(script.memory_limit);
    runtime.set_max_stack_size(MAX_STACK_BYTES);
    let flag = interrupt.clone();
    runtime.set_interrupt_handler(Some(Box::new(move || flag.load(Ordering::Relaxed))));
    let context = Context::full(&runtime).map_err(|error| format!("Cannot start QuickJS: {error}"))?;
    let finished = Arc::new(AtomicBool::new(false));
    let stopped = || interrupt.load(Ordering::Relaxed);

    let api = context.with(|ctx| setup(&ctx, &script, events.clone(), finished.clone()));
    let api = match api {
        Ok(Some(api)) => api,
        Ok(None) => return Ok(()),
        Err(_) if stopped() => return Ok(()),
        Err(error) => return Err(error),
    };

    let result = (|| {
        loop {
            loop {
                match runtime.execute_pending_job() {
                    Ok(true) => continue,
                    Ok(false) => break,
                    Err(_) if stopped() => return Ok(()),
                    Err(error) => return Err(format!("a job of the script's VM failed: {error:?}")),
                }
            }
            if stopped() || finished.load(Ordering::Relaxed) {
                return Ok(());
            }
            let state = context.with(|ctx| -> Result<String, String> {
                let api = api.clone().restore(&ctx).map_err(|error| error.to_string())?;
                let stalled: Function = api.get("stalled").map_err(|error| error.to_string())?;
                stalled.call::<_, String>(()).catch(&ctx).map_err(|error| error.to_string())
            });
            match state.as_deref() {
                Err(_) if stopped() => return Ok(()),
                Err(error) => return Err(error.clone()),
                Ok(_) if finished.load(Ordering::Relaxed) => return Ok(()),
                Ok("waiting") => {}
                // The prelude ended the script, but its result never crossed the bridge.
                Ok(_) => return Err("the script ended without reporting its result".into()),
            }
            let Ok(reply) = replies.recv() else { return Ok(()) };
            let settled = context.with(|ctx| -> Result<(), String> {
                let api = api.clone().restore(&ctx).map_err(|error| error.to_string())?;
                let settle: Function = api.get("settle").map_err(|error| error.to_string())?;
                let has_payload = reply.payload.is_some();
                settle
                    .call::<_, ()>((reply.id as f64, reply.ok, has_payload, reply.payload.unwrap_or_default()))
                    .catch(&ctx)
                    .map_err(|error| error.to_string())
            });
            match settled {
                Err(_) if stopped() => return Ok(()),
                Err(error) => return Err(error),
                Ok(()) => {}
            }
        }
    })();
    // The handle must go before the context and runtime it points into.
    drop(api);
    result
}

/// Evaluates the prelude with the bridge and starts the script. `None` when the script did not
/// compile, which it reports itself.
fn setup(ctx: &Ctx<'_>, script: &Script, events: mpsc::Sender<Event>, finished: Arc<AtomicBool>) -> Result<Option<Persistent<Object<'static>>>, String> {
    let failures = events.clone();
    let failed = finished.clone();
    let bridge = Function::new(ctx.clone(), move |kind: String, id: f64, text: String, extra: Opt<String>| {
        let event = match kind.as_str() {
            "call" | "global" => Event::Call { id: id as u64, global: kind == "global", name: text, args: extra.0 },
            "text" => Event::Text(text),
            "image" => Event::Image { data: text, mime_type: extra.0.unwrap_or_else(|| "application/octet-stream".into()) },
            "done" => {
                finished.store(true, Ordering::Relaxed);
                Event::Done { value: extra.0, writes: text }
            }
            "failed" => {
                finished.store(true, Ordering::Relaxed);
                Event::Failed { error: text }
            }
            _ => return,
        };
        // The script's own thread: waiting here is what keeps a flood of output bounded.
        let _ = events.blocking_send(event);
    })
    .map_err(|error| error.to_string())?;

    let tools = json!(script.tools.iter().map(|tool| json!({ "name": tool.name, "jsName": tool.js_name, "description": tool.description })).collect::<Vec<_>>());
    let globals = json!(script.globals.iter().map(|global| json!({ "name": global.name, "spread": global.spread })).collect::<Vec<_>>());
    let limits = json!({ "storeValueChars": MAX_STORE_VALUE_CHARS, "storeTotalChars": MAX_STORE_TOTAL_CHARS });
    let prelude: Function = ctx.eval_with_options(PRELUDE, eval_options("codemode-prelude.js")).catch(ctx).map_err(|error| format!("the codemode prelude failed: {error}"))?;
    let api: Object = prelude
        .call((bridge, tools.to_string(), globals.to_string(), serde_json::to_string(&script.store).unwrap_or_else(|_| "{}".into()), limits.to_string()))
        .catch(ctx)
        .map_err(|error| format!("the codemode prelude failed: {error}"))?;

    // The wrapper shares the first line with the script, so line numbers in its errors match
    // the script as written.
    let wrapped = format!("(async (tools, console) => {{{}\n}})", script.code);
    let function: Function = match ctx.eval_with_options(wrapped, eval_options("codemode.js")).catch(ctx) {
        Ok(function) => function,
        Err(error) => {
            let (name, message, stack) = match &error {
                rquickjs::CaughtError::Exception(exception) => {
                    let message = exception.message().unwrap_or_default();
                    let name: String = exception.get("name").unwrap_or_else(|_| "SyntaxError".into());
                    let stack = exception.stack().map(|stack| format!("{name}: {message}\n{}", stack.trim_end()));
                    (name, message, stack)
                }
                other => ("Error".to_string(), other.to_string(), None),
            };
            let error = json!({ "name": name, "message": message, "stack": stack.unwrap_or_else(|| format!("{name}: {message}")) });
            failed.store(true, Ordering::Relaxed);
            let _ = failures.blocking_send(Event::Failed { error: error.to_string() });
            return Ok(None);
        }
    };
    let run: Function = api.get("run").map_err(|error| error.to_string())?;
    run.call::<_, ()>((function,)).catch(ctx).map_err(|error| error.to_string())?;
    Ok(Some(Persistent::save(ctx, api)))
}
