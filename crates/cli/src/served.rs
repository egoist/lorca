//! What lorca.app serves for Devices to keep current without a release: the model catalog
//! (`crate::catalog`) and the marketplace index (`crate::marketplace`). Each is a JSON file with
//! the UTC time it was `updated`, which a Device caches in Lorca's folder with its ETag, so a
//! check of a file that has not changed costs a 304.

use std::time::Duration;

use reqwest::header::{ETAG, IF_NONE_MATCH, USER_AGENT};
use reqwest::StatusCode;

use crate::app::App;

/// A check this soon after the last one is skipped, unless forced.
pub const FRESH_SECS: i64 = 60 * 60;
const TIMEOUT: Duration = Duration::from_secs(5);

/// The file at `url` with its ETag, or `None` when it is still the one `etag` names.
pub async fn fetch(app: &App, url: &str, etag: Option<&str>) -> Result<Option<(Option<String>, String)>, String> {
    let mut request = app.http.get(url).timeout(TIMEOUT).header(USER_AGENT, concat!("lorca/", env!("CARGO_PKG_VERSION")));
    if let Some(etag) = etag {
        request = request.header(IF_NONE_MATCH, etag);
    }
    let response = request.send().await.map_err(|e| e.to_string())?;
    if response.status() == StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!("{url} answered {}", response.status()));
    }
    let etag = response.headers().get(ETAG).and_then(|value| value.to_str().ok()).map(str::to_string);
    let text = response.text().await.map_err(|e| e.to_string())?;
    Ok(Some((etag, text)))
}

/// `YYYY-MM-DDTHH:MM:SSZ`, which sorts as the times it names.
pub fn is_utc_time(text: &str) -> bool {
    let pattern = b"dddd-dd-ddTdd:dd:ddZ";
    text.len() == pattern.len() && text.bytes().zip(pattern).all(|(c, p)| if *p == b'd' { c.is_ascii_digit() } else { c == *p })
}

/// Serves `body` at `path` on a free port, with the ETag `"v1"`, or a 304 to a request that sends
/// it back, and keeps each request it got.
#[cfg(test)]
pub(crate) fn test_server(path: &str, body: String) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}{path}", listener.local_addr().unwrap());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for socket in listener.incoming() {
            let Ok(mut socket) = socket else { break };
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            while !request.windows(4).any(|end| end == b"\r\n\r\n") {
                match socket.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => request.extend_from_slice(&buffer[..read]),
                }
            }
            let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
            let reply = if request.contains("\r\nif-none-match: \"v1\"\r\n") {
                "HTTP/1.1 304 Not Modified\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n".to_string()
            } else {
                format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nETag: \"v1\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
            };
            log.lock().unwrap().push(request);
            let _ = socket.write_all(reply.as_bytes());
        }
    });
    (url, seen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_served_time_is_utc_to_the_second() {
        assert!(is_utc_time("2026-10-02T07:29:47Z"));
        assert!(!is_utc_time("2026-10-02T07:29:47+08:00") && !is_utc_time("2026-10-02 07:29:47Z") && !is_utc_time("2026-10-02"));
    }
}
