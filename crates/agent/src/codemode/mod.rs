//! Codemode, after pi's: the model writes JavaScript that calls tools, and only what the script
//! outputs or returns reaches the model. The results of the calls it makes stay out of context,
//! so a script can page through an API, run calls in parallel, and filter what came back down
//! to the few lines that matter.
//!
//! - [`CodemodeTool`] is the `codemode` tool. Its description lists what scripts can call as
//!   TypeScript declarations, within a token budget.
//! - A [`Catalog`] says what scripts can reach. [`StaticCatalog`] is a fixed list; a host whose
//!   tools come and go (servers that connect on first use) implements its own.
//! - Each script runs in a fresh QuickJS VM on a thread of its own, whose only capability is
//!   calling the catalog's tools. Every call goes through the loop's tool pipeline
//!   ([`ToolRunner`]): argument checks, `before_tool_call`, `after_tool_call`. A call a
//!   `before_tool_call` hook blocks ends the script, so a refusal never lets the rest of a batch
//!   run.
//! - `store()` and `load()` keep JSON values across scripts through a [`CodemodeStore`] the host
//!   persists; the crate keeps nothing.

mod declarations;
mod sandbox;
mod search;
mod source;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

pub use declarations::{doc_comment, mcp_structured_content_schema, schema_to_type, to_identifier, tool_sample, tool_signature, MCP_TYPESCRIPT_PREAMBLE};
pub use sandbox::{MAX_STORE_TOTAL_CHARS, MAX_STORE_VALUE_CHARS};
pub use search::DEFAULT_SEARCH_LIMIT;
pub use source::{parse_source, ParsedSource, SourceOptions, OPTIONS_PREFIX};

use crate::tool::{DirectRunner, Tool, ToolError, ToolOutcome, ToolResult, ToolRunner, ToolUpdateFn};
use crate::types::ContentPart;
use sandbox::{Event, Script, ScriptGlobal, ScriptTool, Worker};

pub const CODEMODE_TOOL_NAME: &str = "codemode";

/// How the description presents a tool that scripts can call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exposure {
    /// Also declared to the model as a tool of its own: the description only names it.
    Direct,
    /// Listed with its declaration, while the listing fits its budget.
    Listed,
    /// Callable, never listed: `searchTools()` finds it.
    Deferred,
}

/// A group of related tools, such as one plugin's. Its tools are listed together under its
/// name, and a group whose tools are not known yet is still named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Namespace {
    pub name: String,
    /// Shown under the group's heading.
    pub description: String,
}

/// What `describeNamespace()` gives a script, after pi's: a group's description, its whole
/// instructions, and its tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceDetails {
    pub name: String,
    pub description: String,
    /// Guidance for the group's tools, such as its servers' instructions, whole: the listing
    /// may carry only their start. Empty when there is none.
    pub instructions: String,
    /// Its tools' names.
    pub tools: Vec<String>,
}

/// One tool scripts can call.
#[derive(Clone)]
pub struct Entry {
    pub tool: Arc<dyn Tool>,
    pub exposure: Exposure,
    /// The [`Namespace`] it belongs to, by name.
    pub namespace: Option<String>,
}

impl Entry {
    pub fn new(tool: Arc<dyn Tool>, exposure: Exposure) -> Self {
        Entry { tool, exposure, namespace: None }
    }

    pub fn in_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }

    /// Its description and declaration, as `searchTools()` and `describeTool()` give it.
    pub fn sample(&self) -> String {
        let tool = &self.tool;
        tool_sample(tool.name(), tool.description(), Some(&tool.parameters()), Some(&output_schema_of(tool.as_ref())))
    }
}

/// What a script receives from a tool: its structured output when it declares a schema for it,
/// its text otherwise.
fn output_schema_of(tool: &dyn Tool) -> Value {
    tool.output_schema().unwrap_or_else(|| json!({ "type": "string" }))
}

/// What scripts can reach.
#[async_trait]
pub trait Catalog: Send + Sync {
    /// The tools scripts can call now. The description and `ALL_TOOLS` list these.
    fn entries(&self) -> Vec<Entry>;

    /// Every group, including ones with no tools known yet.
    fn namespaces(&self) -> Vec<Namespace> {
        Vec::new()
    }

    /// A tool `entries` did not have, by its exact name or identifier, such as one of a server
    /// that connects on first use.
    async fn find(&self, name: &str, cancel: &CancellationToken) -> Option<Entry> {
        let _ = (name, cancel);
        None
    }

    /// `searchTools()`: the best matches for `query`, at most `limit`, in `namespace` when
    /// given. The default ranks `entries` with BM25. The error rejects the script's call.
    async fn search(&self, query: &str, namespace: Option<&str>, limit: usize, cancel: &CancellationToken) -> Result<Vec<Entry>, String> {
        let _ = cancel;
        let namespaces = self.namespaces();
        let entries: Vec<Entry> = self.entries().into_iter().filter(|entry| namespace.is_none() || entry.namespace.as_deref() == namespace).collect();
        Ok(rank_entries(query, entries, &namespaces, limit))
    }

    /// `describeNamespace()`: the group named `name`, or by its identifier, with its tools; `None`
    /// for no such group. The default reads `namespaces` and `entries`.
    async fn describe_namespace(&self, name: &str, cancel: &CancellationToken) -> Option<NamespaceDetails> {
        let _ = cancel;
        let entries = self.entries();
        let named = |candidate: &str| candidate == name || to_identifier(candidate) == name;
        let namespace = self.namespaces().into_iter().find(|namespace| named(&namespace.name)).or_else(|| {
            let found = entries.iter().filter_map(|entry| entry.namespace.as_deref()).find(|namespace| named(namespace))?;
            Some(Namespace { name: found.to_string(), description: String::new() })
        })?;
        let tools = entries.iter().filter(|entry| entry.namespace.as_deref() == Some(namespace.name.as_str())).map(|entry| entry.tool.name().to_string()).collect();
        Some(NamespaceDetails { name: namespace.name, description: namespace.description, instructions: String::new(), tools })
    }
}

/// BM25 over entries, for [`Catalog::search`].
pub fn rank_entries(query: &str, entries: Vec<Entry>, namespaces: &[Namespace], limit: usize) -> Vec<Entry> {
    let documents: Vec<String> = entries
        .iter()
        .map(|entry| {
            let namespace = entry
                .namespace
                .as_deref()
                .map(|name| (name, namespaces.iter().find(|namespace| namespace.name == name).map(|namespace| namespace.description.as_str()).unwrap_or("")));
            search::document(entry.tool.name(), entry.tool.description(), &entry.tool.parameters(), namespace)
        })
        .collect();
    search::rank(query, &documents, limit).into_iter().map(|index| entries[index].clone()).collect()
}

