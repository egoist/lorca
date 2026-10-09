#!/usr/bin/env python3
"""User-controlled GitHub PR webhook gateway. Plaintext stays on this paired Device.

Run behind your HTTPS reverse proxy. The Lorca CLI verifies this gateway's HMAC,
seals the envelope to the Runner and durably queues ciphertext before this server
acknowledges GitHub. The gateway needs a running `lorca serve` to drain its outbox.
"""

import argparse
import base64
import hashlib
import hmac
import json
import pathlib
import subprocess
import time
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
    delivery_id = hashlib.sha256(body).hexdigest()
    event = {
        "version": 1,
        "subscription_id": route["subscription_id"],
        "generation": route["generation"],
        "delivery_id": delivery_id,
        "occurred_at": now,
        "event_type": "github.pull_request",
        "payload": body.decode("utf-8"),
    }
    fields = [event[key] for key in (
        "version", "subscription_id", "generation", "delivery_id",
        "occurred_at", "event_type", "payload",
    )]
    signed = json.dumps(fields, ensure_ascii=False, separators=(",", ":")).encode()
    tag = hmac.new(route["secret"].encode(), signed, hashlib.sha256).digest()
    event["signature"] = base64.urlsafe_b64encode(tag).decode().rstrip("=")
    return event


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--route", type=pathlib.Path, required=True)
    parser.add_argument("--github-secret-file", type=pathlib.Path, required=True)
    parser.add_argument("--lorca", default="lorca", help="absolute path recommended")
    parser.add_argument("--bind", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8984)
    args = parser.parse_args()

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            self.connection.settimeout(10)
            if self.path != "/github":
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
                secret = args.github_secret_file.read_bytes().rstrip(b"\r\n")
                if len(secret) < 32:
                    raise ValueError("service secret must have at least 32 bytes")
                event = github_envelope(route, secret, body, self.headers.get("X-Hub-Signature-256", ""), int(time.time()))
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
