import { useEffect, useState } from "react";
import { useVoiceStore } from "../state/voiceStore";
import { getAgentSettings, setAgentSettings } from "../lib/tauriEvents";
import type { AgentSettings } from "../types";

const OPENCLAW_URL = "http://127.0.0.1:18789/v1/chat/completions";
const OPENCLAW_MODEL = "openclaw/default";

const EMPTY_AGENT: AgentSettings = { preset: "openclaw", url: "", key: "", model: "" };

/**
 * Settings panel. The speak/TTS toggle and agent gateway fields are live; the
 * mic-device and Whisper-model controls activate as Phases 3–5 (VAD, wake
 * word) land.
 */
export function SettingsPanel({ onClose }: { onClose: () => void }) {
  const speakEnabled = useVoiceStore((s) => s.speakEnabled);
  const toggleSpeak = useVoiceStore((s) => s.toggleSpeak);

  const [agent, setAgent] = useState<AgentSettings>(EMPTY_AGENT);
  const [saved, setSaved] = useState(true);

  useEffect(() => {
    getAgentSettings().then((s) => setAgent({ ...s, key: s.key ?? "" }));
  }, []);

  const isOpenClaw = agent.preset === "openclaw";

  const persist = (next: AgentSettings) => {
    setSaved(false);
    setAgentSettings({ ...next, key: next.key || null }).then(() => setSaved(true));
  };

  const field = (key: "url" | "key" | "model") => ({
    value: agent[key] ?? "",
    onChange: (e: React.ChangeEvent<HTMLInputElement>) => {
      setAgent((prev) => ({ ...prev, [key]: e.target.value }));
    },
    onBlur: () => persist(agent),
  });

  const handlePresetChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const next = { ...agent, preset: e.target.value as AgentSettings["preset"] };
    setAgent(next);
    persist(next);
  };

  return (
    <div className="settings">
      <div className="settings__head">
        <span>Settings</span>
        <button className="icon-btn" onClick={onClose} aria-label="Close settings">
          ✕
        </button>
      </div>

      <label className="field field--row">
        <span>Speak responses (TTS)</span>
        <button
          className={"switch" + (speakEnabled ? " switch--on" : "")}
          onClick={toggleSpeak}
          role="switch"
          aria-checked={speakEnabled}
        >
          <span className="switch__knob" />
        </button>
      </label>

      <label className="field">
        <span>Microphone</span>
        <select disabled defaultValue="default">
          <option value="default">System default</option>
        </select>
      </label>

      <label className="field">
        <span>Whisper model</span>
        <select disabled defaultValue="base.en">
          <option value="tiny.en">tiny.en — fastest</option>
          <option value="base.en">base.en — recommended</option>
          <option value="small.en">small.en — most accurate</option>
        </select>
      </label>

      <div className="settings__section">
        <div className="settings__section-head">
          <strong>Agent gateway</strong>
          <span className="settings__saved">{saved ? "Saved" : "Saving…"}</span>
        </div>

        <label className="field">
          <span>Preset</span>
          <select value={agent.preset} onChange={handlePresetChange}>
            <option value="openclaw">OpenClaw (default)</option>
            <option value="custom">Custom</option>
          </select>
        </label>

        <label className="field">
          <span>URL</span>
          <input
            type="text"
            placeholder={OPENCLAW_URL}
            spellCheck={false}
            disabled={isOpenClaw}
            {...field("url")}
            value={isOpenClaw ? OPENCLAW_URL : agent.url}
          />
        </label>

        <label className="field">
          <span>API key</span>
          <input type="password" placeholder="optional" autoComplete="off" {...field("key")} />
        </label>

        <label className="field">
          <span>Model</span>
          <input
            type="text"
            placeholder={OPENCLAW_MODEL}
            spellCheck={false}
            disabled={isOpenClaw}
            {...field("model")}
            value={isOpenClaw ? OPENCLAW_MODEL : agent.model}
          />
        </label>

        <div className="settings__note">
          {isOpenClaw ? (
            <>
              Targets a local <strong>OpenClaw</strong> Gateway — enable its
              chat-completions endpoint with{" "}
              <code>gateway.http.endpoints.chatCompletions.enabled: true</code>,
              and put its <code>OPENCLAW_GATEWAY_TOKEN</code> in API key above.
            </>
          ) : (
            <>
              Any OpenAI-compatible chat-completions endpoint works (Kimi/Moonshot
              direct, another OpenClaw instance, etc). Set URL to{" "}
              <code>echo</code> for an offline loopback test.
            </>
          )}
        </div>
      </div>
    </div>
  );
}
