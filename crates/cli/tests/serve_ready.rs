#![cfg(feature = "cli")]

use std::process::Stdio;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

struct Home(std::path::PathBuf);

impl Home {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("lorca-ready-{}", uuid::Uuid::new_v4())))
    }

    fn serve(&self, port: u16) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lorca"));
        command
            .env_clear()
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
            .stderr(Stdio::null())
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