/// A fixed set of tools.
pub struct StaticCatalog {
    entries: Vec<Entry>,
    namespaces: Vec<Namespace>,
}

impl StaticCatalog {
    /// Every tool listed, in no namespace.
    pub fn new(tools: Vec<Arc<dyn Tool>>) -> Self {
        StaticCatalog { entries: tools.into_iter().map(|tool| Entry::new(tool, Exposure::Listed)).collect(), namespaces: Vec::new() }
    }

    pub fn with_entries(entries: Vec<Entry>, namespaces: Vec<Namespace>) -> Self {
        StaticCatalog { entries, namespaces }
    }
}

#[async_trait]
impl Catalog for StaticCatalog {
    fn entries(&self) -> Vec<Entry> {
        self.entries.clone()
    }

    fn namespaces(&self) -> Vec<Namespace> {
        self.namespaces.clone()
    }
}

/// A function a host gives scripts besides tools, such as `models.ask`: called by its name,
/// with `ns.member` grouping it under a namespace object, and every argument handed over as an
/// array. The script's call list records its calls.
#[async_trait]
pub trait HostFunction: Send + Sync {
    /// An identifier, or `ns.member`.
    fn name(&self) -> &str;
    /// Shown as its doc comment.
    fn description(&self) -> &str;
    /// Its TypeScript parameters and return type, such as
    /// `(prompt: string, options?: { system?: string }): Promise<string>`.
    fn signature(&self) -> &str;
    /// The error rejects the script's call.
    async fn call(&self, args: Vec<Value>, cancel: &CancellationToken) -> Result<Value, String>;
}

/// The keys one successful script stored (`set`) and deleted.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct StoreWrites {
    pub set: BTreeMap<String, Value>,
    pub delete: Vec<String>,
}

impl StoreWrites {
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.delete.is_empty()
    }
}

/// Where `store()` values live between scripts. The host keeps them; a failed script writes
/// nothing.
pub trait CodemodeStore: Send + Sync {
    fn load(&self) -> BTreeMap<String, Value>;
    fn save(&self, writes: &StoreWrites);
}

#[derive(Debug, Clone)]
pub struct CodemodeOptions {
    /// Estimated tokens (characters / 4) the listed declarations may take in the description.
    /// Tools that do not fit are left to `searchTools()`.
    pub inline_budget: usize,
    /// The token budget for a script's output, unless its options line sets one.
    pub max_output_tokens: usize,
    /// The most memory a script's VM may allocate.
    pub memory_limit: usize,
    /// How many of a script's calls may run at once; the rest wait their turn. A tool whose
    /// execution mode is sequential runs alone either way.
    pub max_concurrent_calls: usize,
    /// The longest a script may run, its calls and any question they put to a person included.
    /// Its options line may ask for less.
    pub timeout: Duration,
    /// Declare the shared MCP result types even before any MCP tool is known, for a host whose
    /// scripts reach MCP servers that connect on first use.
    pub mcp_types: bool,
    /// Host rules appended to the description.
    pub guidance: Option<String>,
}

impl Default for CodemodeOptions {
    fn default() -> Self {
        CodemodeOptions {
            inline_budget: 3000,
            max_output_tokens: 10_000,
            memory_limit: 256 * 1024 * 1024,
            max_concurrent_calls: 8,
            timeout: Duration::from_secs(30 * 60),
            mcp_types: false,
            guidance: None,
        }
    }
}

/// What the script's calls look like in the result's details and progress updates.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NestedCall {
    /// The call's own id, `<codemode call id>/<n>`.
    pub id: String,
    pub name: String,
    /// Compact JSON of the arguments, cut for display.
    pub args: String,
    /// What the call says it does, from a `description` string among its arguments (a
    /// command's "Run the tests"), cut, for a host's status line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: CallStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// The error text, cut.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallStatus {
    Running,
    Ok,
    Error,
    /// A `before_tool_call` hook refused it, which ended the script.
    Blocked,
    /// Still running when the script ended.
    Cancelled,
}

/// The most calls one result lists; the rest are only counted.
const MAX_RECORDED_CALLS: usize = 256;
const ARGS_PREVIEW_CHARS: usize = 200;
const ERROR_PREVIEW_CHARS: usize = 500;
const CHARS_PER_TOKEN: usize = 4;
/// How long calls still running when a script ends get to wind down after their cancel.
const WIND_DOWN: Duration = Duration::from_secs(10);
/// What one script may send the host, whatever its memory limit: the text it outputs, how many
/// images, the calls it has started and not seen finish, and one call's arguments. Past the
/// first or the third, the script stops. Each image is made one a model takes as it ends
/// (`images::prepare`).
const MAX_OUTPUT_CHARS: usize = 16 * 1024 * 1024;
const MAX_IMAGES: usize = 10;
const MAX_PENDING_CALLS: usize = 1000;
const MAX_ARGUMENT_CHARS: usize = 8 * 1024 * 1024;
/// The largest output budget an options line may ask for: the result is a chat row, which must
/// stay small enough to sync.
const MAX_OUTPUT_TOKENS: usize = 50_000;
/// The most of a script's error the result carries.
const MAX_ERROR_CHARS: usize = 20_000;

/// The `codemode` tool.
pub struct CodemodeTool {
    catalog: Arc<dyn Catalog>,
    store: Option<Arc<dyn CodemodeStore>>,
    functions: Vec<Arc<dyn HostFunction>>,
    options: CodemodeOptions,
    description: String,
}

impl CodemodeTool {
    /// A codemode tool over `catalog`. The description is rendered now, from what the catalog
    /// has, and stays as it is while the tool lives, so it does not break a prompt cache.
    pub fn new(catalog: Arc<dyn Catalog>, options: CodemodeOptions) -> Self {
        let description = describe(&catalog.entries(), &catalog.namespaces(), &[], &options);
        CodemodeTool { catalog, store: None, functions: Vec::new(), options, description }
    }

