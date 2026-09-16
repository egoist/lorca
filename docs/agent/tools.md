# Tools

## Writing a tool

A tool implements `Tool`: a name, a description, a JSON Schema for its arguments, and `execute`.

```rust
use agent::{Tool, ToolError, ToolResult, ToolUpdateFn};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

pub struct Weather;

#[derive(Deserialize)]
struct WeatherArgs {
    city: String,
}

#[async_trait]
impl Tool for Weather {
    fn name(&self) -> &str {
        "get_weather"
    }

    fn label(&self) -> &str {
        "Weather"
    }

    fn description(&self) -> &str {
        "Current weather for a city."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "city": { "type": "string", "description": "City name" }
            },
            "required": ["city"]
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        args: Value,
        cancel: CancellationToken,
        on_update: ToolUpdateFn,
    ) -> Result<ToolResult, ToolError> {
        let args: WeatherArgs = serde_json::from_value(args)?;
        on_update(ToolResult::text(format!("Looking up {}…", args.city)));

        let forecast = tokio::select! {
            _ = cancel.cancelled() => return Err("Cancelled".into()),
            forecast = fetch_forecast(&args.city) => forecast?,
        };

        Ok(ToolResult::text(format!("{}: {}°C", args.city, forecast))
            .with_details(json!({ "temperature_c": forecast })))
    }
}
```

Register it with the agent or the loop as `Arc<dyn Tool>`:

```rust
options.tools = vec![Arc::new(Weather)];
```

### The trait

| Method | Purpose |
| --- | --- |
| `name()` | The name the model calls. Unique within a tool set. |
| `label()` | A human-readable name for UIs. Defaults to `name()`. |
| `description()` | What the model reads to decide when to call the tool. |
| `parameters()` | JSON Schema for the arguments object. |
| `execution_mode()` | `Some(ToolExecutionMode::Sequential)` makes any batch containing this tool run one call at a time. Defaults to `None`. |
| `execute(tool_call_id, args, cancel, on_update)` | Runs the call. |
| `spec()` | The `ToolSpec` sent to the provider. Built from the methods above. |

### Arguments

`args` is always a JSON object. The loop checks only that: a JSON string holding an object is parsed, `null` becomes `{}`, and anything else fails the call before `execute` runs. Validate against your schema by deserializing, as above. `ToolError` converts from `serde_json::Error`, so `?` reports `Invalid arguments: …` to the model.

If the model streams arguments that are not valid JSON, the tool receives `{ "_raw": "<the text>" }`, and deserializing into your type fails with a message the model can act on.

### Results

`ToolResult` has three fields:

- `content: Vec<ContentPart>`: what the model sees. Text parts reach every provider; image parts reach only providers that send images in tool results (the built-in providers send tool results as text).
- `details: Value`: structured data for your logs or UI. The model never sees it; it travels on `tool_execution_end` and in the `ToolResultMessage`.
- `terminate: bool`: a hint that the run should stop after this batch (see below).

Builders: `ToolResult::text(s)`, `.with_details(value)`, `.terminating()`. `result.text_content()` joins the text parts.

### Errors

Return `Err(ToolError)` on failure. Do not encode failures as successful content. The error text becomes the tool result's content with `is_error: true`, and the model sees it on its next turn. `ToolError` converts from `String`, `&str`, and `serde_json::Error`.

### Progress

Call `on_update` with a partial `ToolResult` as often as you like. Each call becomes a `tool_execution_update` event with `partial_result`. Updates are for display only; the model sees only the final result.

### Cancellation

`cancel` is the run's token. Check it in loops and select on `cancel.cancelled()` around long awaits. After cancellation, return promptly, usually with an `Err`.

### Ending a run from a tool

A tool that completes the task can return `.terminating()`:

```rust
Ok(ToolResult::text("Answer recorded.").terminating())
```

When every tool result in a turn sets `terminate`, the loop does not call the model again for that turn's results. A batch where only some calls terminate continues as usual. Steering and follow-up messages still run. An `after_tool_call` hook can set or clear the flag.

## Built-in coding tools

`agent::tools` has seven tools for working in a directory:

```rust
use agent::tools::{coding_tools, coding_tools_guidelines, coding_tools_snippet};

let tools = coding_tools("/path/to/project");  // read, write, edit, bash, grep, find, ls

let mut system_prompt = String::from("You are a coding assistant.\n");
system_prompt.push_str(&format!("Tools: {}\n", coding_tools_snippet()));
for guideline in coding_tools_guidelines() {
    system_prompt.push_str(&format!("- {guideline}\n"));
}
```

Each tool is also available on its own (`ReadTool::new(cwd)`, `BashTool::new(cwd)`, and so on) for a narrower set, such as read-only tools.

Relative paths resolve against the working directory and `~` expands to the home directory. Absolute paths are used as given: the tools reach whatever the process can. Gate calls with a [`before_tool_call`](hooks.md#gating-tool-calls) hook, or run the process in a sandbox, when that matters.

Output limits are applied so a single call cannot flood the context: 2,000 lines or 50 KB, whichever comes first. A truncated result ends with a bracketed note that tells the model how to get more, such as `[Showing lines 1-2000 of 9120. Use offset=2001 to continue.]`.

| Tool | Arguments | Behavior |
| --- | --- | --- |
| `read` | `path`, `offset?` (1-indexed line), `limit?` (lines) | Returns text from the start of the file or from `offset`, keeping the first lines that fit. JPEG, PNG, GIF, WebP, and BMP files (detected by content) come back as an image part. |
| `write` | `path`, `content` | Creates or overwrites the file, creating parent directories. |
| `edit` | `path`, `edits: [{ oldText, newText }]` | Exact text replacement. Each `oldText` must match exactly once in the original file, and edits must not overlap; all edits are matched against the original, not one after another. Preserves a UTF-8 BOM and CRLF line endings. |
| `bash` | `command`, `timeout?` (seconds) | Runs `bash -c` (or `sh -c` where bash is missing) in the working directory with stdin closed. stdout and stderr are combined and truncated at the tail; when truncated, the full output is saved to a temp file whose path is in the note. Streams the output tail as updates, at most every 250 ms. A non-zero exit, timeout, or abort is an error result that includes the output. Timeout and abort kill the command's process group. |
| `grep` | `pattern`, `path?`, `glob?`, `ignoreCase?`, `literal?`, `context?`, `limit?` (default 100) | Regex search (or literal with `literal: true`) that respects `.gitignore` and skips binary files. Prints `path:line: text`, context lines as `path-line- text`. Lines longer than 500 characters are cut. |
| `find` | `pattern`, `path?`, `limit?` (default 1000) | Glob search that respects `.gitignore` and includes hidden files except `.git`. A pattern without `/` matches at any depth. Directories end with `/`. |
| `ls` | `path?`, `limit?` (default 500) | Directory entries including dotfiles, sorted case-insensitively, directories ending with `/`. |

`write`, `edit`, `bash`, `grep`, `find`, and `ls` put a one-line `summary` in `details` (for example `Edited src/main.rs` or `12 matches for TODO`) for display next to the call.

The truncation helpers are public in `agent::tools::truncate` (`truncate_head`, `truncate_tail`, `truncate_line`, `format_size`) for tools of your own with the same limits.
