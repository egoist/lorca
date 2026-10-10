#!/usr/bin/env python3
"""Gateway contract tests; --lorca PATH also tests HTTP -> local CLI -> encrypted inbox."""
import argparse
import base64
import hashlib
import hmac
import importlib.util
import json
import os
import pathlib
import socket
import sqlite3
import stat
import subprocess
import tempfile
import time
import unittest
import urllib.error
import urllib.request

spec = importlib.util.spec_from_file_location("event_gateway", pathlib.Path(__file__).with_name("event-gateway.py"))
gateway = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gateway)


class GatewayContract(unittest.TestCase):
    def setUp(self):
        self.route = {"subscription_id": "ev-example", "generation": 1, "secret": "gateway-secret"}
        self.secret = b"a-private-service-secret-with-entropy"
        self.body = json.dumps({"pull_request": {"title": "こんにちは 🌱"}}, ensure_ascii=False).encode()
        self.signature = "sha256=" + hmac.new(self.secret, self.body, hashlib.sha256).hexdigest()

    def test_provider_signature_binds_original_bytes(self):
        for bad_body, bad_signature in [(self.body + b" ", self.signature), (self.body, ""), (self.body, "sha256=" + "0" * 64)]:
            with self.assertRaises(ValueError):
                gateway.github_envelope(self.route, self.secret, bad_body, bad_signature, 1700000000)

    def test_signed_body_supplies_dedup_and_kind(self):
        event = gateway.github_envelope(self.route, self.secret, self.body, self.signature, 1700000000)
        later = gateway.github_envelope(self.route, self.secret, self.body, self.signature, 1700000001)
        self.assertEqual(event["delivery_id"], hashlib.sha256(self.body).hexdigest())
        self.assertEqual(event["delivery_id"], later["delivery_id"])
        self.assertEqual(event["event_type"], "github.pull_request")
        self.assertEqual(event["payload"], self.body.decode())
        fields = [event[k] for k in ("version", "subscription_id", "generation", "delivery_id", "occurred_at", "event_type", "payload")]
        signed = json.dumps(fields, ensure_ascii=False, separators=(",", ":")).encode()
        expected = base64.urlsafe_b64encode(hmac.new(b"gateway-secret", signed, hashlib.sha256).digest()).decode().rstrip("=")
        self.assertEqual(event["signature"], expected)

    def test_a_signed_ping_is_answered_without_an_event(self):
        body = b'{"zen":"Keep it logically awesome.","hook_id":1}'
        signature = "sha256=" + hmac.new(self.secret, body, hashlib.sha256).hexdigest()
        self.assertIsNone(gateway.github_envelope(self.route, self.secret, body, signature, 1700000000))
        with self.assertRaises(ValueError):
            gateway.github_envelope(self.route, self.secret, body, "sha256=" + "0" * 64, 1700000000)

    def test_another_service_event_is_refused(self):
        body = b'{"message":"not a pull request"}'
        signature = "sha256=" + hmac.new(self.secret, body, hashlib.sha256).hexdigest()
        with self.assertRaises(ValueError):
            gateway.github_envelope(self.route, self.secret, body, signature, 1700000000)

    def test_a_routine_webhook_takes_any_body_from_its_key_holder(self):
        key = b"a-private-routine-webhook-key-with-entropy"
        headers = {"Authorization": "Bearer " + key.decode()}
        event = gateway.webhook_envelope(self.route, key, b'{"status":"deployed"}', headers, 1700000000)
        self.assertEqual((event["event_type"], event["payload"]), ("webhook", '{"status":"deployed"}'))
        text = gateway.webhook_envelope(self.route, key, "plain words 🌱".encode(), {"X-Lorca-Key": key.decode()}, 1700000000)
        self.assertEqual(json.loads(text["payload"]), "plain words 🌱")
        self.assertNotEqual(event["delivery_id"], gateway.webhook_envelope(self.route, key, b'{"status":"deployed"}', headers, 1700000000)["delivery_id"])
        repeat = dict(headers, **{"Idempotency-Key": "deploy-7"})
        self.assertEqual(
            gateway.webhook_envelope(self.route, key, b"{}", repeat, 1700000000)["delivery_id"],
            gateway.webhook_envelope(self.route, key, b"{}", repeat, 1700000001)["delivery_id"],
        )
        for bad in [{}, {"Authorization": "Bearer wrong"}, {"X-Lorca-Key": key.decode() + "x"}]:
            with self.assertRaises(ValueError):
                gateway.webhook_envelope(self.route, key, b"{}", bad, 1700000000)