    pub fn with_store(mut self, store: Arc<dyn CodemodeStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Host functions scripts can call, declared in the description. A name that is taken by
    /// a built-in helper, or not an identifier, is left out.
    pub fn with_functions(mut self, functions: Vec<Arc<dyn HostFunction>>) -> Self {
        self.functions = functions.into_iter().filter(|function| valid_function_name(function.name())).collect();
        self.description = describe(&self.catalog.entries(), &self.catalog.namespaces(), &self.functions, &self.options);
        self
    }
}

/// The helpers every script has, which a host function may not replace.
const BUILT_INS: [&str; 12] = ["tools", "ALL_TOOLS", "console", "text", "image", "exit", "store", "load", "searchTools", "describeTool", "describeNamespace", "globalThis"];

fn valid_function_name(name: &str) -> bool {
    let parts: Vec<&str> = name.split('.').collect();
    parts.len() <= 2
        && parts.iter().all(|part| to_identifier(part) == *part && !part.is_empty())
        && !BUILT_INS.contains(&parts[0])
}

#[async_trait]
impl Tool for CodemodeTool {
    fn name(&self) -> &str {
        CODEMODE_TOOL_NAME
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "Raw JavaScript source."
                }
            },
            "required": ["code"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, tool_call_id: &str, args: Value, cancel: CancellationToken, on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        self.execute_with(tool_call_id, args, cancel, on_update, &DirectRunner).await
    }

    async fn execute_with(&self, tool_call_id: &str, args: Value, cancel: CancellationToken, on_update: ToolUpdateFn, tools: &dyn ToolRunner) -> Result<ToolResult, ToolError> {
        let parsed = parse_source(args["code"].as_str().unwrap_or("")).map_err(ToolError)?;
        Ok(Run::new(self, tool_call_id, &cancel, on_update, tools).execute(parsed).await.result)
    }
}

/// A script a host ran itself, outside a model's turn.
#[derive(Debug, Clone)]
pub struct ScriptRun {
    /// What a model would read had it run the script: the output, the returned value, or the
    /// error, and the calls in `details`.
    pub result: ToolResult,
    /// What the script returned, when it finished and returned anything but `undefined`.
    pub returned: Option<Value>,
}

impl CodemodeTool {
    /// Runs `code` as a call of this tool would, with `tools` running the script's calls, and
    /// keeps what it returned apart from what it printed.
    pub async fn run_script(&self, call_id: &str, code: &str, cancel: CancellationToken, tools: &dyn ToolRunner) -> Result<ScriptRun, ToolError> {
        let parsed = parse_source(code).map_err(ToolError)?;
        Ok(Run::new(self, call_id, &cancel, Arc::new(|_| {}), tools).execute(parsed).await)
    }
}

/// What a script call comes back as.
enum Reply {
    /// Resolves with this JSON, or `undefined`.
    Value(Option<String>),
    /// Rejects with an `Error` carrying this message.
    Throw(String),
    /// Ends the script: a hook refused the call.
    Stop { reason: String, terminate: bool },
}

/// How a script ended.
enum End {
    Done { value: Option<String>, writes: String },
    Failed(String),
    Stopped { reason: String, terminate: bool },
    Timeout(u64),
    Aborted,
    /// It went past what a script may send the host.
    Limit(String),
    Crash(String),
}

/// One script's run.
struct Run<'a> {
    tool: &'a CodemodeTool,
    call_id: &'a str,
    cancel: &'a CancellationToken,
    on_update: ToolUpdateFn,
    tools: &'a dyn ToolRunner,
    /// Callable tools by name and by identifier; grows as searches and lookups find more.
    known: Mutex<HashMap<String, Entry>>,
    calls: Mutex<Vec<NestedCall>>,
    dropped_calls: Mutex<usize>,
    limiter: tokio::sync::Semaphore,
    exclusive: tokio::sync::Mutex<()>,
    /// Cancels the script's calls when it ends, is stopped, or its caller is.
    calls_cancel: CancellationToken,
}

impl<'a> Run<'a> {
    fn new(tool: &'a CodemodeTool, call_id: &'a str, cancel: &'a CancellationToken, on_update: ToolUpdateFn, tools: &'a dyn ToolRunner) -> Self {
        Run {
            tool,
            call_id,
            cancel,
            on_update,
            tools,
            known: Mutex::new(HashMap::new()),
            calls: Mutex::new(Vec::new()),
            dropped_calls: Mutex::new(0),
            limiter: tokio::sync::Semaphore::new(tool.options.max_concurrent_calls.max(1)),
            exclusive: tokio::sync::Mutex::new(()),
            calls_cancel: cancel.child_token(),
        }
    }

    fn remember(&self, entry: &Entry) {
        let mut known = self.known.lock().unwrap();
        let name = entry.tool.name().to_string();
        let identifier = to_identifier(&name);
        known.entry(identifier).or_insert_with(|| entry.clone());
        known.entry(name).or_insert_with(|| entry.clone());
    }

    fn lookup(&self, name: &str) -> Option<Entry> {
        self.known.lock().unwrap().get(name).cloned()
    }

    /// Why a call names no tool, with the known tools whose names come closest, after pi:
    /// `tools.Bash` suggests `tools.bash`, and `tools.github_create_issue`
    /// `tools.github__create_issue`. A short catalog is listed whole instead.
    fn unknown_tool(&self, name: &str) -> String {
        let mut names: Vec<String> = self.known.lock().unwrap().values().map(|entry| to_identifier(entry.tool.name())).collect();
        names.sort();
        names.dedup();
        let comparable = |name: &str| name.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
        let wanted = comparable(name);
        let mut close: Vec<&String> = names.iter().filter(|known| comparable(known) == wanted).collect();
        if close.is_empty() && !wanted.is_empty() {
            close = names
                .iter()
                .filter(|known| {
                    let known = comparable(known);
                    !known.is_empty() && (known.contains(&wanted) || wanted.contains(&known))
                })
                .collect();
        }
        let mut message = format!("Unknown tool \"{name}\".");
        if !close.is_empty() {
            message.push_str(&format!(" Did you mean {}?", close.iter().take(5).map(|known| format!("tools.{known}")).collect::<Vec<_>>().join(", ")));
        } else if !names.is_empty() && names.len() <= 20 {
            message.push_str(&format!(" The tools are {}.", names.join(", ")));
        }
        message.push_str(" ALL_TOOLS lists every tool, and searchTools(query) finds one by what it does.");
        message
    }

    async fn resolve(&self, name: &str) -> Option<Entry> {
        if let Some(entry) = self.lookup(name) {
            return Some(entry);
        }
        let entry = self.tool.catalog.find(name, &self.calls_cancel).await.filter(|entry| entry.tool.name() != CODEMODE_TOOL_NAME)?;
        self.remember(&entry);
        Some(entry)
    }

