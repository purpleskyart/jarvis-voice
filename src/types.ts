/** Voice-engine states — mirrors the Rust `VoiceState` enum (§5.1). */
export type VoiceState =
  | "IDLE"
  | "WAKE_DETECTED"
  | "LISTENING"
  | "PROCESSING"
  | "DISPATCHING"
  | "RESPONDING"
  | "ERROR";

/** Payload of the `voice-state-changed` event. */
export interface StateChanged {
  state: VoiceState;
  detail?: string;
}

/** Payload of the `voice-audio-level` event. */
export interface AudioLevel {
  amplitude: number;
  bands: [number, number, number];
}

export interface HistoryEntry {
  transcript: string;
  response: string;
  ok: boolean;
  at: number;
}

/** Agent gateway settings, editable from the Settings panel. */
export interface AgentSettings {
  preset: "openclaw" | "custom";
  url: string;
  key: string | null;
  model: string;
}
