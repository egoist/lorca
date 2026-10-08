# Native Go/MyGo routine reliability evidence

These images are native MyGo `ui.NewTester` offscreen frames from the Windows/Linux app's Go views, rendered on macOS with synthetic demo account/chat/Runner data. They show the shared native toolkit, without a live Windows/Linux OS window or installed service. Runner names, metadata, chat messages and the service status are fixture values.

The routine scenes show explicit New York timezone/offset, Run once/Skip policies, separate successful-check history with Last run: Never, failure/backoff recovery, authentication-blocked Resume, and an unavailable assigned Runner. The timezone editor holds an Asia/Singapore draft across redraws. The service scenes show a mocked read-only Not installed response and an offline Runner retaining its setup commands. Light and dark appearances are captured.

Reproduce:

```sh
mkdir -p docs/review/issue-78/desktop
cd desktop
LORCA_RENDER=../docs/review/issue-78/desktop go test . -run 'TestRoutineReliability|TestRunnerService' -count=1
```

The captures use the production native view functions, keyed fields, store mapping, and main-thread mock reply queue. The action tests also exercise timezone/policy editing, disabled run controls, persistent drafts, stale service replies after a Device change, and visible actions in a short window. No real credentials, private chats, provider/relay requests, check execution, or service installation is involved. Evidence files stay outside production bundle resources.

Native Windows/Linux window interaction, installers, real paired outages, provider/integration reauthentication, service lifecycle, and combined sibling budget/event/template workflows remain untested. Standalone Windows x64 and Linux x64/arm64 Go binaries are cross-built separately; these images do not imply live-platform validation.
