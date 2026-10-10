# Budgets and connector limits

The bot's Runner enforces limits in `crates/cli/src/budgets.rs` and its `runtime` module. A limit set carries optional `max_usd`, `max_tokens`, `max_runtime_secs`, `max_retries`, and `max_connector_calls`. An omitted limit is none; zero allows nothing in that category. A retry limit of zero still admits the first request. Run time includes checks, model calls, review, connection waits, backoff, and questions to the user, and is at most one year.

## Scopes and accounting

Limits apply to three scopes:

- `chat`: the limits each new turn in that DM starts with. Changing them affects later turns.
- `job`: one turn, under its existing Job id. Turns in a chat with limits get a record with the chat's limits copied in; a turn in a chat without limits has no record.
- `task`: all runs of a [durable task](tasks.md) together, once the user sets limits on it from the task sheet. A task's run counts only toward the task. When the task's limits stop a run, the run's outcome blocks the task with the limit's reason (`tasks::execution::budget_block_reason` reads the task's snapshot), and the task sheet's Resume opens Limits, since a new run would only be refused again.
- `routine`: all runs of a routine and its checks together, until the user resumes it with fresh limits. A routine's run counts only toward the routine.

A `message_bot` handoff is the target bot's turn and counts toward the target's DM limits. An [event's](event-triggers.md#durable-ordering-and-execution) turn counts toward its DM's limits, or its routine's when its subscription targets one.

The ledger keeps only what someone may look at or resume: limits the user set, a turn or routine that stopped, and work in flight. A turn that finishes on its own drops its record. A new turn in a chat replaces a stopped one there, since the user moved on. Records for a deleted chat or routine, other Runners' projections once unpaired, and resume receipts older than a week are pruned. Work with no limits touches no ledger.

`turns::run_job` admits a `BudgetContext` and runs all of the turn's work inside it. `routines::run_now` and the scheduler refuse a stopped routine. `routines::run_check` adds the routine's scope to the context of the turn it runs in, so check-model calls and plugin calls count once. The context follows inference, `models.ask`, Auto-review, compaction, and the memory flush. `providers::provider_for` wraps every adapter, and `ModelsAsk::new` and `mcp::turn_catalog` capture the context for calls made through a codemode bridge.

Before a model request starts, the Runner reserves estimated input and bounded output against every scope the work counts toward. Parallel side-model calls share those reservations. Input uses the agent's text and image estimates plus the tool schemas; the output cap shrinks to the remaining token or spending allowance, and Anthropic's budget-mode thinking shrinks inside it. Spending admission uses the highest published input, cache-write, and output rate, so a cached request can settle for less. Provider-reported usage replaces the reservation when the response completes; a provider that reports none is charged its estimated input and observed output. Without limits, adapters keep their own reply defaults.

Every adapter HTTP attempt passes the admission hook (`RequestHooks::before_request`), including the adapter's own retries, and the agent loop's retry after a retryable error charges the retry limit too. A stop from Send now or an overflow's compaction is not a retry.

A call that already ran is recorded, not turned into a stop: the next model or tool call that finds a limit used up is refused, and that refusal stops every scope the work counts toward. Tool calls check the limits before review and again after it, so a review that spends the rest cannot let one more call through. The run-time deadline cancels the same token the provider, tools, and compaction use. A refused turn ends with a notice in the chat: "Stopped at the token limit. Raise it in Limits to resume."

On restart the Runner counts open reservations and elapsed run time as used. A turn that was running is marked `interrupted` and waits for the user, since it may have done part of its work; a routine runs again on its schedule. A restart never replenishes a limit.

## Pricing

Accounting keeps three categories apart:

- API spending uses published API rates, including zero for a catalogued free model.
- ChatGPT, Grok, and OpenCode Go turns are subscription estimates: what the work would cost at API rates. They count toward a spending limit.
- A custom provider or an uncatalogued model has an unknown price. Its calls and tokens count, and a spending limit alone refuses it; a token or run time limit bounds it instead.

Chat usage carries `api_cost_usd`, `subscription_estimate_usd`, `unknown_price_calls`, and the `pricing_kinds` seen. The inspector's Spent row reads "$0.42 · 18 turns", "$0.42 est. · 18 turns" for an estimate (its tooltip says what that means), or "Price unknown · 18 turns"; mixed kinds join with "+". A price is never shown as $0.00 for lack of one.

## Storage, protocol, and resuming

The Runner stores the ledger (limits, usage, reservations, resume receipts, and a stopped turn's Job) in the `runner_limits` SQLite table as XChaCha20-Poly1305 ciphertext under the account DEK, with the purpose as associated data. A record that does not decrypt refuses admission. Connector limits, call windows, and cooldowns use a separate purpose in the same table. Forgetting the identity clears the table and both caches.

Snapshots carry the scope, its limits and usage, `state` (`ready`, `running`, `complete`, `budget_exhausted`, or `interrupted`), `reached` (`usd`, `tokens`, `runtime`, `retries`, `connector_calls`, or `unknown_price`), and the notice's `reason`. They reach paired Devices only inside the Runner's encrypted `machine` blob, pushed when work starts, ends, stops, or its limits change, not on every model call. The local CLI gives the apps `bootstrap.budgets` and `budgets.changed { budgets }`. The stopped turn's Job stays on its Runner. Limits live on the Runner they were set on; a bot moved to another Runner has none there.

| Method | Parameters and result |
| --- | --- |
| `budgets.set` | `kind` (`chat`, `job`, `task`, or `routine`), `id`, `limits`, `runner_id?`, `bot_id` for a turn, `chat_id?`; sets the limits and keeps what was used |
| `budgets.resume` | `kind`, `id`, `request_id`, `runner_id?`, `renew?`, `run?`; resumes stopped work |

Another Device's call goes to the bot's Runner through the existing sealed request path, and the Runner checks the bot is its own. Raising a limit keeps the stop until the user resumes. `renew: true` grants the limits again in full. A repeated request id returns its receipt and neither renews nor starts work again. A scope still running refuses to resume. `run: true` continues a stopped turn from its transcript under the same Job id (a delegated turn goes on as its [handoff](handoffs.md)'s next attempt), returns an event's delivery to its subscription's inbox, which admits it again under the same Job id, starts a new run of a routine through `run_now`, or of a task through `tasks.run` at its current revision, so the task's owner checks it as for any run. A plugin call the turn made is never sent again from the ledger.

## In the apps

The macOS app and the Windows and Linux app show the same things, and the phone the same in its own idiom ([Phone app](phone-app.md)):

- **The DM inspector**: a Limits row in Runs with, after Spent. Its value is the chat's limits ("$2.00 · 200k tokens", or None), or Limit reached / Interrupted in orange when the chat's newest turn stopped. The whole row opens the Limits sheet.
- **The Limits sheet**: Spending (USD), Tokens, Run time (minutes), Retries, and Plugin calls, each empty for no limit, with what the work used beside each and the reached one in orange. When work stopped, a card above the form names the limit and what it used of it, with Resume. Resume saves the form as the turn's and the chat's limits and goes on, provided the limit it reached went up; otherwise it asks before granting the limits again in full. Save is the sheet's default button.
- **Tasks**: the task sheet's card ends with a Limits row, Limit reached in orange when the task's limits stopped it; then the card's Resume opens Limits.
- **Routines**: the routine sheet shows Limit reached as its state, ahead of any [health](routines.md#missed-occurrences-and-recovery) state, with what it used under it, has a Limits row in Schedule that opens the routine's Limits sheet, and keeps Run Now off until it is resumed. Its inspector row says Limit reached too.
- **Plugins**: the plugin sheet's Status has a Call limit row ("60 a minute", or "Waiting until 5:40 AM" while the service asked to slow down) that opens the Call Limit sheet: calls every n seconds and at once, with Applies to (this account, or all of the service's accounts) only for an account whose service has others.

On the Mac these are `BudgetViewController`, `ConnectorLimitsViewController`, and `DisclosureRow`; in `desktop/` they are `sheet_budget.go`, `sheet_connector_limits.go`, `disclosureRow`, and `model/budgets.go`. Both apps' demo data has limits, a turn stopped at its token limit, and a routine stopped at its spending limit. On the phone, a plugin's screen sets its account's call limit from menus; the service-wide limit is set on a computer.

On the phone (`app/chat-info/limits.tsx`, `src/ui/limits.ts`): Details' Runs with ends with a Limits row: the DM's limits, or Limit reached in orange when its newest turn stopped. A task's screen and a routine's screen have the same row; a routine that stopped leads its row and its screen with Limit reached and drops Run Now. A routine's screen and its row in Details are in [Routines](routines.md#the-apps). The row slides in the Limits screen: a stopped card with Resume, the five limits as fields (empty for none, dollars and minutes), and what the work used, with the limit it reached in orange. Save in the toolbar sets the limits; Resume saves them, then goes on, asking first when it would grant the limits again in full. `budgets` comes from the snapshot and `budgets.changed`.

## Shared connector admission

`crates/cli/src/connector_limits.rs` guards actual MCP tool and resource calls. All bots and all servers of one installed account share its account bucket. A named account id has the form `<service-id>-<32 UUID hex>`, so accounts of one service keep separate buckets and share the service's; a single-account plugin's service is its manifest id. Account labels never choose a bucket.

Both buckets default to 60 calls per 60 seconds and four at once. `connector_limits.get` and `connector_limits.set` take `runner_id?`, `plugin_id`, and `scope` (`account`, the default, or `service`); Set takes `limits { max_calls, window_secs, max_concurrency }`, at most 10,000 calls in a window of one second to a day and 256 at once. Get answers the limits, `retry_at` while a cooldown is in force, and the account's `service_id`. Zero calls or concurrency pauses calls with an actionable error. Saving a limit ends a cooldown.

Admission checks the turn's limits, then waits for both buckets, then counts the call toward the turn, so a turn out of plugin calls takes no slot from the other bots. A waiting call observes cancellation, and a completed, failed, or cancelled call releases concurrency through its permit. Call windows persist, so a restart does not reset the shared rate.

The MCP HTTP client (`connector_limits/http.rs`, rmcp's reqwest client with an observer) reads `retry-after-ms`, a numeric or HTTP-date `Retry-After`, and an exhausted `x-ratelimit-remaining` with `x-ratelimit-reset` as a Unix time or seconds from now. A 429 without guidance waits one second. A failed result's `retry_after_ms`, `retryAfterMs`, `retry_after`, or `retryAfter` counts too. Guidance holds the account's later calls, handshakes, and reconnects for at most a day, and never shortens one already in force. A call is never replayed because of it: the failed call reports its outcome to the bot. A 404 for an expired session re-initializes and sends again, since the server did not run that request; reqwest retries only protocol-level refusals, which the server never processed. MCP OAuth tokens and variables stay on the Runner in its authorization manager and plugin store.

