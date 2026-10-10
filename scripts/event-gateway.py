#!/usr/bin/env python3
"""User-controlled webhook gateway, for a relay without Lorca's hosted receivers.
Plaintext stays on this paired Device.

With --github-secret-file it takes GitHub's events on /github: pull request events as
they came, for an event subscription; or, with --routine, a repository's events put in
the words a routine watching a pull request reads, the way the Lorca GitHub App does.
With --webhook-key-file it takes any request body on /hook from a sender that holds the
key, for one routine's webhook. Run it behind your HTTPS reverse proxy. The Lorca CLI
verifies this gateway's HMAC, seals the envelope to the Runner and durably queues
ciphertext before this server answers. It needs a running `lorca serve` to drain its
outbox.
"""

import argparse
import base64
import hashlib
import hmac
import json
import pathlib
import subprocess
import time
import uuid
from http.server import BaseHTTPRequestHandler, HTTPServer

MAX_BODY = 64 * 1024


def github_envelope(route, service_secret, body, signature, now):
    expected = "sha256=" + hmac.new(service_secret, body, hashlib.sha256).hexdigest()
    if not hmac.compare_digest(expected, signature):
        raise ValueError("invalid service signature")
    payload = json.loads(body)
    # GitHub signs the body, not X-GitHub-Event or X-GitHub-Delivery. Derive both
    # the event kind and replay key from signed content, never from those headers.
    if isinstance(payload, dict) and "zen" in payload and "hook_id" in payload:
        return None  # The ping GitHub sends when the webhook is created.
    if not isinstance(payload, dict) or not isinstance(payload.get("pull_request"), dict):
        raise ValueError("this gateway handles pull_request payloads")
    return signed(route, hashlib.sha256(body).hexdigest(), now, "github.pull_request", body.decode("utf-8"))


FAILED = ("failure", "timed_out", "action_required", "startup_failure")


def summary(kind, actor, name):
    """What happened, in a line, as the Lorca GitHub App says it."""
    by = lambda what: f"{what} by {actor}" if actor else what
    named = lambda what: f"{what}: {name}" if name else what
    return {
        "opened": by("Opened"), "commits": "New commits pushed", "ready": "Ready for review",
        "draft": "Turned back into a draft", "edited": "Edited", "reopened": "Reopened",
        "merged": "Merged", "closed": "Closed without merging", "approved": by("Approved"),
        "changes_requested": by("Changes requested"), "reviewed": by("Reviewed"),
        "review_comment": by("Review comment"), "thread_resolved": "Review thread resolved",
        "thread_unresolved": "Review thread reopened", "comment": by("Comment"),
        "check_failed": named("Check failed"), "checks_failed": named("Checks failed"),
        "status_failed": named("Status failed"), "workflow_failed": named("Workflow failed"),
    }.get(kind, kind)


