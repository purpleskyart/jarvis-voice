import { useEffect, useState } from "react";
import { JarvisOrb } from "./components/JarvisOrb";
import { TranscriptModal } from "./components/TranscriptModal";
import { SettingsPanel } from "./components/SettingsPanel";
import {
  bindVoiceEvents,
  triggerWake,
  setTestMode,
  pushToTalkStart,
  pushToTalkStop,
} from "./lib/tauriEvents";
import { speak, stopSpeaking } from "./lib/tts";
import { useVoiceStore } from "./state/voiceStore";
import type { VoiceState } from "./types";
import "./App.css";

const LABELS: Record<VoiceState, string> = {
  IDLE: "Ready",
  WAKE_DETECTED: "Waking",
  LISTENING: "Listening",
  PROCESSING: "Processing",
  DISPATCHING: "Thinking",
  RESPONDING: "Responding",
  ERROR: "Error",
};

function App() {
  const state = useVoiceStore((s) => s.state);
  const response = useVoiceStore((s) => s.response);
  const historyLen = useVoiceStore((s) => s.history.length);
  const testMode = useVoiceStore((s) => s.testMode);
  const toggleTestMode = useVoiceStore((s) => s.toggleTestMode);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [transcriptOpen, setTranscriptOpen] = useState(false);

  const handleToggleTestMode = () => {
    const next = !testMode;
    toggleTestMode();
    setTestMode(next);
  };

  useEffect(() => {
    const unbind = bindVoiceEvents();
    return () => {
      unbind.then((fn) => fn());
    };
  }, []);

  // Speak each new agent response aloud (browser TTS), if enabled.
  useEffect(() => {
    if (response && useVoiceStore.getState().speakEnabled) speak(response);
  }, [response]);

  // A new interaction interrupts any ongoing speech.
  useEffect(() => {
    if (state === "WAKE_DETECTED") stopSpeaking();
  }, [state]);

  // Spacebar = push-to-talk: hold to record, release to send.
  useEffect(() => {
    const holding = { current: false };
    const isTyping = (el: EventTarget | null) => {
      const t = el as HTMLElement | null;
      if (!t) return false;
      const tag = t.tagName;
      return (
        tag === "INPUT" ||
        tag === "TEXTAREA" ||
        tag === "SELECT" ||
        t.isContentEditable
      );
    };
    const onDown = (e: KeyboardEvent) => {
      if (e.code !== "Space" || e.repeat || isTyping(e.target)) return;
      e.preventDefault();
      if (holding.current) return;
      if (useVoiceStore.getState().state !== "IDLE") return;
      holding.current = true;
      pushToTalkStart();
    };
    const onUp = (e: KeyboardEvent) => {
      if (e.code !== "Space" || !holding.current) return;
      e.preventDefault();
      holding.current = false;
      pushToTalkStop();
    };
    window.addEventListener("keydown", onDown);
    window.addEventListener("keyup", onUp);
    return () => {
      window.removeEventListener("keydown", onDown);
      window.removeEventListener("keyup", onUp);
    };
  }, []);

  const busy = state !== "IDLE";

  return (
    <main className="app" data-state={state}>
      <header className="topbar">
        <div className="brand">
          <span className="brand__dot" />
          <span className="brand__name">JARVIS</span>
        </div>
        <div className="status">
          <span className="mic-live" title="Microphone is always on, listening for a double-clap">
            <span className="mic-live__ring" />●
          </span>
          <span className={"status__pip status__pip--" + state.toLowerCase()} />
          {LABELS[state]}
        </div>
        <div className="topbar__actions">
          <button
            className={"icon-btn" + (testMode ? " icon-btn--on" : "")}
            onClick={handleToggleTestMode}
            aria-label="Test mode"
            aria-pressed={testMode}
            title={
              testMode
                ? "Test mode is on — turns echo back instead of reaching the agent"
                : "Test mode is off — turns dispatch to the agent as normal"
            }
          >
            🔁
          </button>
          <button
            className="icon-btn"
            onClick={() => setSettingsOpen((v) => !v)}
            aria-label="Settings"
          >
            ⚙
          </button>
        </div>
      </header>

      <div className="stage">
        <div className="orb-wrap">
          <JarvisOrb />
        </div>

        <div className="controls">
          <button className="wake-btn" onClick={() => triggerWake()} disabled={busy}>
            {busy ? LABELS[state] + "…" : "Talk"}
          </button>
          <button
            className="transcript-btn"
            onClick={() => setTranscriptOpen(true)}
            aria-label="Open transcript"
          >
            <span className="transcript-btn__icon">❝❞</span>
            Transcript
            {historyLen > 0 && <span className="transcript-btn__badge">{historyLen}</span>}
          </button>
        </div>

        <p className="clap-hint">
          <span className="clap-hint__emoji">👏 👏</span>
          Double-clap or hold <strong>Space</strong> to talk
        </p>
      </div>

      {transcriptOpen && <TranscriptModal onClose={() => setTranscriptOpen(false)} />}
      {settingsOpen && <SettingsPanel onClose={() => setSettingsOpen(false)} />}
    </main>
  );
}

export default App;