    async fn execute(self, parsed: ParsedSource) -> ScriptRun {
        let started = Instant::now();
        let entries: Vec<Entry> = self.tool.catalog.entries().into_iter().filter(|entry| entry.tool.name() != CODEMODE_TOOL_NAME).collect();
        let script_tools = entries
            .iter()
            .map(|entry| {
                self.remember(entry);
                ScriptTool { name: entry.tool.name().to_string(), js_name: to_identifier(entry.tool.name()), description: entry.sample() }
            })
            .collect();
        let store: BTreeMap<String, String> = self
            .tool
            .store
            .as_ref()
            .map(|store| store.load().into_iter().filter_map(|(key, value)| serde_json::to_string(&value).ok().map(|json| (key, json))).collect())
            .unwrap_or_default();
        let script = Script {
            code: parsed.code,
            tools: script_tools,
            globals: ["searchTools", "describeTool", "describeNamespace"]
                .into_iter()
                .map(str::to_string)
                .chain(self.tool.functions.iter().map(|function| function.name().to_string()))
                .map(|name| ScriptGlobal { name, spread: true })
                .collect(),
            store: store.clone(),
            memory_limit: self.tool.options.memory_limit,
        };
        let timeout = parsed.options.timeout_ms.map(Duration::from_millis).map_or(self.tool.options.timeout, |asked| asked.min(self.tool.options.timeout));

        let mut output: Vec<ContentPart> = Vec::new();
        let end = match Worker::start(script) {
            Ok(worker) => self.drive(worker, timeout, &mut output).await,
            Err(error) => End::Crash(error),
        };
        for call in self.calls.lock().unwrap().iter_mut().filter(|call| call.status == CallStatus::Running) {
            call.status = CallStatus::Cancelled;
        }
        self.finish(end, output, parsed.options.max_output_tokens, &store, started).await
    }

    /// Runs the script to its end: its calls, its output, and the host's deadline and stop.
    async fn drive(&self, mut worker: Worker, timeout: Duration, output: &mut Vec<ContentPart>) -> End {
        type Pending<'f> = std::pin::Pin<Box<dyn std::future::Future<Output = (u64, Option<usize>, Reply)> + Send + 'f>>;
        let mut in_flight: FuturesUnordered<Pending<'_>> = FuturesUnordered::new();
        let sleep = tokio::time::sleep(timeout);
        tokio::pin!(sleep);
        let mut next_call = 0u64;
        let mut output_chars = 0usize;
        let mut images = 0usize;

        let end = loop {
            tokio::select! {
                biased;
                _ = self.cancel.cancelled() => break End::Aborted,
                _ = &mut sleep => break End::Timeout(timeout.as_millis() as u64),
                Some((id, record, reply)) = in_flight.next(), if !in_flight.is_empty() => {
                    match reply {
                        Reply::Value(json) => worker.reply(id, true, json),
                        Reply::Throw(message) => worker.reply(id, false, Some(message)),
                        Reply::Stop { reason, terminate } => break End::Stopped { reason, terminate },
                    }
                    if record.is_some() {
                        self.publish();
                    }
                }
                event = worker.next() => match event {
                    Some(Event::Call { .. }) if in_flight.len() >= MAX_PENDING_CALLS => {
                        break End::Limit(format!("It started more than {MAX_PENDING_CALLS} calls without waiting for them, so it was stopped."));
                    }
                    Some(Event::Call { id, global: true, name, args }) => {
                        let args = read_arguments(&name, args.as_deref(), json!([]));
                        in_flight.push(Box::pin(async move { (id, None, self.global(&name, args).await) }));
                    }
                    Some(Event::Call { id, global: false, name, args }) => {
                        next_call += 1;
                        let call_id = format!("{}/{next_call}", self.call_id);
                        match read_arguments(&name, args.as_deref(), json!({})) {
                            // A call whose arguments the host cannot read never runs with other ones.
                            Err(message) => worker.reply(id, false, Some(message)),
                            Ok(args) => {
                                let record = self.record(&call_id, &name, &args);
                                self.publish();
                                in_flight.push(Box::pin(async move {
                                    let reply = self.call(&name, call_id, args, record).await;
                                    (id, record, reply)
                                }));
                            }
                        }
                    }
                    Some(Event::Text(text)) => {
                        output_chars += text.len();
                        if output_chars > MAX_OUTPUT_CHARS {
                            break End::Limit(format!("Its output passed {} MB, so it was stopped.", MAX_OUTPUT_CHARS / (1024 * 1024)));
                        }
                        output.push(ContentPart::text(text));
                    }
                    // Kept as the script gave it until the script ends (`inline_images`), so making
                    // a large one smaller never holds up the deadline or a stop.
                    Some(Event::Image(_)) if images >= MAX_IMAGES => {
                        output.push(ContentPart::text(format!("[An image was left out: a script's output holds at most {MAX_IMAGES} images]")));
                    }
                    Some(Event::Image(data)) => {
                        output_chars += data.len();
                        if output_chars > MAX_OUTPUT_CHARS {
                            break End::Limit(format!("Its output passed {} MB, so it was stopped.", MAX_OUTPUT_CHARS / (1024 * 1024)));
                        }
                        images += 1;
                        output.push(ContentPart::Image { data, mime_type: String::new() });
                    }
                    Some(Event::Done { value, writes }) => {
                        if output_chars + value.as_ref().map_or(0, String::len) > MAX_OUTPUT_CHARS {
                            break End::Limit(format!("Its output passed {} MB, so it was stopped.", MAX_OUTPUT_CHARS / (1024 * 1024)));
                        }
                        break End::Done { value, writes };
                    }
                    Some(Event::Failed { error }) => break End::Failed(error),
                    Some(Event::Crash(message)) => break End::Crash(message),
                    None => break End::Crash("the script's thread ended without a result".into()),
                },
            }
        };

        // Calls the script left running are cancelled, and get a moment to wind down (a
        // question they put to a person resolves) before the result goes back.
        worker.stop();
        self.calls_cancel.cancel();
        let _ = tokio::time::timeout(WIND_DOWN, async { while in_flight.next().await.is_some() {} }).await;
        end
    }

