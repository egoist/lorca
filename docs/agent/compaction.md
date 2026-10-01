# Compaction

A conversation grows until it no longer fits the model's window. `agent::compaction` turns the older part into a structured summary the model writes, keeps the recent part as it is, and hands both back as the context from then on, after pi's compaction.

## Measuring the context

`agent::estimate::estimate_context_tokens(&messages)` is how big a transcript is: the provider's own count from the last assistant message that has one (`input + output + cache reads + cache writes`, the whole request and reply), plus a character estimate (four characters a token, an image counted as 4,800) for what came after it. A message inserted before that assistant message, such as a summary, retires its count. `estimate_message_tokens` and `estimate_text_tokens` are the pieces; `context_tokens(&usage)` is the count one usage stands for.

The window comes from the catalog: `provider.model_info().map(|m| m.context_window)`.

## When

```rust
use agent::compaction::{should_compact, CompactionSettings};

let settings = CompactionSettings::default(); // enabled, reserve 16,384 tokens, keep 20,000 recent, no request limit
if should_compact(estimate.tokens, window, &settings) { /* compact */ }
```

Compact when the context passes `window - reserve_tokens`, so the summary prompt and the reply still fit. Check before a run over a stored transcript, between turns of a run (in `prepare_next_turn`, where the last message's usage is fresh), and after a model answers that the prompt is too long (`agent::retry::is_context_overflow`).

## How

```rust
use agent::compaction::{compact, summary_message, summary_as_llm};
use agent::providers::anthropic::drop_bound_thinking;

let result = compact(provider.as_ref(), &messages, previous_summary, &settings, &cancel).await?;
if let Some(result) = result {
    let mut kept = vec![summary_message(&result.summary, result.tokens_before)];
    kept.extend(messages[result.first_kept..].iter().cloned());
    // The kept messages' thinking is bound to the history the summary replaced.
    drop_bound_thinking(provider.model_id(), &mut kept);
    // kept is the context from now on
}
```

`compact` finds the cut that keeps about `keep_recent_tokens` of recent messages (a cut falls before a user or assistant message, never between a call and its result), asks the model for a summary of everything before it, and answers with the summary, where the kept part starts, the size before, and the summarization requests' usage. The prompt asks for a fixed shape: goal, constraints, progress (done, in progress, blocked), key decisions, next steps, critical context; files the built-in coding tools read or modified are listed at the end. With a `previous_summary` from an earlier compaction the model updates it instead of starting over. When the cut falls inside a turn (one turn is bigger than the recent budget), the turn's prefix gets its own short summary so the kept suffix makes sense. `Ok(None)` means nothing is older than the recent part. With `max_input_tokens` set (a host passes the model's window less twice the reserve), a history longer than one request may carry is summarized in pieces, `chunk_by_tokens` cutting between turns and never between a call and its result, each piece updating the summary of the ones before it.

`summary_message` is the `AgentMessage::Custom { kind: "compaction" }` that carries a summary in a transcript; a `convert_to_llm` hook turns it into what the model reads with `summary_as_llm`, a user message that says the conversation before it was compacted.

`generate_summary` is the summarization request alone, for a harness that cuts differently.

## Inside the conversation

`compact` sends the history again as text under a system prompt of its own, so a provider reads all of it as new input. Between a run's model calls, where the context is exactly what the last request sent, `compact_in_place` asks for the summary as the run's next request instead, and the provider's prompt cache serves everything that request sent:

```rust
use agent::compaction::{compact_in_place, RequestShape};

// The run's own: what goes beside the messages on every model call.
let shape = RequestShape {
    system_prompt: context.system_prompt.clone(),
    tools: context.tools.iter().map(|tool| tool.spec()).collect(),
    cache_points: context.cache_points.clone(),
};
let result = compact_in_place(provider.as_ref(), &shape, my_convert_to_llm, &context.messages, &settings, None, &options, &cancel).await?;
```

`messages` is the context as the loop holds it, an earlier summary first when there is one, and `convert` turns it into what the model reads, as the loop's `convert_to_llm` does; the provider and `options` are the run's, so the thinking settings match too. The request sends the same system prompt, tools, cache points, and messages, then the checkpoint prompt as one more user message, which names the message the kept part starts with (`describe_cut`), so the summary covers everything before the cut, the start of a split turn included, and carries an earlier summary forward. `first_kept` counts from after an earlier summary, as `compact`'s does (`earlier_summary` says how many messages that takes). It falls back to `compact`, and adds the attempt's usage to the result, when the window is unknown, when the context leaves under 4,096 tokens for the reply, and when the model answers with a tool call, a reply the token limit cut off, an error, or anything without the checkpoint's `## Goal`. A history longer than the window has no request to share, and a prompt the provider refused as too long cannot be sent again: both go to `compact`. The harness compacts in place between a run's model calls, unless its hooks' `transform_context` changes the messages: then the run's requests send something else, there is no prefix to share, and the transcript is summarized with `compact`.