def normalize(event, payload):
    """A GitHub event as a routine watching a pull request reads it, or None when it starts
    no run: (repository, pull request numbers, commit, new head, event payload)."""
    def text(value):
        return value if isinstance(value, str) and value else None
    action = payload.get("action") or ""
    repo = _dig(payload, "repository", "full_name")
    if not isinstance(repo, str):
        return None
    pr = payload.get("pull_request") if isinstance(payload.get("pull_request"), dict) else {}
    numbers = [pr["number"]] if isinstance(pr.get("number"), int) else []
    actor = text(_dig(payload, "sender", "login"))
    sha = head = name = None
    closes = False
    numbers_of = lambda items: [i["number"] for i in (items or []) if isinstance(i, dict) and isinstance(i.get("number"), int)]
    if event == "pull_request":
        head = text(_dig(pr, "head", "sha"))
        kind = {"opened": "opened", "synchronize": "commits", "ready_for_review": "ready", "converted_to_draft": "draft",
                "edited": "edited", "reopened": "reopened"}.get(action)
        if action == "closed":
            closes = True
            kind = "merged" if pr.get("merged") is True else "closed"
    elif event == "pull_request_review" and action == "submitted":
        actor = text(_dig(payload, "review", "user", "login")) or actor
        state = str(_dig(payload, "review", "state") or "").lower()
        kind = {"approved": "approved", "changes_requested": "changes_requested"}.get(state, "reviewed")
    elif event == "pull_request_review_comment" and action == "created":
        actor = text(_dig(payload, "comment", "user", "login")) or actor
        kind = "review_comment"
    elif event == "pull_request_review_thread":
        kind = {"resolved": "thread_resolved", "unresolved": "thread_unresolved"}.get(action)
    elif event == "issue_comment" and action == "created" and isinstance(_dig(payload, "issue", "pull_request"), dict):
        numbers = [payload["issue"]["number"]] if isinstance(_dig(payload, "issue", "number"), int) else []
        actor = text(_dig(payload, "comment", "user", "login")) or actor
        kind = "comment"
    elif event in ("check_run", "check_suite", "workflow_run") and action == "completed" and _dig(payload, event, "conclusion") in FAILED:
        numbers = numbers_of(_dig(payload, event, "pull_requests"))
        sha = text(_dig(payload, event, "head_sha"))
        name = text(_dig(payload, "check_suite", "app", "name")) if event == "check_suite" else text(_dig(payload, event, "name"))
        kind = {"check_run": "check_failed", "check_suite": "checks_failed", "workflow_run": "workflow_failed"}[event]
    elif event == "status" and payload.get("state") in ("failure", "error"):
        sha, name, kind = text(payload.get("sha")), text(payload.get("context")), "status_failed"
    else:
        kind = None
    if not kind:
        return None
    out = {"kind": kind, "summary": summary(kind, actor, name), "actor": actor,
           "data": {"event": event, "action": action}}
    title = text(pr.get("title")) or text(_dig(payload, "issue", "title"))
    url = text(pr.get("html_url")) or text(_dig(payload, "issue", "html_url"))
    if title:
        out["title"] = title
    if url:
        out["url"] = url
    if closes:
        out["ends"] = True
    return repo, numbers, sha, head, out


def _dig(obj, *keys):
    for key in keys:
        if not isinstance(obj, dict):
            return None
        obj = obj.get(key)
    return obj


def github_routine_envelopes(route, service_secret, body, signature, event, now, heads):
    """A repository's event, for a routine watching one of its pull requests: an envelope per
    pull request it concerns, each with that pull request as its subject (`Owner/repo#42`,
    as GitHub names the repository). `heads` maps a pull request to its head commit, so a
    commit status finds its pull request; it is updated in place."""
    expected = "sha256=" + hmac.new(service_secret, body, hashlib.sha256).hexdigest()
    if not hmac.compare_digest(expected, signature):
        raise ValueError("invalid service signature")
    payload = json.loads(body)
    if not isinstance(payload, dict) or ("zen" in payload and "hook_id" in payload):
        return []
    seen = normalize(event, payload)
    if not seen:
        return []
    repo, numbers, sha, head, out = seen
    if not numbers and sha:
        numbers = [int(key.rsplit("#", 1)[1]) for key, value in heads.items() if value == sha and key.rsplit("#", 1)[0].lower() == repo.lower()]
    envelopes = []
    for number in numbers:
        subject = f"{repo}#{number}"
        if head:
            heads[subject] = head
        event_payload = dict(out, subject=subject)
        delivery_id = f"{hashlib.sha256(body).hexdigest()}:{number}"
        envelopes.append(signed(route, delivery_id, now, "github", json.dumps(event_payload, ensure_ascii=False)))
    return envelopes


def webhook_envelope(route, key, body, headers, now):
    """Any body, from a sender that holds the routine's key, as the routine's `webhook` event:
    the relay's hosted webhook sends the same."""
    given = headers.get("Authorization", "")
    given = given[7:] if given.startswith("Bearer ") else headers.get("X-Lorca-Key", "")
    if not hmac.compare_digest(key, given.encode()):
        raise ValueError("invalid webhook key")
    text = body.decode("utf-8")
    try:
        data = json.loads(text) if text.strip() else None
    except ValueError:
        # A body that is not JSON reaches the Runner as a string.
        data = text
    payload = json.dumps({"kind": "request", "summary": "Webhook request", "data": data}, ensure_ascii=False)
    # Nothing signs a generic body, so a repeat is one delivery only when the sender says so.
    idempotency = headers.get("Idempotency-Key", "")[:200]
    delivery_id = hashlib.sha256(("key:" + idempotency).encode()).hexdigest() if idempotency else uuid.uuid4().hex
    return signed(route, delivery_id, now, "webhook", payload)


