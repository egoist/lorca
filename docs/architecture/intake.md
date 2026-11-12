# Intake

Intake has a [channel](channels.md) decide by itself which messages are work, in place of a turn per message. It is how a bot becomes a colleague in a group: it reads everything the channel takes, stays quiet about most of it, takes on what the owner's rules cover or what someone the owner lists asks for, and speaks only when someone must act or can use a result. Each piece of work is a [durable task](tasks.md) in a chat of its own, so many run at once. `crates/cli/src/intake.rs` holds its types, the gate, and the Mandate, `intake/assess.rs` the assessment and what code checks of its answer, `intake/decisions.rs` the record, and `intake/run.rs` the flow.

The cost it manages is people's attention. Every message, question, and setting has to give more than it takes.

```
Telegram or Slack ──▶ reader ──▶ inbox ──▶ gate (code) ──▶ assessment (small model, no tools)
                                             │                  │
                       a later message ◀─────┤                  ├─▶ accept ──▶ Task + Mandate ──▶ run in its work chat
                       of a Task, by code    │                  ├─▶ attach / stop
                                             ▼                  ├─▶ reply ──▶ the channel's ordinary turn
                                         recorded               └─▶ ignore, would accept ──▶ recorded
```

## The owner's words

A channel's `ChannelSpec.intake` is `{ mode, people, rules }`, in the subscription, encrypted on the Runner, and carried in its `machine` blob (`ChannelStatus.intake`) so every Device shows it. A channel without one gives each message it takes a turn, as [Channels](channels.md#conversations) describes.

- `mode`: `off` gives each message a turn, as without intake, and keeps the people and rules for later; `observe` records what the bot would take on when nobody asked it, and does nothing; `auto` takes it on. A channel starts in `observe`, so the owner reads what it would have done before it does anything.
- `people`: who may ask the bot for work directly, as `{ id, name }` by the service's own user id (Telegram's number, Slack's `U…`). A name can be changed by its owner, so only the id counts. A person's identities on different services are never mapped to each other.
- `rules`: at most 16 `{ id, name, enabled, instruction }`: a kind of work and how far it goes. Process and communication it asks for hold for the Tasks it accepts, so a rule can name a reviewer, or ask for silent work: "When someone reports a bug here, try to reproduce and fix it. Say nothing unless you confirmed it and have a small fix."

`IntakeSpec::version` hashes all three. Each Mandate records it, and a change discards an assessment in flight, whose messages wait for a fresh one.

`events.set_intake { id, intake, runner_id }` replaces it from the apps, `null` removing it, through a sealed request to another Runner as every `events.*` is. The bot's `channels` tool takes `intake` in `create` and `edit`, only in a turn the user started: a message from outside can never open or widen what the bot takes on, as it can't for chats.

## The gate

Code decides a message's strength from who wrote it and whether it addressed the bot, before any model is asked (`intake::gate`). Words can't reach this. A message addresses the bot when it is a direct message, a mention, a reply to the bot, or carries a hashtag the channel listens for. The readers leave out bots' messages. A message someone forwarded is not their own words, so it counts as written by nobody listed. What the channel takes at all is still its `listen` filter and allowed chats; reading a whole group takes `every`, and on Telegram privacy mode off.

| Sender | Addressed | With an enabled rule | With none |
| --- | --- | --- | --- |
| listed | yes | assessed, `explicit` | assessed, `explicit` |
| listed | no | assessed, `inferred` | recorded and left alone |
| anyone else | yes | assessed, `inferred` | the channel's ordinary turn |
| anyone else | no | assessed, `inferred` | recorded and left alone |

`explicit` work needs no rule and goes ahead in either mode. `inferred` work needs a matching rule and `auto`.

## The assessment

The inbox hands intake a batch: one person's waiting messages in one conversation, in order, at most eight, up to anyone else's next word. A burst reads as one exchange and costs one request, and since a batch has one sender, nobody's words borrow another's standing. One batch per channel runs at a time, as one request to the small model of the bot's provider (`providers::chat_reviewer`), with no tools and no state, cut off after 90 seconds, its cost counted in the conversation. Nothing it reads can act, because it has nothing to act with.