    /// Starts a call's record and returns its index, or `None` past the listing limit.
    fn record(&self, call_id: &str, name: &str, args: &Value) -> Option<usize> {
        let mut calls = self.calls.lock().unwrap();
        if calls.len() >= MAX_RECORDED_CALLS {
            *self.dropped_calls.lock().unwrap() += 1;
            return None;
        }
        calls.push(NestedCall {
            id: call_id.to_string(),
            name: self.lookup(name).map(|entry| entry.tool.name().to_string()).unwrap_or_else(|| name.to_string()),
            args: clipped(&serde_json::to_string(args).unwrap_or_default(), ARGS_PREVIEW_CHARS),
            description: args.get("description").and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty()).map(|text| clipped(text, ARGS_PREVIEW_CHARS)),
            status: CallStatus::Running,
            duration_ms: None,
            error: None,
        });
        Some(calls.len() - 1)
    }

    fn update(&self, record: Option<usize>, change: impl FnOnce(&mut NestedCall)) {
        if let Some(index) = record {
            if let Some(call) = self.calls.lock().unwrap().get_mut(index) {
                change(call);
            }
        }
    }

    /// Progress for the caller: the calls so far, with the one that started last.
    fn publish(&self) {
        (self.on_update)(ToolResult { details: self.details(), ..ToolResult::default() });
    }

    fn details(&self) -> Value {
        let calls = self.calls.lock().unwrap().clone();
        let dropped = *self.dropped_calls.lock().unwrap();
        let mut details = json!({ "calls": calls });
        if dropped > 0 {
            details["calls_not_listed"] = json!(dropped);
        }
        details
    }

    /// One tool call of the script, through the loop's tool pipeline.
    async fn call(&self, name: &str, call_id: String, args: Value, record: Option<usize>) -> Reply {
        let started = Instant::now();
        let Some(entry) = self.resolve(name).await else {
            let message = self.unknown_tool(name);
            self.update(record, |call| {
                call.status = CallStatus::Error;
                call.error = Some(clipped(&message, ERROR_PREVIEW_CHARS));
            });
            return Reply::Throw(message);
        };
        if let Some(index) = record {
            if let Some(call) = self.calls.lock().unwrap().get_mut(index) {
                call.name = entry.tool.name().to_string();
            }
        }
        let permit = tokio::select! {
            biased;
            _ = self.calls_cancel.cancelled() => None,
            permit = self.limiter.acquire() => permit.ok(),
        };
        let Some(_permit) = permit else { return self.never_ran(record) };
        let sequential = entry.tool.execution_mode() == Some(crate::agent_loop::ToolExecutionMode::Sequential);
        let _alone = if sequential {
            tokio::select! {
                biased;
                _ = self.calls_cancel.cancelled() => return self.never_ran(record),
                guard = self.exclusive.lock() => Some(guard),
            }
        } else {
            None
        };
        // A call still waiting when the script ended never starts: no hook sees it, and no card
        // asks about it.
        if self.calls_cancel.is_cancelled() {
            return self.never_ran(record);
        }
        let outcome = self.tools.run(entry.tool.clone(), call_id, args, self.calls_cancel.child_token()).await;
        let cancelled = self.calls_cancel.is_cancelled();
        self.update(record, |call| {
            call.duration_ms = Some(started.elapsed().as_millis() as u64);
            call.status = match (&outcome, cancelled) {
                (ToolOutcome { blocked: true, .. }, _) => CallStatus::Blocked,
                (ToolOutcome { is_error: true, .. }, true) => CallStatus::Cancelled,
                (ToolOutcome { is_error: true, .. }, false) => CallStatus::Error,
                _ => CallStatus::Ok,
            };
            if outcome.is_error {
                call.error = Some(clipped(&outcome.result.text_content(), ERROR_PREVIEW_CHARS));
            }
        });
        script_reply(entry.tool.as_ref(), &outcome)
    }

    /// A call that was waiting to start when the script ended.
    fn never_ran(&self, record: Option<usize>) -> Reply {
        self.update(record, |call| call.status = CallStatus::Cancelled);
        Reply::Throw("The script ended before this call started".into())
    }

    /// `searchTools()`, `describeTool()`, `describeNamespace()`, and the host's functions.
    async fn global(&self, name: &str, args: Result<Value, String>) -> Reply {
        let args: Vec<Value> = match args {
            Ok(Value::Array(args)) => args,
            Ok(_) => Vec::new(),
            Err(message) => return Reply::Throw(message),
        };
        match name {
            "searchTools" => {
                let Some(query) = args.first().and_then(Value::as_str) else { return Reply::Throw("searchTools() expects a query string".into()) };
                let options = args.get(1).cloned().unwrap_or(Value::Null);
                let limit = match options.get("limit") {
                    None | Some(Value::Null) => DEFAULT_SEARCH_LIMIT,
                    Some(limit) => match limit.as_u64().filter(|limit| *limit > 0) {
                        Some(limit) => limit as usize,
                        None => return Reply::Throw("searchTools() limit must be a positive integer".into()),
                    },
                };
                let namespace = match options.get("namespace") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(namespace)) => Some(namespace.as_str()),
                    Some(_) => return Reply::Throw("searchTools() namespace must be a string".into()),
                };
                match self.tool.catalog.search(query, namespace, limit, &self.calls_cancel).await {
                    Ok(found) => {
                        let rows: Vec<Value> = found
                            .iter()
                            .filter(|entry| entry.tool.name() != CODEMODE_TOOL_NAME)
                            .map(|entry| {
                                self.remember(entry);
                                json!({ "name": to_identifier(entry.tool.name()), "description": entry.sample() })
                            })
                            .collect();
                        Reply::Value(Some(Value::Array(rows).to_string()))
                    }
                    Err(error) => Reply::Throw(error),
                }
            }
            "describeTool" => {
                let Some(name) = args.first().and_then(Value::as_str) else { return Reply::Throw("describeTool() expects a tool name".into()) };
                match self.resolve(name).await {
                    Some(entry) => Reply::Value(Some(Value::String(entry.sample()).to_string())),
                    None => Reply::Value(None),
                }
            }
            "describeNamespace" => {
                let Some(name) = args.first().and_then(Value::as_str) else { return Reply::Throw("describeNamespace() expects a namespace name".into()) };
                let Some(details) = self.tool.catalog.describe_namespace(name, &self.calls_cancel).await else { return Reply::Value(None) };
                let tools: Vec<String> = details.tools.iter().filter(|tool| *tool != CODEMODE_TOOL_NAME).map(|tool| to_identifier(tool)).collect();
                let mut value = json!({ "name": details.name, "tools": tools });
                if !details.description.trim().is_empty() {
                    value["description"] = json!(details.description);
                }
                if !details.instructions.trim().is_empty() {
                    value["instructions"] = json!(details.instructions);
                }
                Reply::Value(Some(value.to_string()))
            }
            other => {
                let Some(function) = self.tool.functions.iter().find(|function| function.name() == other).cloned() else {
                    return Reply::Throw(format!("Unknown global \"{other}\""));
                };
                let started = Instant::now();
                let record = self.record(&format!("{}/{other}", self.call_id), other, &Value::Array(args.clone()));
                self.publish();
                let outcome = function.call(args, &self.calls_cancel).await;
                self.update(record, |call| {
                    call.duration_ms = Some(started.elapsed().as_millis() as u64);
                    match &outcome {
                        Ok(_) => call.status = CallStatus::Ok,
                        Err(error) => {
                            call.status = if self.calls_cancel.is_cancelled() { CallStatus::Cancelled } else { CallStatus::Error };
                            call.error = Some(clipped(error, ERROR_PREVIEW_CHARS));
                        }
                    }
                });
                self.publish();
                match outcome {
                    Ok(Value::Null) => Reply::Value(None),
                    Ok(value) => Reply::Value(Some(value.to_string())),
                    Err(error) => Reply::Throw(error),
                }
            }
        }
    }

    async fn finish(&self, end: End, items: Vec<ContentPart>, max_output_tokens: Option<u64>, stored: &BTreeMap<String, String>, started: Instant) -> ScriptRun {
        let mut items = inline_images(items).await;
        let calls = self.calls.lock().unwrap().clone();
        let mut returned = None;
        // The store's limits hold here too, whatever the script did to its own copy of them.
        let end = match end {
            End::Done { value, writes } => match parse_writes(&writes).and_then(|writes| check_writes(stored, writes)) {
                Ok(writes) => {
                    if !writes.is_empty() {
                        if let Some(store) = &self.tool.store {
                            store.save(&writes);
                        }
                    }
                    if let Some(value) = &value {
                        items.push(ContentPart::text(value_text(value)));
                        returned = serde_json::from_str(value).ok();
                    }
                    End::Done { value, writes: String::new() }
                }
                Err(why) => End::Limit(format!("Its store() writes were not saved: {why}")),
            },
            other => other,
        };
        let (ok, terminate) = match &end {
            End::Done { .. } => (true, false),
            End::Stopped { reason, terminate } => {
                items.push(ContentPart::text(format!("Script stopped: {reason}\n\n{}", call_summary(&calls))));
                (false, *terminate)
            }
            other => {
                let head = match other {
                    End::Failed(error) => {
                        let error: Value = serde_json::from_str(error).unwrap_or_else(|_| json!({ "message": error }));
                        let text = error["stack"].as_str().map(str::to_string).unwrap_or_else(|| {
                            format!("{}: {}", error["name"].as_str().unwrap_or("Error"), error["message"].as_str().unwrap_or(""))
                        });
                        clipped(&text, MAX_ERROR_CHARS)
                    }
                    End::Timeout(ms) => format!("Script timed out: Execution timed out after {ms} ms"),
                    End::Aborted => "Script aborted: the run was stopped".into(),
                    End::Limit(message) => message.clone(),
                    End::Crash(message) => format!("Script sandbox failed: {message}"),
                    End::Done { .. } | End::Stopped { .. } => unreachable!(),
                };
                items.push(ContentPart::text(format!("Script error:\n{head}\n\n{}", call_summary(&calls))));
                (false, false)
            }
        };

        let budget = max_output_tokens.map_or(self.tool.options.max_output_tokens, |tokens| (tokens as usize).min(MAX_OUTPUT_TOKENS));
        let (items, full_output_path) = truncate_output(items, budget).await;
        let header = format!(
            "{}\nWall time {:.1} seconds\nOutput:\n",
            match (&end, ok) {
                (End::Stopped { .. }, _) => "Script stopped",
                (_, true) => "Script completed",
                (_, false) => "Script failed",
            },
            started.elapsed().as_secs_f64()
        );
        let mut content = vec![ContentPart::text(header)];
        content.extend(items);
        let mut details = self.details();
        if let Some(path) = full_output_path {
            details["full_output_path"] = json!(path);
        }
        ScriptRun { result: ToolResult { content, details, structured: None, is_error: !ok, terminate }, returned }
    }
}

