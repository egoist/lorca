# Run a bot with Codex

Lorca offers **Runtime → Codex** in the first-bot setup, when creating a bot, and when editing its Details. The first-bot setup can continue with Codex using the Runner's existing Codex login. Codex runs on that bot's assigned desktop Runner. Paired phones send it work through Lorca's encrypted relay.

1. On the Runner, install [Codex CLI](https://learn.chatgpt.com/docs/cli) and run `codex login`.
2. Confirm `codex --version` and `codex app-server --help` work in the Runner's terminal. This adapter uses the experimental dynamic-tool protocol from Codex CLI 0.154.0.
3. In Lorca, choose **Codex** under **Runtime**, then send a message.

Codex loads the Runner's configuration, skills, MCP servers, and sandbox. Lorca provides per-bot model, thinking, speed, and approval choices. Set `LORCA_CODEX_BIN` on the Lorca CLI process to an executable path when Codex is outside its PATH. `CODEX_HOME`, when present in the Runner's environment, selects Codex's configuration and session storage.

The Bot's Provider setting and Lorca's account credentials apply to Runtime → Lorca. A Codex bot uses the Runner's Codex login. The JSON API can set `harness: "codex"` in `bots.create` or `bots.update`, together with optional `model` and `thinking` overrides. `codex_options` stores `speed` (`default`, `standard`, `fast`) and `approvals` (`auto_review`, `user`); omitted options default to the Runner's speed and Approve for me. The apps clear model and thinking overrides when changing Runtime.

## Model, thinking, speed, and approvals

The first-bot setup, New Bot, and bot Details share the Codex controls. Model names and supported thinking levels come from `codex.models` on the assigned Runner, including when a paired phone opens the picker. Reload models refreshes that list. The response contains picker metadata and effective defaults only.

- Model and Thinking offer the Runner's current default plus the available choices. Changing model resets thinking to a supported default.
- Speed offers Default, Standard, and Fast where the model supports it. Fast uses Codex's `priority` service tier and consumes more quota. Standard explicitly requests `default`, including when a previous turn used Fast.
- Approvals defaults to **Approve for me** for both existing and new Codex bots. This sends `approvalPolicy: "on-request"` and `approvalsReviewer: "auto_review"` to Codex. **Ask me** routes native approval requests to Lorca permission cards. The Runner's sandbox settings still apply.
- Settings take effect on the next turn. Returning to Default re-reads the Runner's configuration and replaces previous thread overrides.
- Lorca plugin tools retain their existing permission rules and cards. Codex's native auto-review handles Codex-native approval requests.

## Sessions and controls

- Later messages resume the same Codex thread on the same Runner and workspace. A new workspace or Runner starts a thread seeded with recent Lorca chat text.
- Each turn opens its own App Server process and closes it when finished. Codex persists the thread between turns; background tool processes end with their App Server. A Lorca turn clears the chat's Codex association so switching back seeds a new thread with the intervening chat.
- Replies and activity sync to every paired Device. A message sent during execution steers the current Codex turn when it is still active.
- Stop interrupts Codex and closes its process. The next message resumes the stored thread.
- Command and file approval requests go through the selected Approvals mode. Requests routed to the user appear as Lorca permission cards.
- Codex questions appear in the transcript. Reply in the composer to continue.
- Codex manages context compaction. Provider cost totals and Lorca's manual Compact button apply to the Lorca runtime.
- Lorca team, memory, routine, and discovered plugin tools are available through `lorca_call`. For Lorca plugin actions, exact-tool rules apply; other effectful actions ask for approval. Scheduled runs decline actions requiring user input.

Codex keeps its own local history. Deleting a Lorca chat clears its local thread association and follows Lorca's existing encrypted-chat deletion flow; Codex history remains in Codex storage. Additional permission-profile grants and unsupported server requests are refused by this adapter.

Protocol reference: [Codex App Server](https://learn.chatgpt.com/docs/app-server).

## Verification

`cargo test -p lorca --lib turns::codex` exercises initialization, streaming, resume, steering, approval denial, interruption, and error handling against an in-memory JSON-RPC server. No login is needed.

On a signed-in Runner, `cargo test -p lorca --lib codex_live_read_and_resume -- --ignored --nocapture` runs three native turns: reading a temporary text file, resuming the conversation in a second process with a different model and Standard speed, and calling Lorca's team-list tool after restoring defaults. The first turn explicitly selects a model, thinking level, and Fast speed when supported. All three use Approve for me. It archives its Codex test thread after success. This uses the Runner's Codex account.
