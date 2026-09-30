use std::sync::Mutex as StdMutex;

use super::*;
use crate::tool::ToolUpdateFn;

/// A tool that answers with its arguments as JSON, or fails, or waits until it is cancelled.
struct Probe {
    name: &'static str,
    mode: Mode,
    output_schema: Option<Value>,
}

#[derive(Clone, Copy)]
enum Mode {
    Echo,
    Fail,
    Hang,
    Structured,
}

impl Probe {
    fn tool(name: &'static str, mode: Mode) -> Arc<dyn Tool> {
        let output_schema = matches!(mode, Mode::Structured).then(|| json!({ "type": "object", "properties": { "id": { "type": "number" } } }));
        Arc::new(Probe { name, mode, output_schema })
    }
}

#[async_trait]
impl Tool for Probe {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "Answers with its arguments"
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": { "id": { "type": "number", "description": "Any number" } } })
    }
    fn output_schema(&self) -> Option<Value> {
        self.output_schema.clone()
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        match self.mode {
            Mode::Echo => Ok(ToolResult::text(args.to_string())),
            Mode::Fail => Err(ToolError("boom".into())),
            Mode::Hang => {
                cancel.cancelled().await;
                Err(ToolError("Stopped".into()))
            }
            Mode::Structured => Ok(ToolResult { structured: Some(json!({ "id": args["id"] })), ..ToolResult::text("structured") }),
        }
    }
}

fn tool(tools: Vec<Arc<dyn Tool>>) -> CodemodeTool {
    CodemodeTool::new(Arc::new(StaticCatalog::new(tools)), CodemodeOptions::default())
}

async fn run(codemode: &CodemodeTool, code: &str) -> ToolResult {
    run_with(codemode, code, &DirectRunner).await
}

async fn run_with(codemode: &CodemodeTool, code: &str, runner: &dyn ToolRunner) -> ToolResult {
    codemode.execute_with("call-1", json!({ "code": code }), CancellationToken::new(), Arc::new(|_| {}), runner).await.expect("codemode ran")
}

fn text_of(result: &ToolResult) -> String {
    result.text_content()
}

#[tokio::test]
async fn a_script_calls_tools_and_only_its_output_comes_back() {
    let codemode = tool(vec![Probe::tool("echo", Mode::Echo)]);
    let result = run(&codemode, "const a = await tools.echo({ id: 1 });\ntext('got ' + a);\nreturn { ok: true };").await;
    let text = text_of(&result);
    assert!(!result.is_error, "{text}");
    assert!(text.starts_with("Script completed\nWall time "), "{text}");
    assert!(text.contains("got {\"id\":1}"), "{text}");
    assert!(text.ends_with("{\"ok\":true}"), "{text}");
    assert_eq!(result.details["calls"][0]["name"], "echo");
    assert_eq!(result.details["calls"][0]["status"], "ok");
    assert_eq!(result.details["calls"][0]["id"], "call-1/1");
}