/// The value a script receives for a call: the structured output of a tool that declares a
/// schema for it, also when the call failed with one (an MCP result with `isError`); the text
/// of any other tool. Other failures reject, and a call a hook refused ends the script.
fn script_reply(tool: &dyn Tool, outcome: &ToolOutcome) -> Reply {
    if outcome.blocked {
        let reason = outcome.result.text_content();
        return Reply::Stop { reason: if reason.trim().is_empty() { format!("{} was not allowed", tool.name()) } else { reason }, terminate: outcome.result.terminate };
    }
    if tool.output_schema().is_some() {
        if let Some(structured) = &outcome.result.structured {
            return Reply::Value(Some(structured.to_string()));
        }
    }
    let text = outcome.result.text_content();
    if outcome.is_error {
        return Reply::Throw(if text.trim().is_empty() { format!("Tool \"{}\" failed", tool.name()) } else { text });
    }
    Reply::Value(Some(Value::String(text).to_string()))
}

/// A call's arguments as the host reads them: `absent` when the script passed none, and an
/// error the call rejects with when they are too large or are not JSON the host can read.
fn read_arguments(name: &str, json: Option<&str>, absent: Value) -> Result<Value, String> {
    match json {
        None => Ok(absent),
        Some(json) if json.len() > MAX_ARGUMENT_CHARS => Err(format!("The arguments of {name} are over {} MB", MAX_ARGUMENT_CHARS / (1024 * 1024))),
        Some(json) => serde_json::from_str(json).map_err(|error| format!("The arguments of {name} could not be read: {error}")),
    }
}

/// The script's images as a model takes them (`images::prepare`), made off the async threads;
/// one that cannot go becomes a line saying why.
async fn inline_images(items: Vec<ContentPart>) -> Vec<ContentPart> {
    if !items.iter().any(|item| matches!(item, ContentPart::Image { .. })) {
        return items;
    }
    let made = tokio::task::spawn_blocking(move || {
        items
            .into_iter()
            .flat_map(|item| match item {
                ContentPart::Image { data, .. } => match crate::images::prepare_base64(&data) {
                    Ok(image) => image.into_parts(),
                    Err(why) => vec![ContentPart::text(format!("[An image was left out: {why}]"))],
                },
                other => vec![other],
            })
            .collect()
    });
    made.await.unwrap_or_else(|error| vec![ContentPart::text(format!("[The script's output was lost making its images: {error}]"))])
}

/// A successful script's `store()` writes, from `[[key, json], [key], …]`. Anything else is an
/// error, never a silent loss.
fn parse_writes(writes: &str) -> Result<StoreWrites, String> {
    let mut result = StoreWrites::default();
    let entries: Vec<Vec<String>> = serde_json::from_str(writes).map_err(|error| format!("they could not be read ({error})"))?;
    for entry in entries {
        match entry.as_slice() {
            [key] => {
                result.set.remove(key);
                result.delete.push(key.clone());
            }
            [key, json] => {
                result.delete.retain(|deleted| deleted != key);
                let value = serde_json::from_str(json).map_err(|error| format!("the value of {key:?} could not be read ({error})"))?;
                result.set.insert(key.clone(), value);
            }
            _ => return Err("they could not be read".into()),
        }
    }
    Ok(result)
}

