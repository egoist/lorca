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

Code decides a message's strength from who wrote it and whether it addressed the bot, before any model is asked (`intake::gate`). Words can't reach this. A message addresses the bot when it is a direct message, a mention, a reply to the bot, or carries a hashtag the channel listens for. The readers leave out bots' messages. What the channel takes at all is still its `listen` filter and allowed chats; reading a whole group takes `every`, and on Telegram privacy mode off.

| Sender | Addressed | With an enabled rule | With none |
| --- | --- | --- | --- |
| listed | yes | assessed, `explicit` | assessed, `explicit` |
| listed | no | assessed, `inferred` | recorded and left alone |
| anyone else | yes | assessed, `inferred` | the channel's ordinary turn |
| anyone else | no | assessed, `inferred` | recorded and left alone |

`explicit` work needs no rule and goes ahead in either mode. `inferred` work needs a matching rule and `auto`.

## The assessment

`inbox` hands intake a batch: the waiting messages of one conversation, at most eight, one batch per channel at a time, so a burst costs about one request. It reads one request to the small model of the bot's provider (`providers::chat_reviewer`), with no tools and no state, cut off after 90 seconds, its cost counted in the conversation. Nothing it reads can act, because it has nothing to act with.

The system prompt holds what the model does and the owner's task and rules, which change rarely, so a provider caches them. The request holds the message's standing by the gate, the channel's Tasks by id, goal, and state, newest ten, and `<conversation>`: the newest messages up to forty and 16,000 characters, each one's words with angle brackets replaced so none can close the fence, new ones marked `[new]`. The model answers one JSON object in `<decision>`: `decision` (`accept`, `attach`, `stop`, `reply`, `ignore`), a judgment of each enabled rule, `direction` (`work_requested`, `target_settled`, `needs_design_decision`), `evidence` quoted from the conversation, a `title` and `scope` for an accept, a `task_id`, and a `reason`.

A message can talk the model into any answer, so the answer proves nothing by itself (`assess::outcome`):

- Every enabled rule is judged exactly once, or the assessment failed. An unknown decision or an unreadable reply failed too.
- `accept` of `inferred` work needs `direction` settled, a matched rule, and evidence that is really in the conversation, spacing and case aside. Short of any of these it is `ignore`, with why. In `observe` a good one becomes `would_accept`.
- `attach` and `stop` must name a Task it was shown, and `stop` needs `explicit` strength.
- `reply` needs a message that addressed the bot.

A failed assessment waits and tries again, 30 seconds further apart each time, and its messages stay queued. After three it records `unavailable` and raises a quiet blocker in [Attention](attention.md), since a listed person's request must not vanish; the next decision in the conversation settles it. A restart puts a batch being read back in the queue: an assessment only reads, so it is safe to repeat, and a Task it accepted has an id and a request that follow from its message.

## Decisions

`intake_decisions` in `lorca.sqlite3` keeps each decision encrypted with the account key (associated data `intake_decision`), with only its id, channel, time, and whether it waits as columns; a channel's newest 500 for thirty days, and deleting the channel deletes them. `events.decisions { id, limit, runner_id }` reads them from the channel's Runner, so they stay out of the `machine` blob, which would otherwise change per message. A decision is `accepted`, `would_accept`, `attached`, `stopped`, `replied`, `ignored`, or `unavailable`, with the messages it decided, their sender and user id, its strength, the rules' judgments, its evidence, and its Task. Titles, reasons, and quotes come from outside Lorca, and the bot's `channels` `decisions` action marks them as data.

## The Mandate

An accept makes a Task through `tasks::create_mandated`, the only way to set its `mandate`. No `tasks.*` request or tool edit reaches it, and it travels with the Task to every Device. It records why the work may be done: the channel and its task as they stood, the conversation and message, the sender, the strength, the rules matched in the owner's words as they stood, a listed person's request word for word, the assessment's `scope`, `reason`, and evidence, and the version it was accepted under. Only a new accept widens it.

**Authority stays apart from outside words.** A mandated run is unattended as a channel's turn is ([Auto-review](auto-review.md#what-the-review-reads)), and Auto-review reads its task as the user's own words. So that task (`intake::task_event`) holds only the channel's task, and either the rules, or, for `explicit` work, what the listed person wrote, under the name the owner gave them. It never holds the assessment's `title` or `scope`, which outside words shaped: those reach the run as data in its closing note, marked as granting nothing. An assessor talked into a wide scope gets a Task with that goal and no more authority than its rule gives. What the run does still passes Access and Auto-review, and what would ask is staged in the [review queue](review-queue.md).

`Mandate::holds` is checked again before a later message resumes a Task: an `explicit` one while its sender stays listed, an `inferred` one while the channel is in `auto` and its rules stay on. A rule whose words changed still holds.

## A Task's work chat

Each mandated Task runs in a chat of its own, `Mandate.work_chat_id`: a hidden direct chat whose `channel` carries `task_id`, speaking from the same account and thread as its conversation. Turns in one chat run one at a time and share its transcript, so without this a Telegram group's Tasks would queue behind each other and mix their context. With it they run at once, and each reads only its own work. Every app leaves work chats out of its lists; the Task's sheet opens one.