The system prompt holds what the model does and the owner's task and rules, which change rarely, so a provider caches them. The request holds the message's standing by the gate, the Tasks this sender may point at inside `<tasks>`, and `<conversation>`: every message of the batch, whatever has come since and however long, and before them the newest messages up to forty and 16,000 characters. Each one's words have angle brackets replaced so none can close the fence, and a person is `Person "name"`, the name in quotes, so nobody passes for `Owner` or `Bot` by what they call themselves. The batch's images follow when the small model takes images. The model answers one JSON object in `<decision>`: `decision` (`accept`, `attach`, `stop`, `reply`, `ignore`), a judgment of each enabled rule, `direction` (`work_requested`, `target_settled`, `needs_design_decision`), `evidence` quoted from the conversation, a `title` and `scope` for an accept, a `task_id`, and a `reason`.

A message someone forwarded goes alone, as someone else's words. A message can talk the model into any answer, so the answer proves nothing by itself (`assess::outcome`):

- Every enabled rule is judged exactly once, or the assessment failed. An unknown decision or an unreadable reply failed too.
- `accept` of `inferred` work needs `direction` settled, a matched rule, and evidence that is really in what people wrote, spacing and case aside; the bot's own words don't count. Short of any of these it is `ignore`, with why. In `observe` a good one becomes `would_accept`.
- `attach` and `stop` must name a Task this sender was shown, and `stop` a Task they may stop. Someone entitled to a Task (who asked for it, or is listed) sees the channel's Tasks, ended ones too; anyone else sees only what is open in their own conversation, so another chat's work stays out of their reach and what they could learn of it.
- `reply` needs a message that addressed the bot.

**Failures.** A model that does not answer holds the channel's queue and drops nothing, as do the conversation's limits: its messages wait in order, trying further apart up to 15 minutes, and after three tries a quiet blocker in [Attention](attention.md) says so, until the model answers again. An answer that can't be used may be someone's doing, and so may a request the provider turns down (a 400, a content filter), so neither holds up anybody: it tries once more at once, then records `unavailable`, and only a listed person's lost request raises a blocker, which stays until the owner resolves it. An assessment only reads, so a restart simply puts its batch back in the queue; a batch whose Task was made before the crash finds that Task, whatever a model would answer now, since a Task's id follows from the batch's first message. A change to the rules or a pause during a read discards the read.

A message nobody addressed to the bot is recorded and left alone once it is a day old, whatever held it up. A channel holds at most twelve open `inferred` Tasks: past that, what a rule would accept is only recorded, so nobody can turn a chat into a work queue.

## Decisions

`intake_decisions` in `lorca.sqlite3` keeps each decision encrypted with the account key (associated data `intake_decision`), with only its id, channel, time, and whether it waits as columns; a channel's newest 500 for thirty days, and deleting the channel deletes them. `events.decisions { id, limit, runner_id }` reads them from the channel's Runner, so they stay out of the `machine` blob, which would otherwise change per message. A decision is `accepted`, `would_accept`, `attached`, `stopped`, `replied`, `ignored`, or `unavailable`, with the messages it decided, their sender and user id, its strength, the rules' judgments, its evidence, and its Task. Titles, reasons, and quotes come from outside Lorca, and the bot's `channels` `decisions` action marks them as data; it runs only in a turn the user started, since the record spans every chat.

## The Mandate

An accept makes a Task through `tasks::create_mandated`, the only way to set its `mandate`. No `tasks.*` request or tool edit reaches it, and it travels with the Task to every Device. It records why the work may be done: the channel and its task as they stood, the conversation and message, the sender, the strength, the rules matched in the owner's words as they stood, a listed person's whole burst word for word, the assessment's `scope`, `reason`, and evidence, and the version it was accepted under. Only a new accept widens it.

**Authority stays apart from outside words.** A mandated run is unattended as a channel's turn is ([Auto-review](auto-review.md#what-the-review-reads)), and Auto-review reads its task as the user's own words. So that task (`intake::task_event`) holds only the channel's task, and either the rules as they stand now, or, for `explicit` work, what the listed person wrote, by their user id: a name is someone's choice, and this text counts as the owner's. It never holds the assessment's `title` or `scope`, which outside words shaped: those reach the run as data in its closing note, marked as granting nothing. An assessor talked into a wide scope gets a Task with that goal and no more authority than its rule gives. What the run does still passes Access and Auto-review, and what would ask is staged in the [review queue](review-queue.md).

