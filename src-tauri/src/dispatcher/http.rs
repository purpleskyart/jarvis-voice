//! HTTP command backend — sends the transcript to the agent gateway and
//! returns its text response, which the front end then speaks aloud (TTS).
//!
//! Speaks plain **OpenAI-compatible chat completions**: `POST
//! /v1/chat/completions` with a `messages` array, returning
//! `choices[0].message.content`. Defaults target a local **OpenClaw Gateway**
//! (`model: "openclaw/default"`), but any compatible endpoint works — Kimi /
//! Moonshot direct, or something else entirely. Configurable from the
//! Settings panel, or via env var at startup:
//!   - `JARVIS_AGENT_URL`   — endpoint (default `http://127.0.0.1:18789/v1/chat/completions`)
//!   - `JARVIS_AGENT_KEY`   — optional bearer token
//!   - `JARVIS_AGENT_MODEL` — model / agent target (default `openclaw/default`)

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{CommandBackend, DispatchError};

const SYSTEM_PROMPT: &str =
    "You are Jarvis, a concise spoken voice assistant. Answer in one or two short \
     sentences suitable for being read aloud. Avoid markdown, lists, and code blocks.";

pub struct HttpBackend {
    endpoint: String,
    api_key: Option<String>,
    model: String,
    client: reqwest::Client,
}

impl HttpBackend {
    pub fn new(endpoint: String, api_key: Option<String>, model: String) -> Self {
        Self {
            endpoint,
            api_key,
            model,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl CommandBackend for HttpBackend {
    async fn dispatch(&self, text: String) -> Result<String, DispatchError> {
        // Kimi / Moonshot (OpenAI-compatible) chat-completions request.
        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": SYSTEM_PROMPT },
                { "role": "user", "content": text }
            ],
            "temperature": 0.6,
            "stream": false
        });

        let mut req = self.client.post(&self.endpoint).json(&body);
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }

        let resp = req
            .send()
            .await
            .map_err(|e| DispatchError(format!("request to {} failed: {e}", self.endpoint)))?;
        let status = resp.status();
        let raw = resp
            .text()
            .await
            .map_err(|e| DispatchError(format!("read body failed: {e}")))?;

        if !status.is_success() {
            return Err(DispatchError(format!("agent API {status}: {raw}")));
        }
        extract_reply(&raw).ok_or_else(|| DispatchError(format!("unexpected response: {raw}")))
    }
}

/// Pull the assistant text out of a chat-completions response, accepting the
/// Kimi/OpenAI shape and a few common fallbacks. Returns `None` only if the
/// body is JSON with no recognizable text field.
fn extract_reply(body: &str) -> Option<String> {
    match serde_json::from_str::<Value>(body) {
        Ok(v) => {
            // Kimi / OpenAI: choices[0].message.content
            for ptr in [
                "/choices/0/message/content",
                "/choices/0/text",
                "/content/0/text", // Anthropic-style
                "/message/content",
            ] {
                if let Some(s) = v.pointer(ptr).and_then(|x| x.as_str()) {
                    return Some(s.trim().to_string());
                }
            }
            // Flat shapes: { response | text | reply | ... }
            for key in ["response", "text", "reply", "message", "content", "answer"] {
                if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
                    return Some(s.trim().to_string());
                }
            }
            // Surface an API error object rather than silently failing.
            if let Some(msg) = v.pointer("/error/message").and_then(|x| x.as_str()) {
                return Some(format!("Agent error: {}", msg.trim()));
            }
            None
        }
        // Not JSON → treat as a plain-text endpoint.
        Err(_) => Some(body.trim().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn dispatch_round_trips_against_local_kimi_server() {
        // A mock gateway on 127.0.0.1 that replies in the Kimi/OpenAI shape.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = r#"{"choices":[{"message":{"content":"It is sunny."}}]}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let backend = HttpBackend::new(
            format!("http://{addr}/v1/chat/completions"),
            None,
            "moonshot-v1-8k".into(),
        );
        let out = tauri::async_runtime::block_on(backend.dispatch("weather?".into())).unwrap();
        assert_eq!(out, "It is sunny.");
    }

    #[test]
    fn parses_kimi_and_common_shapes() {
        assert_eq!(
            extract_reply(r#"{"choices":[{"message":{"content":"kimi reply"}}]}"#).unwrap(),
            "kimi reply"
        );
        assert_eq!(
            extract_reply(r#"{"content":[{"text":"claude reply"}]}"#).unwrap(),
            "claude reply"
        );
        assert_eq!(extract_reply(r#"{"response":" hi "}"#).unwrap(), "hi");
        assert_eq!(extract_reply("plain text").unwrap(), "plain text");
        assert_eq!(
            extract_reply(r#"{"error":{"message":"bad key"}}"#).unwrap(),
            "Agent error: bad key"
        );
        assert!(extract_reply(r#"{"unknown":1}"#).is_none());
    }
}