#[tokio::test]
async fn parallel_calls_resolve_to_structured_output() {
    let codemode = tool(vec![Probe::tool("lookup", Mode::Structured)]);
    let result = run(&codemode, "const [x, y] = await Promise.all([tools.lookup({ id: 1 }), tools.lookup({ id: 2 })]);\nreturn x.id + y.id;").await;
    assert!(text_of(&result).ends_with("\n3"), "{}", text_of(&result));
    assert_eq!(result.details["calls"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn a_failed_call_rejects_and_an_unknown_tool_too() {
    let codemode = tool(vec![Probe::tool("fails", Mode::Fail)]);
    let result = run(&codemode, "try { await tools.fails({}); } catch (error) { text('caught: ' + error.message); }\nawait tools.nope({});").await;
    let text = text_of(&result);
    assert!(result.is_error);
    assert!(text.contains("caught: boom"), "{text}");
    assert!(text.contains("Script error:\nError: Unknown tool \"nope\""), "{text}");
    assert!(text.contains("fails (error), nope (error)"), "{text}");
}

/// Refuses every call to one tool, as a permission hook would.
struct Refusing(&'static str);

#[async_trait]
impl ToolRunner for Refusing {
    async fn run(&self, tool: Arc<dyn Tool>, tool_call_id: String, args: Value, cancel: CancellationToken) -> ToolOutcome {
        if tool.name() == self.0 {
            return ToolOutcome { result: ToolResult { terminate: true, ..ToolResult::text("The user did not allow it.") }, is_error: true, blocked: true };
        }
        DirectRunner.run(tool, tool_call_id, args, cancel).await
    }
}

#[tokio::test]
async fn a_refused_call_ends_the_script() {
    let codemode = tool(vec![Probe::tool("echo", Mode::Echo), Probe::tool("danger", Mode::Echo)]);
    let code = "await tools.echo({ id: 1 });\ntry { await tools.danger({}); } catch (error) { text('caught'); }\ntext('after');";
    let result = run_with(&codemode, code, &Refusing("danger")).await;
    let text = text_of(&result);
    assert!(result.is_error);
    assert!(result.terminate, "the refusal's stop hint carries over");
    assert!(text.starts_with("Script stopped\n"), "{text}");
    assert!(text.contains("Script stopped: The user did not allow it."), "{text}");
    assert!(!text.contains("caught") && !text.contains("after"), "{text}");
    assert!(text.contains("echo (ok), danger (not allowed)"), "{text}");
    assert_eq!(result.details["calls"][1]["status"], "blocked");
}

/// A catalog whose second tool only turns up when asked for by name or found by a search.
struct Late;

#[async_trait]
impl Catalog for Late {
    fn entries(&self) -> Vec<Entry> {
        vec![Entry::new(Probe::tool("echo", Mode::Echo), Exposure::Direct)]
    }
    fn namespaces(&self) -> Vec<Namespace> {
        vec![Namespace { name: "late".into(), description: "Connects on first use".into() }]
    }
    async fn find(&self, name: &str, _cancel: &CancellationToken) -> Option<Entry> {
        (name == "late__lookup").then(|| Entry::new(Probe::tool("late__lookup", Mode::Structured), Exposure::Listed).in_namespace("late"))
    }
    async fn search(&self, query: &str, _namespace: Option<&str>, _limit: usize, _cancel: &CancellationToken) -> Result<Vec<Entry>, String> {
        Ok(if query.contains("lookup") { self.find("late__lookup", &CancellationToken::new()).await.into_iter().collect() } else { Vec::new() })
    }
}

#[tokio::test]
async fn searches_and_lookups_reach_tools_the_catalog_finds_late() {
    let codemode = CodemodeTool::new(Arc::new(Late), CodemodeOptions::default());
    let description = codemode.description();
    assert!(description.contains("Your own tools `echo` are callable here too"), "{description}");
    assert!(description.contains("## late (tools not known yet; searchTools() finds them)\nConnects on first use"), "{description}");
    let code = "const found = await searchTools('lookup things');\nconst described = await describeTool(found[0].name);\n\
                const missing = await describeTool('nothing');\nconst { id } = await tools[found[0].name]({ id: 7 });\n\
                return [found.length, found[0].name, described.includes('codemode tool declaration'), missing === undefined, id, ALL_TOOLS.length];";
    let result = run(&codemode, code).await;
    assert!(text_of(&result).ends_with("[1,\"late__lookup\",true,true,7,1]"), "{}", text_of(&result));
}

#[derive(Default)]
struct MemoryStore(StdMutex<BTreeMap<String, Value>>);

impl CodemodeStore for MemoryStore {
    fn load(&self) -> BTreeMap<String, Value> {
        self.0.lock().unwrap().clone()
    }
    fn save(&self, writes: &StoreWrites) {
        let mut values = self.0.lock().unwrap();
        for key in &writes.delete {
            values.remove(key);
        }
        values.extend(writes.set.clone());
    }
}

#[tokio::test]
async fn stored_values_reach_later_scripts_and_failed_scripts_write_nothing() {
    let store = Arc::new(MemoryStore::default());
    store.0.lock().unwrap().insert("old".into(), json!(1));
    let codemode = tool(vec![]).with_store(store.clone());
    let first = run(&codemode, "store('verdicts', [{ id: 'PI-1', mood: 'calm' }]);\nstore('old', undefined);\nreturn load('verdicts').length;").await;
    assert!(text_of(&first).ends_with("\n1"), "{}", text_of(&first));
    assert_eq!(*store.0.lock().unwrap(), BTreeMap::from([("verdicts".to_string(), json!([{ "id": "PI-1", "mood": "calm" }]))]));
    let failed = run(&codemode, "store('verdicts', []);\nthrow new Error('no');").await;
    assert!(failed.is_error);
    let second = run(&codemode, "return load('verdicts')[0].mood;").await;
    assert!(text_of(&second).ends_with("\ncalm"), "{}", text_of(&second));
    let too_big = run(&codemode, "store('big', 'x'.repeat(300000));").await;
    assert!(text_of(&too_big).contains("RangeError"), "{}", text_of(&too_big));
}

#[tokio::test]
async fn exit_ends_early_and_output_keeps_its_order() {
    let codemode = tool(vec![]);
    let result = run(&codemode, "console.log('a', { b: 1 });\nimage('data:image/png;base64,AAAA');\ntext(2);\nexit();\ntext('never');").await;
    assert!(!result.is_error);
    assert_eq!(result.content[1], ContentPart::text("a {\"b\":1}"));
    assert_eq!(result.content[2], ContentPart::Image { data: "AAAA".into(), mime_type: "image/png".into() });
    assert_eq!(result.content[3], ContentPart::text("2"));
    assert_eq!(result.content.len(), 4);
    let remote = run(&codemode, "image('https://example.com/a.png');").await;
    assert!(text_of(&remote).contains("remote image URLs are not supported"), "{}", text_of(&remote));
}

#[tokio::test]
async fn a_spinning_script_times_out_and_a_stop_aborts_a_waiting_one() {
    let codemode = tool(vec![Probe::tool("hang", Mode::Hang)]);
    let result = run(&codemode, "// @options: {\"timeout_ms\": 200}\nwhile (true) {}").await;
    assert!(text_of(&result).contains("Script timed out: Execution timed out after 200 ms"), "{}", text_of(&result));

    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        stop.cancel();
    });
    let started = Instant::now();
    let result = codemode.execute_with("call-2", json!({ "code": "await tools.hang({});" }), cancel, Arc::new(|_| {}), &DirectRunner).await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(text_of(&result).contains("Script aborted"), "{}", text_of(&result));
    assert_eq!(result.details["calls"][0]["status"], "cancelled");
}

#[tokio::test]
async fn scripts_that_can_never_finish_or_do_not_parse_fail_at_once() {
    let codemode = tool(vec![]);
    let stalled = run(&codemode, "await new Promise(() => {});").await;
    assert!(text_of(&stalled).contains("can never settle"), "{}", text_of(&stalled));
    let broken = run(&codemode, "return (;").await;
    assert!(broken.is_error);
    assert!(text_of(&broken).contains("SyntaxError"), "{}", text_of(&broken));
    let thrown = run(&codemode, "\n\nnull.x;").await;
    assert!(text_of(&thrown).contains("codemode.js:3"), "{}", text_of(&thrown));
}

#[tokio::test]
async fn memory_and_stack_limits_fail_inside_the_script() {
    let codemode = CodemodeTool::new(Arc::new(StaticCatalog::new(vec![])), CodemodeOptions { memory_limit: 32 * 1024 * 1024, ..CodemodeOptions::default() });
    let hungry = run(&codemode, "const all = [];\nwhile (true) all.push('x'.repeat(100000) + all.length);").await;
    assert!(hungry.is_error);
    assert!(text_of(&hungry).to_lowercase().contains("memory"), "{}", text_of(&hungry));
    let deep = run(&codemode, "function f(n) { return f(n + 1) + 1; }\ntry { f(0); } catch (error) { return 'caught ' + error.name; }").await;
    assert!(text_of(&deep).contains("caught"), "{}", text_of(&deep));
}

#[tokio::test]
async fn long_output_keeps_its_ends_and_goes_to_a_file() {
    let codemode = tool(vec![]);
    let result = run(&codemode, "// @options: {\"max_output_tokens\": 10}\ntext('a'.repeat(30) + 'b'.repeat(1000) + 'c'.repeat(30));").await;
    let text = text_of(&result);
    assert!(text.contains("Warning: truncated output (original token count: 265)"), "{text}");
    assert!(text.contains(&format!("{}…", "a".repeat(20))), "{text}");
    let path = result.details["full_output_path"].as_str().expect("saved");
    assert_eq!(std::fs::read_to_string(path).unwrap().len(), 1060);
    let _ = std::fs::remove_file(path);
}

#[test]
fn the_description_lists_tools_by_namespace_within_its_budget() {
    let mcp_output = json!({
        "type": "object",
        "properties": { "content": { "type": "array", "items": { "type": "object" } }, "isError": { "type": "boolean" } }
    });
    struct Mcp(&'static str, Value);
    #[async_trait]
    impl Tool for Mcp {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "An MCP tool with a long description that takes room in the listing"
        }
        fn parameters(&self) -> Value {
            json!({ "type": "object", "properties": { "q": { "type": "string" } }, "required": ["q"] })
        }
        fn output_schema(&self) -> Option<Value> {
            Some(self.1.clone())
        }
        async fn execute(&self, _id: &str, _args: Value, _cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::text("ok"))
        }
    }
    let entries = vec![
        Entry::new(Probe::tool("read", Mode::Echo), Exposure::Direct),
        Entry::new(Arc::new(Mcp("github__search_issues", mcp_output.clone())), Exposure::Listed).in_namespace("github"),
        Entry::new(Arc::new(Mcp("github__create_issue", mcp_output.clone())), Exposure::Listed).in_namespace("github"),
        Entry::new(Arc::new(Mcp("linear__list_issues", mcp_output.clone())), Exposure::Listed).in_namespace("linear"),
        Entry::new(Arc::new(Mcp("linear__hidden", mcp_output)), Exposure::Deferred).in_namespace("linear"),
    ];
    let namespaces = vec![Namespace { name: "github".into(), description: "GitHub: issues and pull requests".into() }, Namespace { name: "notion".into(), description: "Notion".into() }];
    let complete = describe(&entries, &namespaces, &[], &CodemodeOptions { inline_budget: 100_000, ..CodemodeOptions::default() });
    assert!(complete.contains("Nested tools: PARTIAL - 3 of 4 shown."), "a deferred tool is never listed: {complete}");
    assert!(complete.contains("Shared MCP types. An MCP tool resolves to its whole `CallToolResult`"));
    assert!(complete.contains("## github (2 tools)\nGitHub: issues and pull requests"), "{complete}");
    assert!(complete.contains("## linear (2 tools, 1 shown)"), "{complete}");
    assert!(complete.contains("## notion (tools not known yet; searchTools() finds them)\nNotion"), "{complete}");
    assert!(complete.contains("github__create_issue(args: { q: string; }): Promise<CallToolResult>;"), "{complete}");
    assert!(!complete.contains("### `read`"), "a direct tool is named, not listed");

    let local_only = describe(&entries[..1], &[], &[], &CodemodeOptions::default());
    assert!(!local_only.contains("Shared MCP types"), "no MCP tool, no MCP types");
    let before_connecting = describe(&entries[..1], &namespaces, &[], &CodemodeOptions { mcp_types: true, ..CodemodeOptions::default() });
    assert!(before_connecting.contains("CallToolResult<TStructured"), "a host reaching MCP servers declares the types before their tools are known");

    let tight = describe(&entries, &namespaces, &[], &CodemodeOptions { inline_budget: 150, ..CodemodeOptions::default() });
    assert!(tight.contains("Nested tools: PARTIAL - 2 of 4 shown."), "each namespace gets one tool in first: {tight}");
    assert!(tight.contains("## github (2 tools, 1 shown)") && tight.contains("## linear (2 tools, 1 shown)"), "{tight}");
    assert!(tight.contains(PARTIAL_GUIDANCE));
}

/// `models.ask` as a host might give it: the prompt back in capitals, or a failure.
struct Shout;

#[async_trait]
impl HostFunction for Shout {
    fn name(&self) -> &str {
        "models.ask"
    }
    fn description(&self) -> &str {
        "Ask a small model"
    }
    fn signature(&self) -> &str {
        "(prompt: string): Promise<string>"
    }
    async fn call(&self, args: Vec<Value>, _cancel: &CancellationToken) -> Result<Value, String> {
        match args.first().and_then(Value::as_str) {
            Some("fail") => Err("the model is busy".into()),
            Some(prompt) => Ok(Value::String(prompt.to_uppercase())),
            None => Err("models.ask() expects a prompt".into()),
        }
    }
}

struct Taken;

#[async_trait]
impl HostFunction for Taken {
    fn name(&self) -> &str {
        "store"
    }
    fn description(&self) -> &str {
        ""
    }
    fn signature(&self) -> &str {
        "(): Promise<void>"
    }
    async fn call(&self, _args: Vec<Value>, _cancel: &CancellationToken) -> Result<Value, String> {
        Ok(Value::Null)
    }
}

#[tokio::test]
async fn host_functions_are_declared_called_and_recorded() {
    let codemode = tool(vec![]).with_functions(vec![Arc::new(Shout), Arc::new(Taken)]);
    let description = codemode.description();
    assert!(description.contains("Host functions:\n```ts\ndeclare const models: {\n  /** Ask a small model */\n  ask(prompt: string): Promise<string>;\n};\n```"), "{description}");
    assert!(!description.contains("declare function store"), "a built-in's name is not taken");
    let result = run(&codemode, "const a = await models.ask('hi');\ntry { await models.ask('fail'); } catch (error) { text(error.message); }\nreturn a;").await;
    let text = text_of(&result);
    assert!(text.contains("the model is busy") && text.ends_with("\nHI"), "{text}");
    assert_eq!(result.details["calls"][0]["name"], "models.ask");
    assert_eq!(result.details["calls"][0]["status"], "ok");
    assert_eq!(result.details["calls"][1]["status"], "error");
}

#[tokio::test]
async fn scripts_have_no_way_out_but_their_tools() {
    let codemode = tool(vec![]);
    let names = ["setTimeout", "setInterval", "fetch", "require", "process", "std", "os", "WebAssembly", "XMLHttpRequest", "Deno", "Bun"];
    let code = format!("return {:?}.filter((name) => typeof globalThis[name] !== \"undefined\");", names);
    let result = run(&codemode, &code).await;
    assert!(text_of(&result).ends_with("\n[]"), "{}", text_of(&result));
    let tools_object = run(&codemode, "return [typeof tools.then, Object.keys(tools).length, Object.isFrozen(ALL_TOOLS)];").await;
    assert!(text_of(&tools_object).ends_with("[\"undefined\",0,true]"), "{}", text_of(&tools_object));
}

/// Counts the calls that reached it, and refuses the one named `refused`.
struct Counting {
    refused: &'static str,
    runs: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl ToolRunner for Counting {
    async fn run(&self, tool: Arc<dyn Tool>, tool_call_id: String, args: Value, cancel: CancellationToken) -> ToolOutcome {
        self.runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if tool.name() == self.refused {
            return ToolOutcome { result: ToolResult::text("not allowed"), is_error: true, blocked: true };
        }
        DirectRunner.run(tool, tool_call_id, args, cancel).await
    }
}

#[tokio::test]
async fn calls_still_waiting_when_a_refusal_ends_the_script_never_start() {
    let options = CodemodeOptions { max_concurrent_calls: 1, ..CodemodeOptions::default() };
    let codemode = CodemodeTool::new(Arc::new(StaticCatalog::new(vec![Probe::tool("danger", Mode::Echo), Probe::tool("echo", Mode::Echo)])), options);
    let runner = Counting { refused: "danger", runs: Default::default() };
    let result = run_with(&codemode, "await Promise.all([tools.danger({}), ...Array.from({ length: 20 }, () => tools.echo({}))]);", &runner).await;
    assert!(text_of(&result).contains("Script stopped: not allowed"), "{}", text_of(&result));
    assert_eq!(runner.runs.load(std::sync::atomic::Ordering::SeqCst), 1, "only the refused call reached the pipeline");
    let calls = result.details["calls"].as_array().unwrap();
    assert!(calls[1..].iter().all(|call| call["status"] == "cancelled"), "{calls:?}");
}

#[tokio::test]
async fn a_script_cannot_flood_the_host() {
    let codemode = tool(vec![Probe::tool("hang", Mode::Hang)]);
    let flood = run(&codemode, "for (let i = 0; i < 40; i++) text('x'.repeat(1000000));").await;
    assert!(flood.is_error && text_of(&flood).contains("Its output passed 16 MB"), "{}", &text_of(&flood)[..200.min(text_of(&flood).len())]);
    let pending = run(&codemode, "for (let i = 0; i < 2000; i++) tools.hang({});\nawait tools.hang({});").await;
    assert!(text_of(&pending).contains("more than 1000 calls without waiting"), "{}", text_of(&pending));
    let returned = run(&codemode, "return 'x'.repeat(17 * 1024 * 1024);").await;
    assert!(returned.is_error && text_of(&returned).contains("Its output passed 16 MB"));
}

#[tokio::test]
async fn images_the_model_cannot_take_are_left_out() {
    let codemode = tool(vec![]);
    let code = "image('data:image/svg+xml;base64,PHN2Zz4=');\nimage('data:image/png;base64,not base64!');\nimage({ type: 'image', data: 'AAAA' });\n\
                for (let i = 0; i < 11; i++) image('data:image/png;base64,AAAA');";
    let result = run(&codemode, code).await;
    let images = result.content.iter().filter(|part| matches!(part, ContentPart::Image { .. })).count();
    let text = text_of(&result);
    assert_eq!(images, 10, "{text}");
    assert!(text.contains("image/svg+xml is not an image type the model takes"), "{text}");
    assert!(text.contains("its data is not base64"), "{text}");
    assert!(text.contains("application/octet-stream is not an image type"), "{text}");
    assert!(text.contains("at most 10 images"), "{text}");
}

#[tokio::test]
async fn broken_strings_are_mended_and_unreadable_arguments_refused() {
    let codemode = tool(vec![Probe::tool("echo", Mode::Echo)]);
    let mended = run(&codemode, "text('a' + '\\u{1F600}'.slice(0, 1));\nreturn await tools.echo({ query: '\\u{1F600}'.slice(0, 1) });").await;
    let text = text_of(&mended);
    assert!(!mended.is_error, "{text}");
    assert!(text.contains("a\u{FFFD}") && text.contains("{\"query\":\"\u{FFFD}\"}"), "{text}");
    let refused = run(&codemode, "try { await tools.echo({ ['\\u{1F600}'.slice(0, 1)]: 1 }); } catch (error) { return error.message; }").await;
    assert!(text_of(&refused).contains("The arguments of echo could not be read"), "{}", text_of(&refused));
}

#[tokio::test]
async fn every_script_has_a_deadline() {
    let options = CodemodeOptions { timeout: Duration::from_millis(200), ..CodemodeOptions::default() };
    let codemode = CodemodeTool::new(Arc::new(StaticCatalog::new(vec![])), options);
    let spinning = run(&codemode, "while (true) {}").await;
    assert!(text_of(&spinning).contains("timed out after 200 ms"), "{}", text_of(&spinning));
    let longer = run(&codemode, "// @options: {\"timeout_ms\": 60000}\nwhile (true) {}").await;
    assert!(text_of(&longer).contains("timed out after 200 ms"), "an options line cannot ask for more: {}", text_of(&longer));
    assert!(tool(vec![]).description().contains("at most and by default 30 minutes"));
}

#[test]
fn the_host_holds_the_store_to_its_limits() {
    let stored = BTreeMap::from([("old".to_string(), "\"x\"".to_string())]);
    let writes = |pairs: Vec<(&str, Value)>| StoreWrites { set: pairs.into_iter().map(|(key, value)| (key.to_string(), value)).collect(), delete: Vec::new() };
    assert!(check_writes(&stored, writes(vec![("a", json!(1))])).is_ok());
    assert!(check_writes(&stored, writes(vec![("big", json!("y".repeat(MAX_STORE_VALUE_CHARS)))])).unwrap_err().contains("\"big\""));
    let many: Vec<(&str, Value)> = ["a", "b", "c", "d", "e"].into_iter().map(|key| (key, json!("z".repeat(MAX_STORE_VALUE_CHARS - 10)))).collect();
    assert!(check_writes(&stored, writes(many)).unwrap_err().contains("would pass"));
    assert!(parse_writes("[[\"k\", \"{bad\"]]").unwrap_err().contains("\"k\""));
    assert_eq!(parse_writes("[[\"k\", \"1\"], [\"gone\"]]").unwrap(), StoreWrites { set: BTreeMap::from([("k".to_string(), json!(1))]), delete: vec!["gone".into()] });
}