/// The writes, when the store stays within its limits after them.
fn check_writes(stored: &BTreeMap<String, String>, writes: StoreWrites) -> Result<StoreWrites, String> {
    let mut sizes: BTreeMap<&str, usize> = stored.iter().map(|(key, json)| (key.as_str(), key.len() + json.len())).collect();
    for key in &writes.delete {
        sizes.remove(key.as_str());
    }
    for (key, value) in &writes.set {
        let json = serde_json::to_string(value).unwrap_or_default().len();
        if json > MAX_STORE_VALUE_CHARS {
            return Err(format!("the value of {key:?} is over {MAX_STORE_VALUE_CHARS} characters of JSON"));
        }
        sizes.insert(key.as_str(), key.len() + json);
    }
    if sizes.values().sum::<usize>() > MAX_STORE_TOTAL_CHARS {
        return Err(format!("the stored values would pass {MAX_STORE_TOTAL_CHARS} characters of JSON"));
    }
    Ok(writes)
}

/// A returned value as `text()` would write it: a string as it is, anything else as JSON.
fn value_text(json: &str) -> String {
    match serde_json::from_str::<Value>(json) {
        Ok(Value::String(text)) => text,
        _ => json.to_string(),
    }
}

fn call_summary(calls: &[NestedCall]) -> String {
    if calls.is_empty() {
        return "No tool calls were made.".into();
    }
    let listed: Vec<String> = calls
        .iter()
        .map(|call| {
            let status = match call.status {
                CallStatus::Running => "running",
                CallStatus::Ok => "ok",
                CallStatus::Error => "error",
                CallStatus::Blocked => "not allowed",
                CallStatus::Cancelled => "cancelled",
            };
            format!("{} ({status})", call.name)
        })
        .collect();
    format!("Tool calls made before the script ended (they are not undone): {}", listed.join(", "))
}

fn clipped(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars.saturating_sub(3)) {
        Some((end, _)) if text.chars().count() > chars => format!("{}...", &text[..end]),
        _ => text.to_string(),
    }
}

/// Keeps the text within `max_tokens`: over it, the text items become one item that keeps the
/// start and the end, and the images follow it. The whole text goes to a temp file.
async fn truncate_output(items: Vec<ContentPart>, max_tokens: usize) -> (Vec<ContentPart>, Option<String>) {
    let texts: Vec<&str> = items.iter().filter_map(ContentPart::as_text).collect();
    let combined = texts.join("\n");
    let budget = max_tokens.saturating_mul(CHARS_PER_TOKEN);
    let total = combined.chars().count();
    if texts.is_empty() || total <= budget {
        return (items, None);
    }
    let head_chars = budget / 2;
    let tail_chars = budget - head_chars;
    let head: String = combined.chars().take(head_chars).collect();
    let tail: String = combined.chars().skip(total - tail_chars).collect();
    let removed = total - head_chars - tail_chars;
    let mut text = format!(
        "Warning: truncated output (original token count: {})\nTotal output lines: {}\n\n{head}…{} tokens truncated…{tail}",
        total.div_ceil(CHARS_PER_TOKEN),
        combined.lines().count(),
        removed.div_ceil(CHARS_PER_TOKEN)
    );
    let path = std::env::temp_dir().join(format!("lorca-codemode-{}.txt", uuid::Uuid::new_v4()));
    let saved = write_private(&path, combined.as_bytes()).await;
    let full_output_path = match saved {
        Ok(()) => {
            text.push_str(&format!("\n\n[Full output: {} (read it with offset/limit)]", path.display()));
            Some(path.display().to_string())
        }
        Err(error) => {
            text.push_str(&format!("\n\n[Could not save the full output: {error}]"));
            None
        }
    };
    let mut kept = vec![ContentPart::text(text)];
    kept.extend(items.into_iter().filter(|item| matches!(item, ContentPart::Image { .. })));
    (kept, full_output_path)
}

/// Script output can hold private data, so only the user may read the file.
async fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).await?;
    tokio::io::AsyncWriteExt::write_all(&mut file, bytes).await?;
    // Tokio finishes a write in the background; the result names the file as soon as it returns.
    tokio::io::AsyncWriteExt::flush(&mut file).await
}

// MARK: - Description

// After pi 1.0's: one line per global, and what errors teach left to the errors, which name the
// closest tools, the limits a script ran into, and the calls made before a failure.
const DESCRIPTION_INTRO: &str = "Run JavaScript that calls other tools. Only what the script outputs or returns reaches you, never the \
results of the calls it makes. The input is raw JavaScript (not JSON, no code fence), run as an async function body in a QuickJS \
sandbox: top-level `await` and `return` work. No Node, file system, network, or timers.
- `await tools.<name>({ ...args })` resolves to a string, or an object if the tool's declaration says so, and rejects with an Error on failure. Calls still running when the script ends are cancelled.
- Optional first line: `// @options: {\"max_output_tokens\": {max_output_tokens}, \"timeout_ms\": 60000}`

Globals:
- `text(value)`, `image(dataUrlOrImageBlock)`, `console.log(...)`, and top-level `return` add output; `exit()` ends the script.
- `store(key, value)` and `load(key)` keep JSON values for later scripts in this chat.
- `ALL_TOOLS`, `searchTools(query, { limit?, namespace? })`, and `describeTool(name)` find tools that are not listed below. \
`describeNamespace(name)` gives a namespace's description, its whole instructions, and every tool in it.";

const MCP_RESULT_GUIDANCE: &str = "Shared MCP types. An MCP tool resolves to its whole `CallToolResult`: `structuredContent` when \
its declaration types it, else `content`, usually one text block of JSON (`JSON.parse(result.content[0].text)`). `isError: true` \
means it failed.";

struct CatalogEntry {
    name: String,
    section: String,
    cost: usize,
    deferred: bool,
}

