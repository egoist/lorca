# Codemode

`agent::codemode` is a tool that runs model-written JavaScript which calls other tools, after pi's codemode. Only what the script outputs or returns reaches the model. The results of the calls it makes do not, so a script can page through an API, run calls in parallel, loop over what came back, and hand the model the three lines that matter.

```js
const { structuredContent } = await tools.linear__list_issues({ team: "ENG", state: "open" });
const counts = await Promise.all(
  structuredContent.issues.map(async (issue) => {
    const { content } = await tools.linear__list_comments({ issue: issue.id });
    return { id: issue.id, comments: JSON.parse(content[0].text).comments.length };
  }),
);
return counts.sort((a, b) => b.comments - a.comments).slice(0, 3);
```

The feature is on by default (`codemode`); it compiles QuickJS from C through `rquickjs`.

## The tool

```rust
use std::sync::Arc;

use agent::codemode::{CodemodeOptions, CodemodeTool, StaticCatalog};

let catalog = Arc::new(StaticCatalog::new(my_tools));
let codemode = CodemodeTool::new(catalog, CodemodeOptions::default());
tools.push(Arc::new(codemode));
```

The tool takes one argument, `code`, the body of an async function: top-level `await` and `return` work. The code may start with an options line:

```js
// @options: {"max_output_tokens": 2000, "timeout_ms": 60000}
```

`max_output_tokens` is the budget for the script's output (default `CodemodeOptions::max_output_tokens`, 10,000, and at most 50,000). Longer output keeps its start and end, and the whole text goes to a temp file the result names. `timeout_ms` shortens the deadline for the whole script, tool calls included, which is `CodemodeOptions::timeout` (30 minutes) at most; the run's cancel token stops a script at any time.

The result starts with `Script completed`, `Script failed`, or `Script stopped`, the wall time, and then the output in the order the script produced it. A failure appends the error with its stack (`codemode.js:<line>`, which matches the script as written) and the calls made before it, which are not undone. A failed script is an error result (`ToolResult::is_error`) that keeps its partial output. `details.calls` lists the script's calls (name, arguments cut for display, status, duration, error), the first 256 of them.

A host can run a script itself, outside a model's turn: `codemode.run_script(call_id, code, cancel, &runner)` runs it as a call of the tool would, with `runner` (a `ToolRunner`) running its calls, and answers with a `ScriptRun`: the `result` a model would have read, and `returned`, the JSON the script returned when it finished and returned anything but `undefined`. A runner that answers a call with `blocked` ends the script, as a refusing hook does.

## What a script has

- `tools.<name>(args)`: a promise per tool. A name becomes a JavaScript identifier, with characters that are not valid in one turned into `_`; `tools["my-tool"]` works too, and so does a name the script found with a search after it started.
- A tool that declares `output_schema()` resolves to its `structured` output, also for an error result that carries one. Any other tool resolves to its text. A call that fails, or whose arguments fail the schema check, rejects with an `Error` carrying the tool's error text.
- `text(value)` and `console.log(...)` append output; `image(dataUrlOrMcpImage)` appends an image; `return value` appends the value; `exit()` ends the script successfully.
- `store(key, value)` and `load(key)` keep JSON values across scripts through a `CodemodeStore` the host persists (`with_store`). A value may take 256 Ki characters of JSON and all of them together 1 Mi; a failed script writes nothing.
- `ALL_TOOLS`, `await searchTools(query, { limit, namespace })` (BM25 over names, descriptions, schemas, and namespaces by default), and `await describeTool(name)`.
- The host's functions (`with_functions`, below).
- Nothing else: no timers, `fetch`, modules, file system, or network. `eval` and `Function` only make more code inside the same VM.

A script that waits on a promise nothing can settle fails at once instead of hanging.

The host holds a script to limits its VM's memory cap does not cover. Past 16 MB of output, or with more than 1,000 calls it started and has not seen finish, the script stops. An image must be PNG, JPEG, GIF, or WebP, in valid base64, of at most 5 MB, and ten at most; any other is left out with a note in the output. A call whose arguments are over 8 MB, or are not JSON the host can read, rejects without running. Strings are made well-formed before they cross, so a lone surrogate becomes U+FFFD. `store()` writes over their limits fail the script, whatever the script did to the prelude's own checks.

