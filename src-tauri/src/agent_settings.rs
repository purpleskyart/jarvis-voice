//! Persisted, user-editable agent-gateway settings (Settings panel → Agent).
//!
//! Defaults target a local OpenClaw Gateway (`openclaw gateway --port 18789`,
//! its default port) via its OpenAI-compatible `/v1/chat/completions` surface
//! — `model: "openclaw/default"` is OpenClaw's stable alias for "whatever the
//! configured default agent is" (see docs/gateway/openai-http-api.md in the
//! OpenClaw repo). That endpoint is disabled by default on the gateway side;
//! enable it with `gateway.http.endpoints.chatCompletions.enabled: true`.
//!
//! Any other OpenAI-compatible endpoint (Kimi/Moonshot direct, etc.) works
//! too — just point the URL/model elsewhere from the Settings panel. Defaults
//! come from the `JARVIS_AGENT_*` env vars; whatever the user saves in the UI
//! overrides those and survives restarts as a small JSON file in the app's
//! config dir.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

pub const DEFAULT_AGENT_URL: &str = "http://127.0.0.1:18789/v1/chat/completions";
pub const DEFAULT_AGENT_MODEL: &str = "openclaw/default";

/// Which URL/model the dispatcher actually uses (see `dispatcher::backend_for`).
/// `OpenClaw` always dispatches to the hardcoded OpenClaw defaults above,
/// regardless of whatever's saved in `url`/`model` — so switching back to it
/// from `Custom` is always correct even with stale saved fields. `Custom`
/// uses `url`/`model` as saved. The API key applies in both cases.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AgentPreset {
    #[default]
    OpenClaw,
    Custom,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AgentSettings {
    #[serde(default)]
    pub preset: AgentPreset,
    pub url: String,
    #[serde(default)]
    pub key: Option<String>,
    pub model: String,
}

impl Default for AgentSettings {
    fn default() -> Self {
        let url = std::env::var("JARVIS_AGENT_URL").ok().filter(|u| !u.trim().is_empty());
        let model = std::env::var("JARVIS_AGENT_MODEL").ok().filter(|m| !m.trim().is_empty());
        // An env var pointing anywhere other than the OpenClaw defaults means
        // the user already intends a custom endpoint.
        let is_custom = url.as_deref().is_some_and(|u| u != DEFAULT_AGENT_URL)
            || model.as_deref().is_some_and(|m| m != DEFAULT_AGENT_MODEL);
        Self {
            preset: if is_custom { AgentPreset::Custom } else { AgentPreset::OpenClaw },
            url: url.unwrap_or_else(|| DEFAULT_AGENT_URL.to_string()),
            key: std::env::var("JARVIS_AGENT_KEY")
                .ok()
                .filter(|k| !k.is_empty())
                .or_else(|| {
                    // Auto-detect OpenClaw gateway token from config
                    std::env::var("OPENCLAW_GATEWAY_TOKEN").ok().filter(|k| !k.is_empty())
                }),
            model: model.unwrap_or_else(|| DEFAULT_AGENT_MODEL.to_string()),
        }
    }
}

fn config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("agent_settings.json"))
}

impl AgentSettings {
    /// Load the persisted file if present, else fall back to env-var defaults.
    pub fn load(app: &AppHandle) -> Self {
        config_path(app)
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, app: &AppHandle) -> Result<(), String> {
        let path = config_path(app).ok_or("no app config directory available")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(path, json).map_err(|e| e.to_string())
    }
}