/// The description: the globals, the shared MCP types when an MCP tool is callable, the direct
/// tools by name, and a section per tool, grouped by namespace, within the budget. Every
/// namespace is named either way, marked when some of its tools are not listed, as in pi.
fn describe(entries: &[Entry], namespaces: &[Namespace], functions: &[Arc<dyn HostFunction>], options: &CodemodeOptions) -> String {
    let mut sections = vec![DESCRIPTION_INTRO.replace("{max_output_tokens}", &options.max_output_tokens.to_string())];

    let callable: Vec<&Entry> = entries.iter().filter(|entry| entry.tool.name() != CODEMODE_TOOL_NAME).collect();
    let direct: Vec<&Entry> = callable.iter().copied().filter(|entry| entry.exposure == Exposure::Direct).collect();
    let listable: Vec<&Entry> = callable.iter().copied().filter(|entry| entry.exposure != Exposure::Direct).collect();

    // Groups: tools in no namespace first, then namespaces by name, with every known namespace
    // named even when it has no tool yet. A namespace the catalog did not describe is named
    // without a description.
    let mut named: Vec<Namespace> = namespaces.to_vec();
    for entry in &listable {
        if let Some(name) = &entry.namespace {
            if !named.iter().any(|namespace| &namespace.name == name) {
                named.push(Namespace { name: name.clone(), description: String::new() });
            }
        }
    }
    named.sort_by(|a, b| a.name.cmp(&b.name));
    named.dedup_by(|a, b| a.name == b.name);
    let mut groups: Vec<(Option<&Namespace>, Vec<CatalogEntry>)> = vec![(None, Vec::new())];
    for namespace in &named {
        groups.push((Some(namespace), Vec::new()));
    }
    for entry in &listable {
        let tool = &entry.tool;
        let identifier = to_identifier(tool.name());
        let heading = if identifier == tool.name() { format!("### `{identifier}`") } else { format!("### `{identifier}` (`{}`)", tool.name()) };
        let section = format!("{heading}\n{}", entry.sample().trim());
        let group = match &entry.namespace {
            Some(name) => groups.iter_mut().find(|(namespace, _)| namespace.is_some_and(|namespace| &namespace.name == name)),
            None => groups.first_mut(),
        };
        if let Some((_, members)) = group {
            members.push(CatalogEntry { name: tool.name().to_string(), cost: section.len().div_ceil(CHARS_PER_TOKEN), section, deferred: entry.exposure == Exposure::Deferred });
        }
    }

    let shown = select_catalog(&groups, options.inline_budget);
    if options.mcp_types || listable.iter().any(|entry| mcp_structured_content_schema(entry.tool.output_schema().as_ref()).is_some()) {
        sections.push(format!("{MCP_RESULT_GUIDANCE}\n```ts\n{MCP_TYPESCRIPT_PREAMBLE}\n```"));
    }
    if !functions.is_empty() {
        sections.push(format!("Host functions:\n```ts\n{}\n```", render_functions(functions)));
    }
    if !direct.is_empty() {
        let names = direct.iter().map(|entry| format!("`{}`", entry.tool.name())).collect::<Vec<_>>().join(", ");
        // A tool with an output schema says what it resolves to, as pi's descriptions do.
        let typed: Vec<String> = direct
            .iter()
            .filter_map(|entry| entry.tool.output_schema().map(|schema| format!("`{}` resolves to `{}`", entry.tool.name(), declarations::output_summary(&schema))))
            .collect();
        sections.push(if typed.is_empty() {
            format!("Your own tools {names} are callable here too, with the same arguments, and resolve to their text output.")
        } else {
            let others = if typed.len() < direct.len() { "; the others resolve to their text output" } else { "" };
            format!("Your own tools {names} are callable here too, with the same arguments. {}{others}.", typed.join("; "))
        });
    }

    let mut listing = vec!["Nested tools:".to_string()];
    for (namespace, members) in &groups {
        let visible: Vec<&CatalogEntry> = members.iter().filter(|member| shown.contains(&member.name)).collect();
        if let Some(namespace) = namespace {
            // Only what is missing is marked, so the heading stays the same while the tools do.
            let marker = if members.is_empty() {
                " (tools not known yet; describeNamespace() lists them)"
            } else if visible.is_empty() {
                " (tools not listed)"
            } else if visible.len() < members.len() {
                " (some tools not listed)"
            } else {
                ""
            };
            let description = namespace.description.trim();
            listing.push(if description.is_empty() { format!("## {}{marker}", namespace.name) } else { format!("## {}{marker}\n{description}", namespace.name) });
        }
        for member in visible {
            listing.push(member.section.clone());
        }
    }
    if !listable.is_empty() || !named.is_empty() {
        sections.push(listing.join("\n\n"));
    }
    if let Some(guidance) = options.guidance.as_deref().map(str::trim).filter(|guidance| !guidance.is_empty()) {
        sections.push(guidance.to_string());
    }
    sections.join("\n\n")
}

/// Host functions as TypeScript: `declare function name…;`, or members of `declare const ns`
/// for `ns.member` names.
fn render_functions(functions: &[Arc<dyn HostFunction>]) -> String {
    let mut sections = Vec::new();
    let mut namespaces: Vec<(String, Vec<String>)> = Vec::new();
    for function in functions {
        match function.name().split_once('.') {
            None => sections.push(format!("{}declare function {}{};", doc_comment(function.description(), ""), function.name(), function.signature())),
            Some((namespace, member)) => {
                let line = format!("{}  {member}{};", doc_comment(function.description(), "  "), function.signature());
                match namespaces.iter_mut().find(|(name, _)| name == namespace) {
                    Some((_, members)) => members.push(line),
                    None => namespaces.push((namespace.to_string(), vec![line])),
                }
            }
        }
    }
    for (namespace, members) in namespaces {
        sections.push(format!("declare const {namespace}: {{\n{}\n}};", members.join("\n")));
    }
    sections.join("\n\n")
}

/// Picks the sections that fit the budget, after OpenCode's catalog: each round, every group
/// (tools in no namespace first) places its cheapest remaining tool; a group whose next tool
/// does not fit drops out while the others go on. Every namespace gets a tool in before any
/// namespace is complete.
fn select_catalog(groups: &[(Option<&Namespace>, Vec<CatalogEntry>)], budget: usize) -> Vec<String> {
    let mut queues: Vec<Vec<&CatalogEntry>> = groups
        .iter()
        .map(|(_, members)| {
            let mut queue: Vec<&CatalogEntry> = members.iter().filter(|member| !member.deferred).collect();
            queue.sort_by_key(|member| member.cost);
            queue.reverse();
            queue
        })
        .collect();
    let mut shown = Vec::new();
    let mut remaining = budget;
    let mut active: Vec<usize> = (0..queues.len()).filter(|index| !queues[*index].is_empty()).collect();
    while !active.is_empty() {
        active.retain(|&index| {
            let queue = &mut queues[index];
            let Some(next) = queue.last() else { return false };
            if next.cost > remaining {
                return false;
            }
            remaining -= next.cost;
            shown.push(next.name.clone());
            queue.pop();
            !queue.is_empty()
        });
    }
    shown
}

#[cfg(test)]
mod tests;
