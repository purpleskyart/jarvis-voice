import { create } from "zustand";
import type { AudioLevel, HistoryEntry, VoiceState } from "../types";

interface VoiceStore {
  /** Current voice-engine state, driven by `voice-state-changed`. */
  state: VoiceState;
  /** Live RMS amplitude (0..1) from `voice-audio-level`. */
  amplitude: number;
  /** Low / mid / high frequency bands (0..1). */
  bands: [number, number, number];
  /** Interim transcript of what the user said. */
  transcript: string;
  /** The backend's most recent response. */
  response: string;
  /** Most recent error message, if any. */
  lastError: string;
  /** Completed interactions, newest first. */
  history: HistoryEntry[];
  /** Whether responses are spoken aloud (browser TTS). */
  speakEnabled: boolean;
  /** Whether test mode is armed — turns echo back instead of hitting the agent. */
  testMode: boolean;

  setState: (state: VoiceState, detail?: string) => void;
  setAudioLevel: (level: AudioLevel) => void;
  setTranscript: (text: string) => void;
  toggleSpeak: () => void;
  toggleTestMode: () => void;
}

export const useVoiceStore = create<VoiceStore>((set) => ({
  state: "IDLE",
  amplitude: 0,
  bands: [0, 0, 0],
  transcript: "",
  response: "",
  lastError: "",
  history: [],
  speakEnabled: true,
  testMode: false,

  setState: (state, detail) =>
    set((prev) => {
      const next: Partial<VoiceStore> = { state };
      switch (state) {
        case "WAKE_DETECTED":
          // New interaction begins — clear the previous turn's text.
          next.transcript = "";
          next.response = "";
          next.lastError = "";
          break;
        case "DISPATCHING":
          if (detail) next.transcript = detail;
          break;
        case "RESPONDING":
          if (detail) next.response = detail;
          next.history = [
            {
              transcript: prev.transcript,
              response: detail ?? prev.response,
              ok: true,
              at: Date.now(),
            },
            ...prev.history,
          ].slice(0, 20);
          break;
        case "ERROR":
          if (detail) next.lastError = detail;
          next.history = [
            {
              transcript: prev.transcript,
              response: detail ?? "error",
              ok: false,
              at: Date.now(),
            },
            ...prev.history,
          ].slice(0, 20);
          break;
      }
      return next;
    }),

  setAudioLevel: ({ amplitude, bands }) => set({ amplitude, bands }),

  setTranscript: (text) => set({ transcript: text }),

  toggleSpeak: () => set((prev) => ({ speakEnabled: !prev.speakEnabled })),

  toggleTestMode: () => set((prev) => ({ testMode: !prev.testMode })),
}));
