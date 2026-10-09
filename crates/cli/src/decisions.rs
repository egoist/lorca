//! Decision models: models that answer typed questions about a state with probabilities and
//! write no text. Two shapes are served: TypeSafe's System One (`state` and a map of questions;
//! TypeSafe, OpenCode Zen, OpenRouter, LLM Gateway) and OpenAI's Decisions API (`input` and a
//! list of questions; OpenAI, Vercel's AI Gateway). Auto-review asks one when the user picks it.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};
use tokio_util::sync::CancellationToken;

/// The shape a decision endpoint speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    SystemOne,
    OpenAi,
}

/// Where and how one decision model is asked.
#[derive(Debug, Clone)]
pub struct Decider {
    pub shape: Shape,
    /// The endpoint itself.
    pub url: String,
    /// Empty for a server that takes no key.
    pub api_key: String,
    pub model: String,
    /// More headers the server wants, such as OpenCode's `User-Agent`.
    pub headers: Vec<(String, String)>,
    /// The header that carries the chat id, for a server that routes by session (OpenCode).
    pub session_header: Option<&'static str>,
}

/// A question that picks one of its choices, each a value and when it applies.
pub struct Choice<'a> {
    pub name: &'a str,
    pub instructions: &'a str,
    pub choices: &'a [(&'a str, &'a str)],
}

impl Decider {
    /// Asks `question` about `state`, answering each choice's probability. A choice the answer
    /// leaves out has none.
    pub async fn choose(
        &self,
        http: &reqwest::Client,
        state: &str,
        question: &Choice<'_>,
        session_id: &str,
        cancel: &CancellationToken,
    ) -> Result<BTreeMap<String, f64>, String> {
        let body = match self.shape {
            Shape::SystemOne => {
                let criteria: Map<String, Value> = question.choices.iter().map(|(value, description)| (value.to_string(), json!(description))).collect();
                json!({
                    "model": self.model,
                    "state": state,
                    "questions": { question.name: { "type": "choice", "instructions": question.instructions, "criteria": criteria } },
                })
            }
            Shape::OpenAi => {
                let choices: Vec<Value> = question.choices.iter().map(|(value, description)| json!({ "value": value, "description": description })).collect();
                json!({
                    "model": self.model,
                    "input": state,
                    "questions": [{ "type": "choice", "name": question.name, "instructions": question.instructions, "choices": choices }],
                })
            }
        };
        let mut request = http.post(&self.url).json(&body);
        if !self.api_key.is_empty() {
            request = request.bearer_auth(&self.api_key);
        }
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if let Some(header) = self.session_header.filter(|_| !session_id.is_empty()) {
            request = request.header(header, session_id);
        }
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err("stopped".into()),
            response = request.send() => response.map_err(|e| lorca_tls::describe(&e))?,
        };
        let status = response.status();
        let text = tokio::select! {
            _ = cancel.cancelled() => return Err("stopped".into()),
            text = response.text() => text.map_err(|e| e.to_string())?,
        };
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(match error_message(&value) {
                Some(message) => format!("{status}: {message}"),
                None => status.to_string(),
            });
        }
        probabilities(&value, question.name).ok_or_else(|| "the decision model answered without the question's choice".into())
    }
}

/// What an error body says: OpenAI's and OpenRouter's `error.message`, TypeSafe's
/// `detail.message`, or a bare `message` or `error`.
fn error_message(body: &Value) -> Option<String> {
    ["/error/message", "/detail/message", "/detail", "/message", "/error"]
        .iter()
        .find_map(|path| body.pointer(path).and_then(Value::as_str))
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
}