`Mandate::holds` is checked again before a later message resumes a Task: an `explicit` one while its sender stays listed, an `inferred` one while the channel is in `auto` and its rules stay on. A rule whose words changed still holds.

## A Task's work chat

Each mandated Task runs in a chat of its own, `Mandate.work_chat_id`: a hidden direct chat whose `channel` carries `task_id`, speaking from the same account and thread as its conversation. Turns in one chat run one at a time and share its transcript, so without this a group's Tasks would queue behind each other and mix their context. With it they run at once, and each reads only its own work. Every app leaves work chats out of its lists; the Task's sheet opens one, and search finds it. Nothing there marks anything unread or alerts, unless a bot reports something urgent. A finished Task's work chat goes after 30 days, since the roster carries it to every Device.

A Runner runs at most four mandated runs at once, with one more for someone waiting on a Task; the others stay `queued` and start oldest first. Each Task gets [limits](budgets.md) of its own (four million tokens and two hours across its runs, set again for a new run after it finished), and one that reaches them stops for the owner, who resumes it from its Limits.

The batch's messages are copied in, and the conversation's ten lines before them are inlined in the first run's closing note, as data; `recall` finds the rest. What the run sends through the account's tools lands in the work chat and in the conversation, whose transcript the next assessment reads and the user follows (`channels::sent`). Its reasoning and its tool calls stay in the work chat. A reply from a work chat is refused past 100 words (`intake::reply_too_long`): a result's detail belongs in the pull request or document it links. Telegram shows the bot typing only for work someone asked for; work a rule took on may be silent.

The run's prompt (`intake::run_prompt`) says when to speak: confirm once before substantial work, naming what it will do and the result, unless a rule asks for silence; stay quiet while working; answer with the conclusion and its link; ask one concrete question and record the Task blocked when someone must decide; say so when it can't finish; answer PASS and send nothing when there is nothing to do. `inferred` work rechecks the request and stops without widening when it reaches past the rules. An assessor's accept is no proof.

**How a Task ends.** A run whose answer reached the thread completes its Task: the people who asked have it, and it adds nothing to the owner's Attention. What counts is what the run sent last, with no more work after it, as the Task's own chat shows: a confirmation before the work isn't an answer, and another Task's message in the group isn't this one's. A plain reply reached nobody, so that Task awaits the owner's review. A first run that answers PASS and sends nothing cancels it with the reason `NOTHING_TO_DO`, which `stopped` tells apart from someone stopping it; a later one read something like "thanks", so the Task goes back to where it stood, its question still open. A run that leaves a coding agent or a background command going keeps its Task `working` with no run. What that wakes runs unattended under the same Mandate (`intake::mandated_task`), and once nothing runs for it, the Task finishes by what that turn sent (`wake_finished`); a turn with nothing to add leaves it where it stood before its run. A wake-up for a Task that is finished or stopped, or under a Mandate that no longer holds, doesn't run at all. Stop in an app holds while a Task waits, cancelling a Task stops what it left running, and a limit stops it too. Wake-ups count toward the Task's limits, and what it left running may go on for two hours at most, since a dev server left going is not work.

A blocked Task is a quiet blocker in Attention, as any is. Other turns of the bot neither read nor change what a channel took on: its goal came from outside words, so the Tasks note shows it only in its own chats, marked `from_outside`, the recent-work brief leaves work chats out, and the `tasks` tool reads it only from its own chats in a turn nobody started, and changes it only from a turn the user started, or as its own run records it blocked.

## Later messages

A message that plainly continues open work goes to it by code, with no model (`follow_up_target`), since a router that read every one would cost far more and answer no better. In Slack that is the Task of its thread; under intake a thread keeps no conversation, so a Runner remembers the threads the bot answered in, where a reply counts as addressed to it. Anywhere, it is a reply to a message of the Task's own chat, or a reply to the bot while one Task there is open. A Task's `follows` names the conversations it takes messages from; an accept starts it with its own. The message is copied into the work chat, and a running turn reads it at its next step (`turns::steer_contact`).

**Who it reaches depends on who wrote it.** Someone entitled, the person who asked or someone listed by their id on the service, wakes the Task: the decision stays `waiting` until a turn has read the message (`steering_read`), so one too late for the turn that was ending starts the next run (`start_waiting`). Anyone else's message waits there as context and wakes nothing, so a stranger's "lol" can't spend a run or overwrite a question that is waiting for an answer.