## Calls go through the loop

The loop calls `Tool::execute_with`, which hands the tool a `ToolRunner`. Codemode runs every call through it, so a script's call gets the same treatment as a call the model makes: the tool's argument shim, coercion and the schema check, `before_tool_call`, and `after_tool_call`. The hooks see the script's own call as `ctx.parent`, and each nested call has the id `<codemode call id>/<n>`. Nested calls send no events of their own; the codemode tool reports them in its updates (`details.calls`) and its result.

A call that `before_tool_call` blocks ends the script. Calls still waiting for their turn never start and reach no hook, calls still running are cancelled, and the result says `Script stopped:` with the hook's reason, carrying the block's `terminate` hint. A refusal is final: the script cannot catch it and go on with the rest of a batch. The loop runs no hook for a call that was cancelled before it started.

A tool that declares an output schema hands the script its `structured` output, so an `after_tool_call` hook that changes what a script sees sets `AfterToolCallResult::structured`, not only the content.

Calls run in parallel, up to `max_concurrent_calls` (8) at once. A tool whose `execution_mode()` is `Sequential` runs alone, so two calls that may ask a person never wait at the same time.

## The description

`CodemodeTool::new` renders the description once, from the catalog, so it stays the same for as long as the tool lives and does not break a prompt cache. It lists:

- what a script has, and the limits;
- the shared MCP result types, when an MCP tool is callable or `CodemodeOptions::mcp_types` is set, with how to read a `CallToolResult`;
- the host's functions as TypeScript;
- the tools the model can also call directly (`Exposure::Direct`), by name only;
- every other tool as a TypeScript declaration built from its JSON Schemas, grouped by namespace, within `inline_budget` estimated tokens (3,000). Each round, every group places its cheapest remaining tool, so each namespace is represented before any is complete. `Exposure::Deferred` tools are never listed. The listing says whether it is complete, and every namespace is named with its tool count, including one with no tools known yet.

A tool whose output schema is shaped like MCP's `CallToolResult` (a `content` array of objects and a boolean `isError`) renders as `Promise<CallToolResult<T>>`, with `T` from its `structuredContent` schema.

## Catalogs

A `Catalog` says what scripts can reach:

| Method | Purpose |
| --- | --- |
| `entries()` | The tools scripts can call now, each an `Entry { tool, exposure, namespace }`. The description and `ALL_TOOLS` list these. |
| `namespaces()` | Every group, with a description, including ones whose tools are not known yet. |
| `find(name, cancel)` | A tool `entries()` did not have, by its name or identifier, such as one of a server that connects on first use. Defaults to none. |
| `search(query, namespace, limit, cancel)` | `searchTools()`. Defaults to BM25 over `entries()`. |

`StaticCatalog` is a fixed list. Lorca's CLI implements its own over the plugins installed on a Runner, whose tool lists it keeps on disk so a turn lists them without starting a server.

## Host functions

A `HostFunction` is a function scripts call by name, such as `models.ask(prompt)`: `name()` (an identifier, or `ns.member` to group it under a namespace object), `description()` and `signature()` for its declaration, and `call(args, cancel)`, which gets every argument as an array and returns a JSON value or an error the script's promise rejects with. The script's call list records each call. A name taken by a built-in helper is left out.

## The sandbox

Each script runs in a fresh QuickJS VM on a thread of its own, so a spinning script never blocks the async runtime. The VM's only way out is the bridge its prelude holds: tool calls, global calls, and output cross it as JSON text. Its heap is capped at `memory_limit` (256 MiB), and an allocation past it fails inside the script as `out of memory`; deep recursion throws a catchable error before the native stack runs out. Stopping sets an interrupt flag the VM polls and closes the reply channel, which ends the thread whether the script spins or waits on a call.

pi runs QuickJS compiled to WebAssembly, whose linear memory keeps the interpreter apart from the host process. Here the interpreter shares the host's process: the sandbox makes sure a script can do nothing its tools cannot, and the tools carry the permission checks.
