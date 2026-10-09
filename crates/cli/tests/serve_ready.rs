#![cfg(feature = "cli")]

use std::process::Stdio;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

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

    let token = std::fs::read_to_string(home.0.join("serve-token")).unwrap();
    let mut request = format!("ws://127.0.0.1:{port}/ws").into_client_request().unwrap();
    request.headers_mut().insert("Authorization", format!("Bearer {token}").parse().unwrap());
    let (mut socket, _) = timeout(
        Duration::from_secs(2),
        tokio_tungstenite::connect_async(request),
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

/// Starts `lorca serve` on a free port and answers the port once it is ready.
async fn ready(home: &Home) -> (tokio::process::Child, u16) {
    let mut child = home.serve(0).spawn().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    timeout(Duration::from_secs(5), stdout.read_line(&mut line)).await.unwrap().unwrap();
    let ready: Value = serde_json::from_str(&line).unwrap();
    (child, ready["port"].as_u64().unwrap() as u16)
}

/// The status line's code for a websocket upgrade of `path` with these extra headers.
async fn handshake(port: u16, path: &str, headers: &[(&str, String)]) -> u16 {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut request = format!("GET {path} HTTP/1.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n");
    if !headers.iter().any(|(name, _)| *name == "Host") {
        request.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    }
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut line = String::new();
    timeout(Duration::from_secs(2), BufReader::new(stream).read_line(&mut line)).await.unwrap().unwrap();
    line.split(' ').nth(1).unwrap().parse().unwrap()
}

#[tokio::test]
async fn only_a_local_client_with_the_token_gets_the_websocket() {
    let home = Home::new();
    let (mut child, port) = ready(&home).await;
    let path = home.0.join("serve-token");
    let token = std::fs::read_to_string(&path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let bearer = format!("Bearer {token}");
    let authorized = ("Authorization", bearer.clone());

    assert_eq!(handshake(port, "/ws", std::slice::from_ref(&authorized)).await, 101);
    assert_eq!(handshake(port, "/ws", &[("Host", format!("localhost:{port}")), authorized.clone()]).await, 101);
    assert_eq!(handshake(port, "/ws", &[]).await, 401, "no token");
    assert_eq!(handshake(port, "/ws", &[("Authorization", "Bearer wrong".into())]).await, 401, "a wrong token");
    assert_eq!(handshake(port, "/ws", &[("Authorization", token.clone())]).await, 401, "the token without its scheme");
    assert_eq!(handshake(port, "/ws", &[authorized.clone(), ("Origin", "https://example.com".into())]).await, 403, "a web page");
    assert_eq!(handshake(port, "/ws", &[authorized.clone(), ("Origin", "null".into())]).await, 403, "a sandboxed page");
    assert_eq!(handshake(port, "/ws", &[("Host", format!("attacker.example:{port}")), authorized.clone()]).await, 403, "a DNS-rebound page");
    assert_eq!(handshake(port, "/ws", &[("Host", "127.0.0.1".into()), authorized.clone()]).await, 403, "another port");
    // The check that the port answers: open to local clients only, too.
    assert_eq!(handshake(port, "/", &[]).await, 200);
    assert_eq!(handshake(port, "/", &[("Host", format!("attacker.example:{port}"))]).await, 403);
    assert_eq!(handshake(port, "/", &[("Origin", "https://example.com".into())]).await, 403);
    child.kill().await.unwrap();

    // A later start keeps the token, so a client that read it still connects.
    let (mut child, port) = ready(&home).await;
    assert_eq!(std::fs::read_to_string(&path).unwrap(), token);
    assert_eq!(handshake(port, "/ws", &[authorized]).await, 101);
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
