# Event triggers

An event subscription starts work in a bot's DM on its assigned Runner when a service sends an event: a PR update, an incoming message, a finished meeting transcript. A user-controlled gateway verifies the service, signs a Lorca envelope with the subscription's secret, and seals it to the Runner. The Runner checks the signature, filters the event, and queues it in an encrypted inbox that deduplicates, orders or coalesces deliveries, and starts an `event` Job through the normal runtime. `crates/cli/src/event_triggers.rs` holds subscriptions, delivery, health, and the inbox. Routines keep their scheduled checks beside this; an event runs its subscription's task directly. A [channel](channels.md) is a subscription whose source is Telegram or Slack, which the Runner reads itself.

## Configuration and health

`lorca events add <config.json>` creates a subscription on the Runner the bot is assigned to:

```json
{
  "name": "PR changes",
  "source": "gateway_hmac",
  "bot_id": "bot-12345678",
  "routine_id": null,
  "prompt": "Summarize new PR activity. Read the PR to verify its current state, and tell me what needs attention.",
  "event_types": ["github.pull_request"],
  "filters": [
    {"pointer": "/repository/full_name", "equals": "acme/project"},
    {"pointer": "/action", "equals": "synchronize"}
  ],
  "queue_policy": "fifo",
  "is_enabled": true,
  "expires_at": null
}
```

`source` is `gateway_hmac` for a gateway's subscription, or `telegram` or `slack` with a `channel` for a [channel](channels.md#a-channel), which has no route or generation to rotate. The CLI generates the subscription id and a random signing secret. A filter is an RFC 6901 JSON pointer into the payload and the value it must equal; an event passes when its type is listed and every filter matches. `routine_id` names one of the bot's routines: the event's Job carries it as its scope, and a paused routine holds the event's work, but the event runs the subscription's own prompt, never the routine's check, and leaves the routine's schedule and last outcome alone.

- `events list` shows each subscription's configuration, state (`idle`, `ready`, `running`, `paused`, `expired`, `blocked`, `attention`), pending count, its twenty newest deliveries and their states, the last authenticated receipt and last successful turn, and why work is held. It never shows the secret or a payload.
- `events edit <id> <config.json>` replaces the configuration; the target bot and routine stay fixed.
- `events pause <id>` holds the work while deliveries keep queueing; `events resume <id>` runs it again. A turn already running settles normally; one waiting for its chat lock checks pause, expiry, and its routine again before inference.
- `events reconnect <id> [--expires-at <unix-seconds>]` rotates the secret and generation and sets the expiry (none without the flag). A route from the old generation no longer authenticates; deliveries still on the gateway or relay under the old secret need redelivery. Queued work stays.
- `events route <id> <file>` writes the gateway route: the subscription id, generation, secret, and the Runner's id and box key. The file is created with mode 0600 and never overwrites one that exists.
- `events remove <id>` deletes the subscription and its inbox once no turn of it is running.
- `events retry <delivery-id>` runs a failed or interrupted delivery again after the user has read the chat; `events discard <delivery-id>` drops a pending, failed, or interrupted one and keeps only its deduplication mark.

