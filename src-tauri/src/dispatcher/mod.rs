//! Command dispatcher — the pluggable bridge between transcribed speech and the
//! agent backend. Defaults target a local **OpenClaw** Gateway (the "open
//! claw" referenced in the architecture doc), but the backend is intentionally
//! swappable — any OpenAI-compatible chat-completions endpoint works.
//! Everything the rest of the app touches goes through the [`CommandBackend`]
//! trait, so swapping the integration never has to touch the voice pipeline.

use async_trait::async_trait;
use std::fmt;

pub mod http;

/// An error produced while dispatching a command to the backend.
#[derive(Debug)]
pub struct DispatchError(pub String);

impl fmt::Display for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "dispatch error: {}", self.0)
    }
}

impl std::error::Error for DispatchError {}

/// A swappable command backend. Implementations forward transcribed text to
/// whatever actually fulfils the request and return a textual response.
#[async_trait]
pub trait CommandBackend: Send + Sync {
    async fn dispatch(&self, text: String) -> Result<String, DispatchError>;
}

/// Build the backend from the current agent settings (Settings panel, backed
/// by `crate::agent_settings`). The `OpenClaw` preset always uses the
/// hardcoded OpenClaw defaults for URL/model, ignoring whatever's saved in
/// those fields; `Custom` uses them as saved. Set the URL to `echo` for an
/// offline loopback test — the same shortcut the old `JARVIS_AGENT_URL=echo`
/// env var gave.
pub fn backend_for(settings: &crate::agent_settings::AgentSettings) -> Box<dyn CommandBackend> {
    use crate::agent_settings::{AgentPreset, DEFAULT_AGENT_MODEL, DEFAULT_AGENT_URL};

    let (url, model) = match settings.preset {
        AgentPreset::OpenClaw => (DEFAULT_AGENT_URL.to_string(), DEFAULT_AGENT_MODEL.to_string()),
        AgentPreset::Custom => (settings.url.clone(), settings.model.clone()),
    };
    if url.trim() == "echo" {
        return Box::new(EchoBackend);
    }
    Box::new(http::HttpBackend::new(url, settings.key.clone(), model))
}

/// Fallback stub backend: echoes the transcript back so the full state machine
/// (`IDLE → … → RESPONDING → IDLE`) can be exercised end to end even before an
/// external agent API is configured.
pub struct EchoBackend;

#[async_trait]
impl CommandBackend for EchoBackend {
    async fn dispatch(&self, text: String) -> Result<String, DispatchError> {
        if text.trim().is_empty() {
            return Err(DispatchError("empty transcript".into()));
        }
        Ok(format!("You said: \"{}\"", text.trim()))
    }
}
