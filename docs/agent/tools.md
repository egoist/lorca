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
| `prepare_arguments(args)` | A shim over the raw arguments before the schema check, for a tool that accepts an older or looser shape. Defaults to returning them as they are. |
| `execute(tool_call_id, args, cancel, on_update)` | Runs the call. |
| `spec()` | The `ToolSpec` sent to the provider. Built from the methods above. |

### Arguments

`args` is a JSON object that already passed `parameters()`. Before `execute` runs, the loop:

1. Parses the streamed argument text. A string with raw control characters or a stray backslash is repaired; text cut off mid-value keeps the members that were complete. Anything else is `{}`.
2. Hands the object to `prepare_arguments`.
3. Coerces the values the way models get them wrong: `"3"` becomes `3` for an integer field, `"true"` becomes `true`, a number becomes its string for a string field, and `null` for an optional field that does not take null is dropped as "not given".
4. Validates against the schema. A failure becomes an error result the model sees, listing each field's problem and the arguments as received, and `execute` is not called:

```
Validation failed for tool "read":
  - path: "path" is a required property
  - limit: "lots" is not of type "integer"

Received arguments:
{
  "limit": "lots"
}
```

Deserialize into your type inside `execute` as above; with the schema enforced first, that only fails for shapes the schema does not describe. `ToolError` converts from `serde_json::Error`, so `?` reports `Invalid arguments: …` to the model.

A message the model's output limit cut off (`stop_reason` `Length`) never runs its tool calls at all: each gets an error result asking the model to re-issue the call with complete arguments.

### Results

`ToolResult` has three fields:

- `content: Vec<ContentPart>`: what the model sees. Text parts reach every provider. Image parts reach the Anthropic Messages adapter as image blocks of the tool result, and the OpenAI-compatible adapter as a user message right after the tool message; a model that does not take images gets a note in their place.
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
| `bash` | `command`, `timeout?` (seconds) | Runs `bash -c` (or `sh -c` where bash is missing) in the working directory with stdin closed and the login shell's environment (`login_shell::command`). stdout and stderr are combined and truncated at the tail; when truncated, the full output is saved to a temp file whose path is in the note. Streams the output tail as updates, at most every 250 ms. A non-zero exit, timeout, or abort is an error result that includes the output. Timeout and abort kill the command's process group. |
| `grep` | `pattern`, `path?`, `glob?`, `ignoreCase?`, `literal?`, `context?`, `limit?` (default 100) | Regex search (or literal with `literal: true`) that respects `.gitignore` and skips binary files. Prints `path:line: text`, context lines as `path-line- text`. Lines longer than 500 characters are cut. |
| `find` | `pattern`, `path?`, `limit?` (default 1000) | Glob search that respects `.gitignore` and includes hidden files except `.git`. A pattern without `/` matches at any depth. Directories end with `/`. |
| `ls` | `path?`, `limit?` (default 500) | Directory entries including dotfiles, sorted case-insensitively, directories ending with `/`. |

`write`, `edit`, `bash`, `grep`, `find`, and `ls` put a one-line `summary` in `details` (for example `Edited src/main.rs` or `12 matches for TODO`) for display next to the call.

The truncation helpers are public in `agent::tools::truncate` (`truncate_head`, `truncate_tail`, `truncate_line`, `format_size`) for tools of your own with the same limits.

### Commands in a terminal

`coding_tools` runs `bash` as pi does: pipes, nothing on stdin, and a call that lasts as long as the command. That suits a person at a terminal, who answers a prompt there. A host whose commands run where nobody watches builds the tools with a store for terminal sessions instead, and gets two more tools:

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use agent::tools::{coding_tools_with_sessions, session_tools_snippet, BashSession, BashSessions};

#[derive(Default)]
struct Sessions(Mutex<HashMap<String, Arc<BashSession>>>);

impl BashSessions for Sessions {
    fn insert(&self, _call_id: &str, session: Arc<BashSession>) {
        self.0.lock().unwrap().insert(session.id().to_string(), session);
    }
    fn get(&self, id: &str) -> Option<Arc<BashSession>> {
        self.0.lock().unwrap().get(id).cloned()
    }
    fn remove(&self, id: &str) {
        self.0.lock().unwrap().remove(id);
    }
}

// read, write, edit, bash, grep, find, ls, bash_input, bash_output
let tools = coding_tools_with_sessions("/path/to/project", Arc::new(Sessions::default()));
```

On macOS and Linux, `bash` then starts each command on a pseudo-terminal of its own, as its controlling terminal (`setsid`, `TIOCSCTTY`), so programs that ask on `/dev/tty` (`sudo`, `ssh`, `getpass`) ask there. The terminal is 80×24 with `TERM=xterm-256color`, `PAGER=cat`, and `GIT_PAGER=cat`, and echo is off, so typed input never comes back as output. The child ignores SIGHUP, so a background job outlives the shell as under `nohup`. Output is drained into the session whether or not a call is reading, and results carry text: `agent::tools::sanitize::terminal_text` drops escape sequences and control characters and applies carriage returns and backspaces. The temp file with the full output keeps the raw bytes.

A call returns when the command exits, with the same result as on pipes, or while it still runs: 2 s after it stops on an open line that reads like a question (`BashSession::prompt` has the heuristic), or after 20 s with no output (`bash_session::WAITING_AFTER`; `BashTool::waiting_after` changes it). That result is the output so far, a note that the command waits, and its session id, which `details["session_id"]` also carries. `timeout` still kills the command when it runs that long; cancellation kills its process group.

| Tool | Arguments | Behavior |
| --- | --- | --- |
| `bash_input` | `session_id`, `text`, `enter?` (default `true`) | Types the text into the session's terminal, then Return unless `enter` is false. Control characters are keys: `\u0003` is Ctrl-C, `\u0004` Ctrl-D. Returns what the command printed since the model last read it, once it ends, asks again, or goes quiet. |
| `bash_output` | `session_id`, `wait_seconds?` (default 0, at most 300) | Returns what the command printed since the model last read it, waiting up to `wait_seconds` for it to end or ask. |

A result on a command that ended reads the same as `bash`'s: output, then `Command exited with code N` (an error), `Command terminated by signal N`, or why it was stopped. The tool that reads an end calls `remove`.

The host decides how long a session lives, and must end it: `BashSession::stop(reason)` kills the command's process group, closes its terminal, and gives the next call that reason. Stop it when the user stops the run, after a while without output, and when the process exits. Dropping the last handle to a session kills whatever still runs. Beyond the tools, a session offers what a host's own UI needs: `write` to type into it, `prompt` and `last_lines` for what it shows, `end` and `ended` for how it ended, and `changes` to follow it.

On Windows `bash` keeps pipes even with a store, its description says input is not available, and `coding_tools_with_sessions` leaves out the two tools. `session_tools_snippet()` is their one-line summary for a system prompt.