def free_port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def smoke(binary):
    with tempfile.TemporaryDirectory(prefix="lorca-event-smoke-") as folder:
        home = pathlib.Path(folder)
        port, gateway_port = free_port(), free_port()
        env = dict(os.environ, LORCA_HOME=str(home), LORCA_PORT=str(port), LORCA_RELAY_URL="")
        env.pop("LORCA_DEFAULT_RELAY_URL", None)
        env.pop("LORCA_DEV", None)
        def cli(*args):
            result = subprocess.run([binary, *args], env=env, capture_output=True, timeout=20)
            if result.returncode:
                raise AssertionError("Lorca CLI failed: " + " ".join(args[:2]))
            return result.stdout.decode()
        # Identity output includes a backup phrase; capture it and keep it out of test logs.
        cli("identity", "new")
        with sqlite3.connect(home / "lorca.sqlite3") as db:
            bot = json.loads(db.execute("SELECT json FROM bots LIMIT 1").fetchone()[0])["id"]
        config = {
            "name": "PR smoke", "source": "gateway_hmac", "bot_id": bot,
            "prompt": "Summarize PR activity.", "event_types": ["github.pull_request"],
            "filters": [{"pointer": "/repository/full_name", "equals": "acme/project"}],
            "is_enabled": False,
        }
        config_file = home / "subscription.json"
        config_file.write_text(json.dumps(config))
        sub = json.loads(cli("events", "add", str(config_file)))
        route_file = home / "route.json"
        cli("events", "route", sub["id"], str(route_file))
        if os.name == "posix":
            assert stat.S_IMODE(route_file.stat().st_mode) == 0o600
        exists = subprocess.run([binary, "events", "route", sub["id"], str(route_file)], env=env, capture_output=True, timeout=20)
        assert exists.returncode != 0, "route export must not overwrite a private file"
        secret = b"a-random-service-secret-of-at-least-32-bytes"
        secret_file = home / "service-secret"
        secret_file.write_bytes(secret)
        serve = subprocess.Popen([binary, "serve", "--ready-stdout"], env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        adapter = None
        try:
            for _ in range(100):
                with socket.socket() as s:
                    if s.connect_ex(("127.0.0.1", port)) == 0:
                        break
                time.sleep(0.05)
            else:
                raise AssertionError("local CLI did not listen")
            adapter = subprocess.Popen([
                "python3", str(pathlib.Path(__file__).with_name("event-gateway.py")),
                "--route", str(route_file), "--github-secret-file", str(secret_file),
                "--lorca", binary, "--port", str(gateway_port),
            ], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            for _ in range(100):
                with socket.socket() as s:
                    if s.connect_ex(("127.0.0.1", gateway_port)) == 0:
                        break
                time.sleep(0.05)
            else:
                raise AssertionError("gateway did not listen")
            body = json.dumps({"pull_request": {"title": "こんにちは 🌱"}, "repository": {"full_name": "acme/project"}}, ensure_ascii=False).encode()
            signature = "sha256=" + hmac.new(secret, body, hashlib.sha256).hexdigest()
            def post(sig, delivery):
                request = urllib.request.Request(f"http://127.0.0.1:{gateway_port}/github", data=body, headers={
                    "Content-Type": "application/json", "X-Hub-Signature-256": sig,
                    "X-GitHub-Delivery": delivery, "X-GitHub-Event": "spoofed-header",
                })
                try:
                    with urllib.request.urlopen(request, timeout=20) as response:
                        return response.status
                except urllib.error.HTTPError as error:
                    return error.code
            assert post("sha256=" + "0" * 64, "bad") == 403
            assert post(signature, "first") == 202
            assert post(signature, "another-unsigned-delivery-id") == 202
            listing = json.loads(cli("events", "list"))["subscriptions"][0]
            assert listing["state"] == "paused" and listing["pending"] == 1
            assert len(listing["deliveries"]) == 1
            cli("events", "resume", sub["id"])
            for _ in range(100):
                listing = json.loads(cli("events", "list"))["subscriptions"][0]
                if listing["health"]["last_outcome"] == "error":
                    break
                time.sleep(0.05)
            else:
                raise AssertionError("event turn did not reach the normal no-provider result")
            assert listing["state"] == "attention" and listing["pending"] == 0
            with sqlite3.connect(home / "lorca.sqlite3") as db:
                assert db.execute("SELECT COUNT(*) FROM event_inbox").fetchone()[0] == 1
                messages = [json.loads(row[0]) for row in db.execute("SELECT message_json FROM messages")]
                assert sum(m.get("body", {}).get("text", "").startswith("Event ·") for m in messages) == 1
            print("HTTP gateway -> local CLI -> encrypted inbox -> unattended turn: passed")
        finally:
            for process in [adapter, serve]:
                if process:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()


def smoke_webhook(binary):
    """A routine's webhook: any body from the key's holder reaches the routine's inbox."""
    with tempfile.TemporaryDirectory(prefix="lorca-webhook-smoke-") as folder:
        home = pathlib.Path(folder)
        port, gateway_port = free_port(), free_port()
        env = dict(os.environ, LORCA_HOME=str(home), LORCA_PORT=str(port), LORCA_RELAY_URL="")
        env.pop("LORCA_DEFAULT_RELAY_URL", None)
        env.pop("LORCA_DEV", None)
        def cli(*args):
            result = subprocess.run([binary, *args], env=env, capture_output=True, timeout=20)
            if result.returncode:
                raise AssertionError("Lorca CLI failed: " + " ".join(args[:2]))
            return result.stdout.decode()
        cli("identity", "new")
        with sqlite3.connect(home / "lorca.sqlite3") as db:
            bot = json.loads(db.execute("SELECT json FROM bots LIMIT 1").fetchone()[0])["id"]
        config = {
            "name": "Deploys", "source": "gateway_hmac", "bot_id": bot,
            "prompt": "Tell me when a deploy fails.", "event_types": ["webhook"], "filters": [], "is_enabled": False,
        }
        config_file = home / "subscription.json"
        config_file.write_text(json.dumps(config))
        sub = json.loads(cli("events", "add", str(config_file)))
        route_file = home / "route.json"
        cli("events", "route", sub["id"], str(route_file))
        key = b"a-random-routine-webhook-key-of-32-bytes"
        key_file = home / "webhook-key"
        key_file.write_bytes(key)
        serve = subprocess.Popen([binary, "serve", "--ready-stdout"], env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        adapter = None
        try:
            for listening in [port, None]:
                if listening is None:
                    adapter = subprocess.Popen([
                        "python3", str(pathlib.Path(__file__).with_name("event-gateway.py")),
                        "--route", str(route_file), "--webhook-key-file", str(key_file),
                        "--lorca", binary, "--port", str(gateway_port),
                    ], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    listening = gateway_port
                for _ in range(100):
                    with socket.socket() as s:
                        if s.connect_ex(("127.0.0.1", listening)) == 0:
                            break
                    time.sleep(0.05)
                else:
                    raise AssertionError("nothing listened")
            def post(headers, body=b'{"deploy":"failed"}'):
                request = urllib.request.Request(f"http://127.0.0.1:{gateway_port}/hook", data=body, headers=headers)
                try:
                    with urllib.request.urlopen(request, timeout=20) as response:
                        return response.status
                except urllib.error.HTTPError as error:
                    return error.code
            assert post({"Authorization": "Bearer wrong"}) == 403
            assert post({"Authorization": "Bearer " + key.decode(), "Idempotency-Key": "deploy-7"}) == 202
            assert post({"X-Lorca-Key": key.decode(), "Idempotency-Key": "deploy-7"}) == 202
            assert post({"X-Lorca-Key": key.decode()}, b"plain text") == 202
            listing = json.loads(cli("events", "list"))["subscriptions"][0]
            assert listing["pending"] == 2, listing
            print("routine webhook -> local CLI -> encrypted inbox: passed")
        finally:
            for process in [adapter, serve]:
                if process:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lorca")
    args = parser.parse_args()
    outcome = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(GatewayContract))
    if not outcome.wasSuccessful():
        raise SystemExit(1)
    if args.lorca:
        smoke(str(pathlib.Path(args.lorca).resolve()))
        smoke_webhook(str(pathlib.Path(args.lorca).resolve()))