/// The probabilities of the answer named `name`, read from either shape: `answers` as a map by
/// name or a list with names, and `probabilities` as a map by value or a list of values. An
/// answer with a `choice` and no probabilities is that choice alone.
fn probabilities(body: &Value, name: &str) -> Option<BTreeMap<String, f64>> {
    let answers = &body["answers"];
    let answer = answers.get(name).or_else(|| answers.as_array()?.iter().find(|answer| answer["name"] == name))?;
    let mut probabilities = BTreeMap::new();
    match &answer["probabilities"] {
        Value::Object(map) => {
            for (value, probability) in map {
                probabilities.insert(value.clone(), probability.as_f64()?);
            }
        }
        Value::Array(list) => {
            for entry in list {
                let value = entry["value"].as_str().map(str::to_string).or_else(|| entry["value"].as_i64().map(|n| n.to_string()))?;
                probabilities.insert(value, entry["probability"].as_f64()?);
            }
        }
        _ => {
            probabilities.insert(answer["choice"].as_str()?.to_string(), 1.0);
        }
    }
    Some(probabilities)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_read_in_either_shape() {
        let system_one = json!({
            "model": "typesafe/jev-1.13",
            "answers": { "verdict": { "type": "choice", "choice": "allow", "probabilities": { "allow": 0.9, "publish": 0.1 }, "confidence": 0.8 } },
        });
        let read = probabilities(&system_one, "verdict").unwrap();
        assert_eq!((read["allow"], read["publish"]), (0.9, 0.1));
        let openai = json!({
            "answers": [{ "type": "choice", "name": "verdict", "choice": "publish", "probabilities": [{ "value": "allow", "probability": 0.2 }, { "value": "publish", "probability": 0.8 }] }],
        });
        assert_eq!(probabilities(&openai, "verdict").unwrap()["publish"], 0.8);
        let bare = json!({ "answers": { "verdict": { "choice": "allow" } } });
        assert_eq!(probabilities(&bare, "verdict").unwrap()["allow"], 1.0);
        assert!(probabilities(&json!({ "answers": {} }), "verdict").is_none());
        assert!(probabilities(&json!({ "error": "no" }), "verdict").is_none());
    }

    #[test]
    fn errors_say_what_the_server_said() {
        assert_eq!(error_message(&json!({ "error": { "message": "A valid API key is required." } })).as_deref(), Some("A valid API key is required."));
        assert_eq!(error_message(&json!({ "detail": { "message": "Must supply an API key!" } })).as_deref(), Some("Must supply an API key!"));
        assert_eq!(error_message(&json!({ "detail": "Not found" })).as_deref(), Some("Not found"));
        assert_eq!(error_message(&Value::Null), None);
    }

    /// Answers one request with `body`, and hands back the request it read.
    fn serve(body: Value) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/decide", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 8192];
            // Read the head, then as much body as it says.
            loop {
                let read = socket.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&request).to_string();
                if let Some(head_end) = text.find("\r\n\r\n") {
                    let length = text[..head_end]
                        .lines()
                        .find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").map(|n| n.trim().parse::<usize>().unwrap()))
                        .unwrap_or(0);
                    if request.len() >= head_end + 4 + length {
                        break;
                    }
                }
            }
            let body = body.to_string();
            let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            socket.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&request).to_string()
        });
        (url, server)
    }

    #[tokio::test]
    async fn each_shape_asks_its_own_way() {
        let question = Choice { name: "verdict", instructions: "May it run?", choices: &[("allow", "Safe."), ("publish", "Others will see it.")] };
        let http = reqwest::Client::new();
        let cancel = CancellationToken::new();

        let (url, server) = serve(json!({ "answers": { "verdict": { "type": "choice", "choice": "allow", "probabilities": { "allow": 0.7, "publish": 0.3 } } } }));
        let decider = Decider {
            shape: Shape::SystemOne, url, api_key: "zen".into(), model: "jev-1.13".into(),
            headers: vec![("User-Agent".into(), "lorca/test".into())], session_header: Some("x-opencode-session"),
        };
        assert_eq!(decider.choose(&http, "state", &question, "chat-1", &cancel).await.unwrap()["allow"], 0.7);
        let request = server.join().unwrap();
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains("authorization: bearer zen") && lower.contains("x-opencode-session: chat-1") && lower.contains("user-agent: lorca/test"), "{request}");
        let body: Value = serde_json::from_str(&request[request.find("\r\n\r\n").unwrap() + 4..]).unwrap();
        assert_eq!(body["state"], "state");
        assert_eq!(body["questions"]["verdict"]["criteria"]["publish"], "Others will see it.");

        let (url, server) = serve(json!({ "answers": [{ "name": "verdict", "choice": "publish", "probabilities": [{ "value": "allow", "probability": 0.1 }, { "value": "publish", "probability": 0.9 }] }] }));
        let decider = Decider { shape: Shape::OpenAi, url, api_key: String::new(), model: "gpt-6-luna".into(), headers: Vec::new(), session_header: None };
        assert_eq!(decider.choose(&http, "state", &question, "chat-1", &cancel).await.unwrap()["publish"], 0.9);
        let request = server.join().unwrap();
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
        let body: Value = serde_json::from_str(&request[request.find("\r\n\r\n").unwrap() + 4..]).unwrap();
        assert_eq!((body["input"].as_str(), body["questions"][0]["name"].as_str()), (Some("state"), Some("verdict")));
        assert_eq!(body["questions"][0]["choices"][1], json!({ "value": "publish", "description": "Others will see it." }));
    }
}
