#![cfg(feature = "cli")]

use std::process::Stdio;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

struct Home(std::path::PathBuf);

impl Home {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("lorca-ready-{}", uuid::Uuid::new_v4())))
    }

    fn serve(&self, port: u16) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lorca"));
        command.env_clear();
        // Windows loads its network stack from under %SystemRoot%: without the variable, binding a
        // socket fails with WSAEPROVIDERFAILEDINIT.
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
        command
            .env("RUST_LOG", "off")
            .args([
                "serve",
                "--ready-stdout",
                "--port",
                &port.to_string(),
                "--home",
            ])
            .arg(&self.0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn readiness_is_flushed_with_logs_disabled_and_the_websocket_is_ready() {
    let home = Home::new();
    let mut child = home.serve(0).spawn().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    timeout(Duration::from_secs(5), stdout.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    if line.is_empty() {
        let mut stderr = String::new();
        let _ = timeout(Duration::from_secs(5), child.stderr.take().unwrap().read_to_string(&mut stderr)).await;
        panic!("lorca serve exited before it was ready ({:?}): {stderr}", child.wait().await);
    }
    let ready: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["event"], "ready");
    let port = ready["port"].as_u64().unwrap();
    assert!(port > 0);
    assert!(child.try_wait().unwrap().is_none());

    let (mut socket, _) = timeout(
        Duration::from_secs(2),
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")),
    )
    .await
    .unwrap()
    .unwrap();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({ "id": 7, "method": "bootstrap", "params": {} })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let response = timeout(Duration::from_secs(2), async {
        loop {
            let frame = socket.next().await.unwrap().unwrap();
            if let Ok(text) = frame.to_text() {
                let value: Value = serde_json::from_str(text).unwrap();
                if value["id"] == 7 {
                    break value;
                }
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(response["result"]["has_identity"], false);
    child.kill().await.unwrap();
}

#[tokio::test]
async fn a_build_wrappers_message_format_is_ignored() {
    // `cargo run` hands the tokens after the program's name to the program, and a build
    // wrapper (mbx) adds its own `--message-format=…` at the end of that command line, so
    // `cargo run -p lorca serve` reaches serve with the flag it never asked for.
    let home = Home::new();
    let mut command = home.serve(0);
    command.arg("--message-format=json,json-diagnostic-rendered-ansi");
    let mut child = command.spawn().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    timeout(Duration::from_secs(5), stdout.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    assert!(!line.is_empty(), "lorca serve exited before it was ready: {:?}", child.wait().await);
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["event"], "ready");
    child.kill().await.unwrap();
}

#[tokio::test]
async fn a_failed_bind_exits_without_announcing_readiness() {
    let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let home = Home::new();
    let output = timeout(
        Duration::from_secs(5),
        home.serve(occupied.local_addr().unwrap().port()).output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[tokio::test]
async fn bare_lorca_lists_the_commands_and_starts_nothing() {
    let home = Home::new();
    let output = Command::new(env!("CARGO_BIN_EXE_lorca")).env("LORCA_HOME", &home.0).env("RUST_LOG", "off").stdin(Stdio::null()).output();
    let output = timeout(Duration::from_secs(10), output).await.expect("it returns rather than serving").unwrap();
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("Usage: lorca") && help.contains("serve") && help.contains("mcp"), "{help}");
    assert!(!home.0.exists(), "a help page makes no data folder");
}
