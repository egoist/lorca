# Stop

What Stop in a chat ends, on every Runner it reaches, and what it leaves running.

## Stopping a turn

`chats.stop { chat_id }`, the Stop button and Stop Responding on the Mac and on Windows and Linux, goes through `runtime::cancel_chat`: it cancels every job the chat has on this Device, including one still waiting for the chat lock, and seals a `job_cancel` to each Device that runs one of the chat's turns or lists one in its `machine` blob (`App::cancel_chat`). A Runner that gets a `job_cancel` cancels that job (`App::cancel_job`). The phone has no Stop for a turn; its Running tasks stop one command.

On the Runner, a cancelled turn ends this way:

- **The model call** in flight ends; a reply keeps the words it had shown.
- **Each tool call** in flight gets the cancelled token and returns. One that has not returned 15 seconds later (`lorca_agent::STOP_GRACE`), such as a `read` blocked on a named pipe or a plugin server that never answers, is cut off: the loop stops waiting, and the call's result reads "Cut off: this call was stopped and had not ended 15 seconds later…", saying it may have done part of its work and may still be running. The same bound holds for the check before a call (Auto-review, a permission card). So a turn ends within about 15 seconds of Stop whatever its tools do, and the transcript the next turn rebuilds carries that result, so the bot knows what was cut off. A blocking call keeps its thread until the system call returns.
- **Commands** in the chat stop (`shell::Sessions::stop_chat`), except background ones ([Terminal sessions](terminal-sessions.md#background-commands)): a server or a watcher is meant to outlive the turn. When the stopped turn started commands in the background, or the user sent some there, that still run, the chat gets a notice naming them, "Still running in the background: Serve the docs." (`turns::left_running_notice`), and Running tasks stops them.
- **Coding agents** in the chat stop (`coding::Agents::stop_chat`), here or on their Runner: Lorca's own process tree, or the agent in its Herdr or Luvus pane, and their cards read Stopped ([Coding agents](coding-agents.md#stop-quitting-and-restarts)). A start that Stop cuts off goes on in a task of its own, since a host can take a while to get an agent ready, and the agent it starts is stopped as soon as it runs; the call answers "Stopped".
- **Browser calls** return at once. A profile's browser stays the bot's until its server answers, up to ten seconds, and then closes ([Browser sessions](browser-sessions.md#taking-over)); an open profile otherwise stays open. A run of a skill's recorded steps (`browser_session` `run`) ends at once the same way: no further step starts, and the step in flight keeps the browser until its call answers, up to ten seconds, or closes it ([Running recorded steps](browser-sessions.md#running-recorded-steps)).
- **Codemode scripts** stop, and the calls they left running get ten seconds to wind down. **Plugin calls** are called off at the server with `notifications/cancelled`.
- **Messages** the turn held for its next step no longer wait (`App::unqueue_chat`).
- **A review item** the turn was staging, such as an email or Slack message it was [drafting](drafts.md), is not saved: the staging gets the turn's cancellation, which ends its wait on the plugin's server, and a draft's attachment files go with it.

A routine's run is a turn like any other, and Stop ends it. A routine's check, or a watch's or a calendar's read, that runs inside a turn (Run Now, or the first one as the bot sets the routine up) stops with the turn and leaves the routine's health as it was, since Stop is not a failed check (`routines::check_now`). A stopped run still counts as the run: a [one-time routine and a watch's last run](routine-triggers.md) end their routine all the same, and an event's run does not run again.

Send now cuts a step short the same way ([Agent loop](runtime.md#agent-loop)): a call that does not end within the same 15 seconds is cut off, and the turn reads the user's message.

## What the turn handed off

Every handoff attempt carries the job it belongs to (`HandoffRequest.origin_job_id`): the requesting turn's, or for a turn that continues from a report (`handoff_result`), the job that handed that work off (`handoffs::origin_of`). A follow-up takes the origin of the turn that sends it.

When a job ends cancelled on its Runner (`runtime::run_job_started`), `handoffs::stop_handed_off` stops every attempt this Runner sent with that job's origin that has not reported:

- The attempt gets the requester's cancellation, which wins a concurrent finish, and a recipient on another Runner gets a sealed `job_cancel` ([Handoffs](handoffs.md#inspection-follow-up-and-recovery)); a recipient here is cancelled at once, running or waiting. A Runner that has not started the attempt, offline or behind another turn, finds the cancellation when the job arrives and never runs it.
- The requesting chat gets the report as any other: "Message from ◉ Scout · Stopped before finishing." It starts no continuation; the requesting bot reads it in its next turn.
- The recipient's turn, cancelled in turn, stops what it handed off the same way, so the Stop follows the work from Runner to Runner.

So stopping a continuation stops the rest of the work the first turn handed off, while stopping another turn of the same bot, such as the answer to a later message, leaves earlier handoffs running. A delegated turn stopped in its own chat reports `cancelled` with the same words and wakes nobody either. A turn its runtime limit cancels is not stopped by the user: it reports `blocked`, as [Limits](budgets.md) say, and what it handed off goes on.

## What keeps running

- Background commands, named in the notice and listed in Running tasks.
- A [channel](channels.md#conversations): Stop settles the message whose turn it ended, and the channel takes the next one; its switch pauses it.
- An open browser profile, idle once its call has answered.
- Whatever a cut-off call, or a plugin server that ignores the cancellation, was still doing.
- Draft cards and review items already in the chat: they wait for the user, and one the user sent or approved finishes on its Runner.
- On a Runner that is offline, the turn until it reconnects and reads its `job_cancel` from the relay; its handoffs' cancellations are account-wide records and sync like any other.