def signed(route, delivery_id, now, event_type, payload):
    event = {
        "version": 1,
        "subscription_id": route["subscription_id"],
        "generation": route["generation"],
        "delivery_id": delivery_id,
        "occurred_at": now,
        "event_type": event_type,
        "payload": payload,
    }
    fields = [event[key] for key in (
        "version", "subscription_id", "generation", "delivery_id",
        "occurred_at", "event_type", "payload",
    )]
    message = json.dumps(fields, ensure_ascii=False, separators=(",", ":")).encode()
    tag = hmac.new(route["secret"].encode(), message, hashlib.sha256).digest()
    event["signature"] = base64.urlsafe_b64encode(tag).decode().rstrip("=")
    return event


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--route", type=pathlib.Path, required=True)
    service = parser.add_mutually_exclusive_group(required=True)
    service.add_argument("--github-secret-file", type=pathlib.Path)
    service.add_argument("--webhook-key-file", type=pathlib.Path)
    parser.add_argument("--routine", action="store_true",
                        help="with --github-secret-file: the route is a routine's that watches a pull request")
    parser.add_argument("--lorca", default="lorca", help="absolute path recommended")
    parser.add_argument("--bind", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8984)
    args = parser.parse_args()

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            self.connection.settimeout(10)
            path = "/github" if args.github_secret_file else "/hook"
            if self.path != path:
                self.respond(404)
                return
            try:
                size = int(self.headers.get("Content-Length", "-1"))
                if size < 1 or size > MAX_BODY or self.headers.get("Transfer-Encoding"):
                    self.respond(413)
                    return
                body = self.rfile.read(size)
                if len(body) != size:
                    self.respond(400)
                    return
                # Read on every request so a rotated route/secret takes effect immediately.
                route = json.loads(args.route.read_bytes())
                secret = (args.github_secret_file or args.webhook_key_file).read_bytes().rstrip(b"\r\n")
                if len(secret) < 32:
                    raise ValueError("service secret must have at least 32 bytes")
                if args.github_secret_file and args.routine:
                    heads_file = args.route.with_name(args.route.name + ".heads.json")
                    heads = json.loads(heads_file.read_text()) if heads_file.exists() else {}
                    events = github_routine_envelopes(route, secret, body, self.headers.get("X-Hub-Signature-256", ""),
                                                      self.headers.get("X-GitHub-Event", ""), int(time.time()), heads)
                    heads_file.write_text(json.dumps(heads))
                elif args.github_secret_file:
                    event = github_envelope(route, secret, body, self.headers.get("X-Hub-Signature-256", ""), int(time.time()))
                    events = [event] if event else []
                else:
                    events = [webhook_envelope(route, secret, body, self.headers, int(time.time()))]
            except (ValueError, UnicodeError):
                self.respond(403)
                return
            except (OSError, TimeoutError):
                self.respond(503)
                return
            if not events:
                self.respond(200)
                return
            for event in events:
                try:
                    result = subprocess.run(
                        [args.lorca, "events", "forward", str(args.route)],
                        input=json.dumps(event, ensure_ascii=False).encode(),
                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15,
                        check=False,
                    )
                    if result.returncode:
                        self.respond(503)
                        return
                except (OSError, subprocess.TimeoutExpired):
                    self.respond(503)
                    return
            self.respond(202)

        def respond(self, status):
            self.send_response(status)
            self.send_header("Content-Length", "0")
            self.end_headers()

        def log_message(self, *_):
            # Event content, signature, route and service secrets never enter logs.
            pass

    HTTPServer((args.bind, args.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