Expiry holds execution and keeps pending work. When nobody has written in any chat for seven days (the routines' away rule), work waits, and runs once the user writes again.

The local API has `events.list/create/update/pause/resume/reconnect/route/delete/retry/discard`, taking `id`, `config`, or both, and `events.forward { route, envelope }` for a gateway. With `runner_id`, the local CLI asks that Runner through a sealed `request`, so any paired Device can inspect and manage a Runner's subscriptions.

## User-controlled gateway

The gateway is the assigned Runner or another paired Device of the account. It receives the service's plaintext on the user's HTTPS endpoint, verifies the service's signature, signs a Lorca envelope, and pipes it to `lorca events forward <route.json>`. The CLI checks the envelope's signature, subscription, and generation against the route, refuses an expired route, then:

- on the assigned Runner, verifies and stores it in the inbox directly;
- on another Device, checks that the route's Runner is a paired Runner whose box key matches, seals the envelope to that key, and queues the ciphertext in its durable relay outbox under a random blob id. A running `lorca serve` uploads it, so it survives a relay outage and a gateway restart.

The gateway uses its existing machine and relay bearer, registers nothing new, and keeps the service's token and webhook secret to itself.

`scripts/event-gateway.py` is the gateway for GitHub pull requests:

1. On the Runner, create the subscription above and export its route with `lorca events route <id> /private/path/pr-route.json`.
2. Put the route on the gateway (or keep it on the Runner), with a random GitHub webhook secret of at least 32 bytes in another private file, and run `lorca serve` there.
3. Run `python3 scripts/event-gateway.py --route /private/path/pr-route.json --github-secret-file /private/path/github-secret --lorca /absolute/path/lorca`. It listens on `127.0.0.1:8984/github`; put the user's HTTPS reverse proxy in front of it.
4. In the repository's webhook settings, set that URL, JSON content, the same secret, and Pull requests events.

The script compares `X-Hub-Signature-256` with HMAC-SHA256 of the exact body in constant time ([GitHub's contract](https://docs.github.com/en/webhooks/using-webhooks/validating-webhook-deliveries)). GitHub does not sign its event and delivery headers, so the script takes the type from the signed body (`github.pull_request`) and the delivery id from SHA-256 of the body: identical bodies are one delivery whatever the headers say. It answers `202` once Lorca has stored or queued the envelope, `200` to GitHub's signed ping, `403` to a bad signature or another event, `413` to a body over 64 KiB, and `503` when Lorca cannot take it; GitHub's delivery history redelivers a refused one. It logs no bodies or secrets, and reads the route and secret on every request, so a new route takes effect at once.

An adapter for another service follows the same contract: verify the provider's signature and replay protection, pick a stable delivery id, sign the envelope.

## A routine's webhook

A routine can take any service's or script's request, with a URL and a key of its own: an event subscription whose `routine_id` is the routine and whose `event_types` is `["webhook"]`, and the gateway in webhook mode. `python3 scripts/event-gateway.py --route <route.json> --webhook-key-file <key-file> --lorca <path>` listens on `127.0.0.1:8984/hook` behind the user's HTTPS proxy and takes a body from a sender that holds the key, as `Authorization: Bearer <key>` or `X-Lorca-Key: <key>`, compared in constant time; the key has at least 32 bytes. The body is the event's `payload`: JSON as it came, any other text as a JSON string. Nothing signs a generic body, so an `Idempotency-Key` header makes a repeat one delivery, and without one every request is its own. Each routine's webhook is its own subscription, route, key, and gateway, so rotating one (`events reconnect`) leaves the others as they are. The run reads the body as untrusted data, like any event's.

### No webhook inbox on the relay

A webhook inbox on the relay, a public URL per routine whose requests the relay seals to the Runner, would spare the user the gateway and its HTTPS endpoint. The relay has none:

- It would read every request in plaintext before sealing it. Services send webhooks unencrypted, so for that traffic the relay would stop being the store of ciphertext the [constraints](../../ARCHITECTURE.md#constraints) require: whoever runs it, lorca.app included, could read the pull requests, payments, and messages passing through.
- A service's signature is checked with its secret. On the relay that is one more secret its operator holds; without it the relay keeps whatever anyone posts to a leaked URL until the Runner turns it away.
- The relay takes only requests signed by an account's Devices. A public inbox is unauthenticated ingress with its own abuse, quota, and rate limits, and new relay endpoints that every client and relay must follow.

The gateway keeps plaintext on a Device the user owns, and a tunnel or reverse proxy gives it the public URL. Following one pull request needs no webhook: a [watch](routine-triggers.md#watches) reads it through GitHub on the Runner.

## Signed delivery contract

```json
{
  "version": 1,
  "subscription_id": "ev-uuid",
  "generation": 1,
  "delivery_id": "provider-stable-delivery-id",
  "occurred_at": 1791395200,
  "event_type": "github.pull_request",
  "payload": "{\"repository\":{\"full_name\":\"acme/project\"}}",
  "signature": "base64url-hmac-sha256-without-padding"
}
```

`payload` is the exact JSON text. The signature is unpadded base64url of HMAC-SHA256, keyed by the route secret's UTF-8 bytes, over the compact UTF-8 JSON (non-ASCII unescaped) of

```text
[version, subscription_id, generation, delivery_id, occurred_at, event_type, payload]
```

so it covers every routing, replay, and filter field as well as the body. The Runner verifies it in constant time before anything else, relay deliveries included. An unknown version, a bad signature or generation, malformed JSON, a payload over 64 KiB, a timestamp more than five minutes ahead, or one more than seven days old (the relay's retention of envelopes; UTC Unix seconds) starts no work, and an authentication failure shows in the subscription's health. The signature says the configured gateway sent it; the content is still untrusted data.

## Durable ordering and execution

`event_subscriptions` and `event_inbox` in `lorca.sqlite3` hold rows encrypted with the account key (associated data `event_subscription` and `event_inbox`); only subscription ids, delivery hashes, and queue positions are columns. Forgetting the identity clears both.

A delivery's key is SHA-256 of `(subscription_id, delivery_id)`, without the relay blob id or the generation, so a repeated upload, a redelivery, or a key rotation is one delivery. The delivery and its deduplication mark commit in one transaction before the relay copy is let go: a storage failure leaves the sync cursor before that envelope, and it is read again. Deleting the relay copy is queued once the delivery is stored, whether or not a turn runs. Done, filtered, and coalesced deliveries drop their payloads and keep their marks for thirty days, past the seven-day window. Pending, failed, and interrupted ones stay until they run or are removed.

`fifo` runs one delivery per subscription at a time, in arrival order; a failed or interrupted one holds the rest until it is retried or discarded. `latest` turns every pending delivery of the subscription into a coalesced mark when a new one passes its filters, across all its event types, and leaves a running one alone. Subscriptions order independently; their Jobs queue with all other work for the bot's DM.

Admission marks the delivery running and snapshots the subscription's task. A channel's delivery runs in the conversation it names, which must still be the channel's, opening with the contact's message instead of a marker; one whose conversation the user deleted is dropped ([Channels](channels.md#conversations)). The `event` Job goes through `runtime::start_turn`, the chat lock, the account's providers, the bot's [Access](bot-permissions.md), the turn hooks, codemode, and tool Auto-review, as unattended work like a routine's run: a ten-minute script limit, and an action that would ask the user is staged in the [review queue](review-queue.md) instead. The turn opens with an `Event · Name` notice. The system prompt carries the owner's task; the payload, clipped to 8,000 characters, is the turn's closing note, marked as untrusted data that cannot instruct, approve, or grant anything. Auto-review reads the task and the turn's steps, never the payload. Only the inbox admits an `event` Job: one arriving as a relay `job` blob is dropped, and a turn whose Job id, bot, routine, or requesting Device differs from the admitted delivery does not run or settle it.

A Runner that restarts with a delivery still running marks it interrupted: a tool may already have acted, so the delivery does not run again until the user retries it. Delivery is durable and deduplicated; external effects are not exactly once. A turn the user stops while it waits for its chat fails its delivery too, except a channel's: every channel delivery the user stops is settled, so the channel takes the next message ([Channels](channels.md#conversations)).

An event's turn counts toward its DM's [limits](budgets.md), or its routine's when the subscription targets one. A routine stopped at its limits holds the events aimed at it until the user resumes it. A turn its limits stop fails its delivery; Resume in its Limits sheet (`budgets.resume { kind: job, run: true }`) puts the delivery back in the inbox, which admits it again in its place under the same Job id and what it used.

While a delivery has failed or was interrupted, the bot's DM has a quiet blocker in [Coordinator attention](attention.md), "Events on hold: Name" ("Channel on hold: Name" for a channel, whose sheet in every app offers Try Again and Skip), which every app lists and which opens the DM; a gateway that stopped authenticating has another, "Events refused: Name". The first settles once the delivery is retried or discarded, the second once a delivery authenticates again or the subscription is reconnected, and removing the subscription settles both.

## Relay transport

Relay protocol 3 adds `kind=event` to the relay and to what Devices poll. A protocol-3 relay must be deployed before clients that ask for this kind, since an older one refuses the unknown kind. An `event` blob is sealed to `recipient_machine_pubkey`, with no slot or group; the relay refuses one without a recipient. It stores the ciphertext and metadata as for other machine envelopes, only the recipient lists or opens it, and an unconsumed one goes after seven days with other sealed work.
