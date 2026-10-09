#!/usr/bin/env python3
"""User-controlled webhook gateway. Plaintext stays on this paired Device.

With --github-secret-file it takes GitHub pull request events on /github; with
--webhook-key-file it takes any request body on /hook from a sender that holds the
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


def webhook_envelope(route, key, body, headers, now):
    """Any body, from a sender that holds the routine's key, as a `webhook` event."""
    given = headers.get("Authorization", "")
    given = given[7:] if given.startswith("Bearer ") else headers.get("X-Lorca-Key", "")
    if not hmac.compare_digest(key, given.encode()):
        raise ValueError("invalid webhook key")
    text = body.decode("utf-8")
    try:
        json.loads(text)
        payload = text
    except ValueError:
        # A body that is not JSON reaches the Runner as a JSON string.
        payload = json.dumps(text, ensure_ascii=False)
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
                if args.github_secret_file:
                    event = github_envelope(route, secret, body, self.headers.get("X-Hub-Signature-256", ""), int(time.time()))
                else:
                    event = webhook_envelope(route, secret, body, self.headers, int(time.time()))
            except (ValueError, UnicodeError):
                self.respond(403)
                return
            except (OSError, TimeoutError):
                self.respond(503)
                return
            if event is None:
                self.respond(200)
                return
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
