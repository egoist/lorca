# Hosted receivers

A receiver is where the relay takes a service's requests for [routines that run on events](routine-triggers.md#routines-on-events): the Lorca GitHub App's webhooks for pull request watches, and each routine's own webhook. It checks what the service sent (GitHub's signature, the routine's key), turns it into an event about one subject, signs it as the Runner's event inbox expects ([the delivery contract](event-triggers.md#signed-delivery-contract)), seals it to the Runner's box key, and stores it as an `event` blob for that machine alone. The Runner verifies, deduplicates, and runs it as any [event subscription](event-triggers.md)'s delivery. `crates/relay/src/receivers` holds them: `mod.rs` the API, signing, and sealing, `github.rs` the App, `webhook.rs` the routine webhooks. Each is on when its settings are given; a relay without one answers `404` for it, and the routine goes through the user's [gateway](event-triggers.md#user-controlled-gateway) instead.

## What the relay sees

A service sends its webhooks in plain text, so a receiver reads each request while it handles it: the pull request event or the webhook's body, in memory, until the sealed envelope is stored. It keeps none of it. What stays is the sealed `event` blob, which only the Runner opens, and a row per subscription: the receiver, the account's identity and the Runner's machine key, the subject (`acme/project#42`, none for a webhook), the Runner's subscription id, generation, and signing secret, a webhook key's SHA-256, a GitHub installation id, and a pull request's head commit. With the signing secret the relay can sign an event for that subscription, which is what a gateway does; it can't sign for another, and the Runner still checks the subscription, generation, and filter of each. The rest of the relay stays the store of ciphertext and public keys ([Constraints](../../ARCHITECTURE.md#constraints)). Forgetting the identity deletes its rows; a subscription whose machine is unpaired goes at its next delivery.

## The API

A Runner calls the receivers with its machine token, like any relay request:

- `POST /v1/receivers/{receiver}/subscriptions` with `{ subject, subscription_id, generation, secret, key_hash? }` answers `{ status: "subscribed", id, name, title?, url?, endpoint? }`, `{ status: "needs_setup", name, setup_url }` with the link the user follows first, or `{ status: "refused", message }`. An identity keeps at most 200 subscriptions per receiver (`409` past that).
- `DELETE /v1/receivers/{receiver}/subscriptions/{id}` removes one of the identity's.
- `POST /v1/receivers/webhook/subscriptions/{id}/key` with `{ key_hash }` replaces a webhook's key; the old key stops working at once.
- `POST /v1/receivers/{receiver}/setup` with `{ subject? }` answers `{ url }`: for GitHub, the App's install link, or its authorize link when the App is installed on the subject's repository already.

Outside `/v1`, without the protocol header: `POST /github/webhook` (GitHub's deliveries, up to 1 MiB), `GET /github/callback` (the install's end, rate-limited per IP), and `POST /r/{id}` (a routine's webhook). `receiver_states`, `receiver_accounts`, and `receiver_subscriptions` hold them on SQLite and Postgres (migration 3), and `/metrics` counts sealed events in `receiver_events_total`.

## The Lorca GitHub App

`crates/relay/github-app.json` is the App's manifest: read access to metadata, pull requests, issues (for pull request comments), checks, statuses, and actions, and the events `pull_request`, `pull_request_review`, `pull_request_review_comment`, `pull_request_review_thread`, `issue_comment`, `check_run`, `check_suite`, `status`, and `workflow_run`, sent to `https://hooks.lorca.app/github/webhook`. It writes nothing; a bot reads the pull request through its own [GitHub plugin](marketplace.md) with the user's token.

**Install and bind.** An installation belongs to whoever installed it, and an organization's covers repositories that some of its members can't see. Binding it wrongly would hand someone pull requests they can't read, so a binding carries the repositories its user showed they can access. `setup` stores a random state for the identity (an hour) and answers `https://github.com/apps/<slug>/installations/new?state=<state>`, or, where the App is installed on the repository already (by the user, or an organization's owner), `https://github.com/login/oauth/authorize?client_id=<id>&state=<state>`. The App asks for the user's authorization at install (`request_oauth_on_install`), so either ends at `/github/callback` with the state and a code, and an install with the installation id. The relay takes the state (once) and exchanges the code for the user's token (`POST /login/oauth/access_token` with the App's client id and secret). With it, it binds to the state's identity every installation `GET /user/installations` lists, each with the repositories `GET /user/installations/{id}/repositories` lists (the ones the user can access there, up to 1,000), replacing what an earlier binding kept. The token is used for that alone and kept nowhere. The callback redirects to `https://lorca.app/github/connected?status=…` ([Website](website.md)): `connected` (with the account), `not_yours` (the installation GitHub named isn't among the user's), `expired`, `requested` (an organization owner approves the install first), or `failed`. An installation can be bound to several identities, each with its own repositories.

**Subscribe.** The subject is `owner/repo#42`, compared in lowercase. The relay finds the App's installation on the repository with its own token (an RS256 JWT from the private key). With none, it answers `needs_setup` with an install link. When the identity's binding to that installation doesn't list the repository, it answers `needs_setup` with an authorize link: the user may not see the repository, or it was added since, and authorizing again lists it if they can. Otherwise it reads the pull request with an installation token: one that doesn't exist is refused, and so is one already merged or closed. The row keeps the installation, the subject as the Runner gave it, and the head commit; the answer carries the title and link. Access is checked when the user authorizes and when a routine subscribes: a watch that is subscribed keeps its events if the user loses access later, until it ends or is deleted.

**Deliveries.** `/github/webhook` checks `X-Hub-Signature-256`, HMAC-SHA256 of the exact body with the App's webhook secret, in constant time, before anything else (`401` otherwise), and takes `X-GitHub-Delivery` as the delivery id, so GitHub's redelivery is one delivery. It turns an event into what a watch reads, or into nothing:

| Event | Starts a run |
| --- | --- |
| `pull_request` | opened, new commits (`synchronize`), ready for review, back to draft, edited, reopened, merged, closed |
| `pull_request_review` | submitted: approved, changes requested, reviewed |
| `pull_request_review_comment`, `issue_comment` on a pull request | created |
| `pull_request_review_thread` | resolved, unresolved |
| `check_run`, `check_suite`, `workflow_run` | completed with `failure`, `timed_out`, `action_required`, or `startup_failure` |
| `status` | `failure` or `error`, matched to a pull request by its head commit |

The event is `{ subject, kind, summary, actor, title, url, data, ends? }`: `summary` in a line ("Changes requested by kim", "Check failed: test (ubuntu)"), `data` the review, comment (clipped to 2,000 characters), check, or status with the pull request's state. Each subscription to a pull request the event names gets its own envelope, and a new head commit is kept for the statuses after it. `closed` sets `ends`: the subscription goes once its envelope is stored, and that run is the routine's last. An `installation` deleted unbinds it and ends its subscriptions, and a repository taken off it ends that repository's; each routine they served gets `{ kind: "unsubscribed", unsubscribed: true }`, which puts it back to subscribing, and so to Install App. GitHub sends nothing when the base branch moves and makes a conflict, so a watch doesn't notice one.

`LORCA_RELAY_GITHUB_APP_ID`, `_APP_SLUG`, `_CLIENT_ID`, `_CLIENT_SECRET`, `_WEBHOOK_SECRET` (at least 16 characters), and `_PRIVATE_KEY` (the PEM or its path) turn the App on, all or none; `LORCA_RELAY_GITHUB_DONE_URL` is the page the install ends on, `https://lorca.app/github/connected` by default.

## Routine webhooks

`LORCA_RELAY_HOOKS_URL` (`https://hooks.lorca.app`, a public address that reaches the relay) turns them on. A routine set up with `receiver: "webhook"` subscribes with the SHA-256 of a 32-byte key the Runner made, and gets `endpoint`, `<hooks>/r/<id>` with a random id, which the routine keeps with the key. The routine sheet shows both, and the Authorization header.

`POST /r/{id}` answers `401` unless `Authorization: Bearer <key>` hashes to the row's hash (compared in constant time), `413` over 64 KiB, `429` past a request a second in bursts of 30 per webhook, and `400` for a body that isn't UTF-8. The body is `data`: JSON as it came, any other text as a string, null when empty, with `content_type`. An `Idempotency-Key` makes a repeat within a day `200 { duplicate: true }`, and its delivery id derives from the key, so the Runner's inbox drops a later repeat too; without one every request is its own. `202` means the request is sealed to the Runner, which runs the routine once with the body as data from outside. Regenerate Key sends a new hash; deleting the routine deletes the row, and the URL answers `404`.

## Setting it up

The relay's owner registers the App from the manifest (GitHub's App settings, or the manifest flow), sets its webhook secret, downloads its private key, notes its id, slug, client id, and client secret, and gives them to the relay in the variables above with `LORCA_RELAY_HOOKS_URL`. `hooks.lorca.app` routes to the relay's `/github/*` and `/r/*`. A self-hosted relay can register an App of its own the same way, or leave both off: routines on events then use the gateway, and the GitHub skill says how.