The batch's messages are copied in, and the conversation's ten lines before them are inlined in the first run's closing note, as data; `recall` finds the rest. What the run sends through the account's tools lands in the work chat and in the conversation, whose transcript the next assessment reads and the user follows (`channels::sent`). Its reasoning and its tool calls stay in the work chat, where nothing marks anything unread or alerts: a result reaches people in their thread, and a blocker reaches the owner in Attention. A reply from a work chat is refused past 100 words (`intake::reply_too_long`): a result's detail belongs in the pull request or document it links.

The run's prompt (`intake::run_prompt`) says when to speak: confirm once before substantial work, naming what it will do and the result, unless a rule asks for silence; stay quiet while working; answer with the conclusion and its link; ask one concrete question and record the Task blocked when someone must decide; say so when it can't finish; answer PASS and send nothing when there is nothing to do. `inferred` work rechecks the request and stops without widening when it reaches past the rules. An assessor's accept is no proof.

A run that ends with a result and evidence completes its Task (`Task.state` `completed`): the people who asked have the answer in their thread, and a reply that needs no reading adds nothing to the owner's Attention. PASS with nothing sent cancels it with the reason `NOTHING_TO_DO`, which `stopped` tells apart from someone stopping it. A blocked Task is a quiet blocker in Attention as any is.

## Later messages

A message that plainly continues a Task goes to it by code, with no model (`follow_up_target`), since a router that read every one would cost far more and answer no better. In a Slack thread that is the thread's open Task, else its newest. In a chat that holds many exchanges it is a reply to the message the Task started from, or a reply to the bot while one Task there is open. The Task's `follows` names the conversations it takes messages from; an accept starts it with its own, and an `attach` from another chat adds that one, so a person can pick the work up elsewhere by pointing at it. Following lends no authority.

The message is copied into the work chat and recorded `attached` and `waiting`. If the Task's turn is running, it reads the message at its next step (`turns::steer_contact`), and reading it clears `waiting` (`steering_read`). Anything still `waiting` when the run ends starts the next run (`start_waiting`), and an ended Task is requeued first, unless someone stopped it or its Mandate no longer holds. A message too late for the turn that was ending is therefore never lost.

The run decides each one: it does what is within the Task, leaves ordinary talk unanswered, and calls `task_handback { message_id, reason }` for a request beyond it, which sends the message through the inbox again for a fresh assessment that can't attach it back. When someone asks it to stop, it calls `task_stop { message_id }`, which stops the Task only if code finds the message's sender, by their id on the service, is the requester or listed: a display name proves nothing, since anyone can take one. A message that no code path ties to a Task goes to the assessment, which sees the channel's Tasks and may attach it or, for a listed person, stop one.

## Recovery

`intake::watch` looks over mandated Tasks every half minute, in code, outside any turn. A run that was cut off (`RUN_FAILED`, a provider or runtime failure, or a restart's `RESTARTED_DURING_RUN`) starts again up to three times, further apart each time, with a closing note to check the chat, its folder, git, and outside services before repeating anything, since what it did may have happened. Past that it stays blocked for the owner. Reasons a person or a limit gave stay as they are: `STOPPED_BY_USER`, a run's own blocker, a budget. A paused channel, or a user away for a week, holds these runs as it holds everything else. It judges no progress: a run that goes in circles ends at the ten-minute limit on an unattended turn or its [limits](budgets.md).

## In the apps

The desktop app (`desktop/sheet_intake.go`, `desktop/model/intake.go`) and the phone (`app/chat-info/channel/[id].tsx`, `channel/rule.tsx`) have an Intake section on the channel: Takes on work (Off, Only Record, On) with a note that says what each does, Off keeping what the owner wrote; Can Ask Directly, a row per person with Add…; Rules, a row per rule with its switch, opening its editor; and Decisions, the newest ten from `events.decisions`, leaving out what was left alone, each opening its task or conversation, with Let *name* Ask Directly on a row's menu. A task's sheet says who it came from, why (asked directly, or its rules), and the work. Nothing about intake notifies anyone, in any mode: observing is read when the owner opens the channel.

## Testing

`intake/tests.rs` runs against a stand-in chat model that answers the assessment and the turns apart. The gate's table, every check of an answer (a rule left out or invented, a quote that isn't there, a stranger's stop, a Task it was not shown), what `observe` holds back, that `scope` never reaches what Auto-review reads, two Tasks from one group running at once in their own chats, a follow-up reaching its Task with no assessment, a hand-back reassessed and never attached back, a stop refused to someone who took a listed person's name, a stopped Task staying stopped, a batch reaching its Task whole, work chats marking nothing unread, Off keeping the rules, an assessment that fails three times raising a blocker, and a cut-off run starting again with its note while a user's stop stays. A live test needs a Telegram bot with privacy mode off in a group, a Telegram account on a Runner, and a provider.