**A finished Task takes the assessment,** since a "thanks" is not worth a run. Only someone entitled is shown it, so only they can reopen it, or point a Task at another chat with `attach`, which adds that chat to `follows`. Nobody else can replay a Mandate by replying "again". Following lends no authority, and a Task someone stopped takes nothing more: from the chat, with Stop in an app, or by the user.

The run decides each message: it does what is within the Task, leaves ordinary talk unanswered, and calls `task_handback { message_id, reason }` for a request beyond it, which sends the message through the inbox again, once per message, for a fresh assessment under its real sender that can't attach it back. When someone asks it to stop, it calls `task_stop { message_id }`, which stops the Task only if code finds that message came in since the run before this one and its sender, by their id on the service, is entitled: a display name proves nothing, since anyone can take one. A message that no code path ties to a Task goes to the assessment, which may attach it or, for someone entitled, stop one.

When the owner changes the rules, each open Task under them hears at its next step (`intake_changed`), and each turn after that reads the rules as they stand, so a rule they narrowed narrows what is already running. It keeps going within them, since stopping half-finished work costs more; a next run starts only if its Mandate still holds.

An ordinary turn for a message intake left to the channel's task runs apart, in a queue of its own: one per conversation and at most three per channel at once, while assessing goes on, so nobody else's message waits on it. If one fails or a restart cuts it off, its delivery settles and a quiet blocker says so.

## Recovery

`intake::watch` looks over mandated Tasks every half minute, in code, outside any turn. A run that was cut off (`RUN_FAILED`, a provider or runtime failure, or a restart's `RESTARTED_DURING_RUN`) starts again up to three times, further apart each time, with a closing note to check the chat, its folder, git, and outside services before repeating anything, since what it did may have happened. Past that it stays blocked for the owner. A Task left waiting on background work that nothing lists and no turn is hearing goes the same way after two minutes, and one whose chat the user deleted closes. A queued Task starts when a slot is free, also after Reopen, and one whose Mandate stopped holding closes rather than wait. Reasons a person or a limit gave stay as they are: `STOPPED_BY_USER`, a run's own blocker, a budget. A paused channel, or a user away for a week, holds these runs as it holds everything else. It judges no progress: a run that goes in circles ends at its Task's limits, not sooner.

## In the apps

The desktop app (`desktop/sheet_intake.go`, `desktop/model/intake.go`) and the phone (`app/chat-info/channel/[id].tsx`, `channel/rule.tsx`) have an Intake section on the channel: Takes on work (Off, Only Record, On) with a note that says what each does, Off keeping what the owner wrote; Can Ask Directly, a row per person with Add…; Rules, a row per rule with its switch, opening its editor; and Decisions, the newest ten from `events.decisions`, leaving out what was left alone, each opening its task or conversation, with Let *name* Ask Directly on a row's menu. A task's sheet says who it came from, why (asked directly, or its rules), and the work. Nothing about intake notifies anyone, in any mode: observing is read when the owner opens the channel.

## Testing

`intake/tests.rs` runs against a stand-in chat model that answers the assessment and the turns apart: the gate's table; every check of an answer; what `observe` holds back; that `scope` and a name someone chose never reach what Auto-review reads; a batch keeping to its one sender; forwarded words; Tasks from one group running at once up to the limit, and the cap on open proactive work; who may wake a Task, and a finished one that can't be replayed; a hand-back reassessed once and never attached back; a stop refused to someone who took a listed person's name; an outage that drops nothing, and an answer or a refusal that holds up nobody; ordinary turns waiting apart; what counts as an answer reaching the thread; a wait on background work, a Stop during it, and a wake-up after it; a turn with nothing to add; a batch read again finding its Task; the assessor reading a batch however much came after; a Slack channel as one conversation with Tasks in their threads; what other turns can't read or change; rule changes reaching running work; a cut-off run's note; a deleted chat.

`scripts/intake-e2e.ts <lorca binary>` runs a real `lorca serve` against a stand-in Bot API and model, through the inbox, the turn, codemode, the account's tool, and Auto-review, with `kill -9` during an assessment and during a run. A test against the real services needs a Telegram bot with privacy mode off in a group, a Telegram account on a Runner, and a provider.

