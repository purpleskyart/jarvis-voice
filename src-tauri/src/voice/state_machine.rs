//! The always-listening voice engine.
//!
//! A single mic stream is opened at startup and consumed continuously:
//!   - In `IDLE` the engine watches for a **double-clap** (two sharp transients
//!     ~0.12–0.8s apart) — this replaces an always-on wake word.
//!   - On trigger it records the spoken command until trailing silence, runs it
//!     through the local Whisper model, POSTs the text to the agent API, and
//!     emits the response for the front end to speak.
//!
//! Every transition is emitted as `voice-state-changed`; the orb just reacts.

use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager};

use super::capture;
use super::stt::Transcriber;
use crate::agent_settings::AgentSettings;
use crate::dispatcher;

/// The states from §5.1 of the architecture, serialized in `SCREAMING_SNAKE_CASE`
/// to match the strings the front end switches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VoiceState {
    Idle,
    WakeDetected,
    Listening,
    Processing,
    Dispatching,
    Responding,
    Error,
}

#[derive(Clone, Serialize)]
pub struct StateChanged {
    pub state: VoiceState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct AudioLevel {
    pub amplitude: f32,
    pub bands: [f32; 3],
}

// ---- Clap / speech tuning ------------------------------------------------
// Claps must be LOUD, SHARP impulses that rise out of relative quiet. The
// onset check (previous frame quiet) is what keeps ordinary speech — even loud,
// plosive speech — from ever registering, since speech has no silent gap right
// before each syllable.
const FRAME_SECS: f32 = 0.02; // 20 ms frames localize the transient better
const CLAP_PEAK_MIN: f32 = 0.35; // absolute peak a clap must exceed
const CLAP_NF_RATIO: f32 = 8.0; // ...and this multiple of the noise floor
const CLAP_CREST: f32 = 5.0; // peak/rms — claps are far sharper than speech
const CLAP_ONSET_QUIET: f32 = 0.06; // the frame just before a clap must be this quiet
const CLAP_REFRACTORY: f32 = 0.10; // min gap so one clap isn't counted twice
const DOUBLE_MIN: f32 = 0.12; // valid spacing between the two claps
const DOUBLE_MAX: f32 = 0.7;
const SPEECH_AMP: f32 = 0.12; // amplitude that counts as speech
const SILENCE_END: f32 = 0.9; // trailing silence that ends a command
const NO_SPEECH_TIMEOUT: f32 = 1.0; // give up if nothing was said at all within this long
const LISTEN_MAX: f32 = 12.0; // hard cap on a single command
// Below this peak amplitude a captured buffer is digital silence, not quiet
// speech (e.g. mic permission denied — CoreAudio hands back zeroed frames
// instead of erroring). Whisper reliably hallucinates "you"/"Thank you." on
// true silence, so catch it before transcribing rather than after.
const MIC_SILENCE_FLOOR: f32 = 0.004;

#[derive(Clone)]
pub struct VoiceEngine {
    current: Arc<Mutex<VoiceState>>,
    transcriber: Arc<Mutex<Option<Arc<Transcriber>>>>,
    /// Set to start a turn (Talk button, or spacebar press).
    pending_start: Arc<AtomicBool>,
    /// Set to force-end the current turn immediately (spacebar release).
    pending_stop: Arc<AtomicBool>,
    /// When true the turn records until an explicit stop (push-to-talk), rather
    /// than auto-ending on trailing silence.
    hold_mode: Arc<AtomicBool>,
    /// Persistent toggle (not per-turn): while true, every turn — however it's
    /// triggered (double-clap, spacebar, or the Talk button) — skips the agent
    /// backend and simply echoes the transcript back as text + speech.
    test_mode: Arc<AtomicBool>,
    /// Agent gateway URL/key/model — editable from the Settings panel,
    /// persisted to disk (see `crate::agent_settings`).
    agent_settings: Arc<Mutex<AgentSettings>>,
}

impl VoiceEngine {
    pub fn new() -> Self {
        Self {
            current: Arc::new(Mutex::new(VoiceState::Idle)),
            transcriber: Arc::new(Mutex::new(None)),
            pending_start: Arc::new(AtomicBool::new(false)),
            pending_stop: Arc::new(AtomicBool::new(false)),
            hold_mode: Arc::new(AtomicBool::new(false)),
            test_mode: Arc::new(AtomicBool::new(false)),
            agent_settings: Arc::new(Mutex::new(AgentSettings::default())),
        }
    }

    pub fn agent_settings(&self) -> AgentSettings {
        self.agent_settings.lock().unwrap().clone()
    }

    pub fn set_agent_settings(&self, settings: AgentSettings) {
        *self.agent_settings.lock().unwrap() = settings;
    }

    pub fn state(&self) -> VoiceState {
        *self.current.lock().unwrap()
    }

    /// Begin a turn that auto-ends on trailing silence (the "Talk" button).
    pub fn request_listen(&self) {
        self.hold_mode.store(false, Ordering::SeqCst);
        self.pending_start.store(true, Ordering::SeqCst);
    }

    /// Arm/disarm test mode (the top-bar toggle). Applies to every subsequent
    /// turn, regardless of how it's triggered, until toggled off again.
    pub fn set_test_mode(&self, enabled: bool) {
        self.test_mode.store(enabled, Ordering::SeqCst);
    }

    /// Begin a push-to-talk turn — records until `stop_listen` (spacebar held).
    pub fn start_hold(&self) {
        self.hold_mode.store(true, Ordering::SeqCst);
        self.pending_start.store(true, Ordering::SeqCst);
    }

    /// End the current push-to-talk turn and send it (spacebar released).
    pub fn stop_hold(&self) {
        self.pending_stop.store(true, Ordering::SeqCst);
    }

    fn set(&self, app: &AppHandle, state: VoiceState, detail: Option<String>) {
        *self.current.lock().unwrap() = state;
        let _ = app.emit("voice-state-changed", StateChanged { state, detail });
    }

    fn emit_level(app: &AppHandle, amp: f32) {
        let _ = app.emit(
            "voice-audio-level",
            AudioLevel {
                amplitude: amp,
                bands: [amp * 0.9, amp, amp * 0.7],
            },
        );
    }

    fn resolve_model(app: &AppHandle) -> Option<PathBuf> {
        if let Ok(p) = app
            .path()
            .resolve("models/ggml-base.en.bin", BaseDirectory::Resource)
        {
            if p.exists() {
                return Some(p);
            }
        }
        for candidate in [
            "models/ggml-base.en.bin",
            "src-tauri/models/ggml-base.en.bin",
            "../src-tauri/models/ggml-base.en.bin",
        ] {
            let p = PathBuf::from(candidate);
            if p.exists() {
                return Some(p);
            }
        }
        None
    }

    /// Load the STT model on a background thread so startup isn't blocked.
    pub fn load_model(&self, app: AppHandle) {
        let slot = self.transcriber.clone();
        thread::spawn(move || {
            let Some(path) = Self::resolve_model(&app) else {
                eprintln!("[stt] model file not found — STT disabled");
                return;
            };
            match Transcriber::load(&path.to_string_lossy()) {
                Ok(t) => {
                    *slot.lock().unwrap() = Some(Arc::new(t));
                    eprintln!("[stt] whisper model loaded: {}", path.display());
                }
                Err(e) => eprintln!("[stt] failed to load model: {e}"),
            }
        });
    }

    /// Start the always-on mic listener: ambient levels + double-clap detection
    /// + command capture, all off one persistent stream.
    pub fn spawn_listener(&self, app: AppHandle) {
        let engine = self.clone();
        thread::spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel::<Vec<f32>>();
            let mic = match capture::open_input(tx) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("[mic] failed to open input: {e}");
                    engine.set(&app, VoiceState::Error, Some(format!("Mic unavailable: {e}")));
                    return;
                }
            };
            let src_rate = mic.sample_rate;
            let _keep_alive = &mic.stream; // dropping this stops capture
            eprintln!("[mic] always-on listener @ {src_rate} Hz — double-clap to talk");

            let frame_len = ((src_rate as f32) * FRAME_SECS).max(1.0) as usize;

            let mut leftover: Vec<f32> = Vec::new();
            let mut noise_floor = 0.005f32;
            let mut prev_rms = 0.0f32; // energy of the previous frame (onset check)
            let mut clap_count: u8 = 0;
            let mut first_clap = 0.0f32;
            let mut last_clap = -10.0f32;

            // Command-capture state (valid while LISTENING).
            let mut cmd_buf: Vec<f32> = Vec::new();
            let mut spoke = false;
            let mut silence = 0.0f32;
            let mut listen_start = 0.0f32;

            let start = Instant::now();

            loop {
                while let Ok(chunk) = rx.try_recv() {
                    leftover.extend(chunk);
                }

                while leftover.len() >= frame_len {
                    let frame: Vec<f32> = leftover.drain(0..frame_len).collect();
                    let now = start.elapsed().as_secs_f32();

                    let mut peak = 0.0f32;
                    let mut sq = 0.0f32;
                    for &s in &frame {
                        let a = s.abs();
                        if a > peak {
                            peak = a;
                        }
                        sq += s * s;
                    }
                    let rms = (sq / frame.len() as f32).sqrt();
                    let amp = (rms * 6.0).clamp(0.0, 1.0);

                    match engine.state() {
                        VoiceState::Idle => {
                            Self::emit_level(&app, amp);

                            // A clap is a loud, sharp impulse that rises out of a
                            // quiet frame — the onset (prev_rms) check is what
                            // rejects speech, which is never quiet right before a
                            // loud syllable.
                            let is_clap = peak > CLAP_PEAK_MIN
                                && peak > noise_floor * CLAP_NF_RATIO
                                && peak > rms * CLAP_CREST
                                && prev_rms < CLAP_ONSET_QUIET
                                && (now - last_clap) > CLAP_REFRACTORY;
                            if is_clap {
                                if clap_count == 1 {
                                    let gap = now - first_clap;
                                    if (DOUBLE_MIN..=DOUBLE_MAX).contains(&gap) {
                                        clap_count = 0;
                                        Self::flush(&rx, &mut leftover);
                                        engine.hold_mode.store(false, Ordering::SeqCst);
                                        engine.begin_listen(
                                            &app,
                                            &mut cmd_buf,
                                            &mut spoke,
                                            &mut silence,
                                            &mut listen_start,
                                            start,
                                        );
                                        prev_rms = 0.0;
                                        continue;
                                    } else {
                                        first_clap = now; // too far apart → new first
                                    }
                                } else {
                                    clap_count = 1;
                                    first_clap = now;
                                }
                                last_clap = now;
                            }
                            if clap_count == 1 && (now - first_clap) > DOUBLE_MAX {
                                clap_count = 0;
                            }

                            // Track the noise floor (ambient rms).
                            noise_floor = noise_floor * 0.98 + rms * 0.02;

                            // Talk button / spacebar press.
                            if engine.pending_start.swap(false, Ordering::SeqCst) {
                                engine.pending_stop.store(false, Ordering::SeqCst);
                                Self::flush(&rx, &mut leftover);
                                engine.begin_listen(
                                    &app,
                                    &mut cmd_buf,
                                    &mut spoke,
                                    &mut silence,
                                    &mut listen_start,
                                    start,
                                );
                            }
                        }

                        VoiceState::Listening => {
                            cmd_buf.extend_from_slice(&frame);
                            Self::emit_level(&app, amp);
                            if amp > SPEECH_AMP {
                                spoke = true;
                                silence = 0.0;
                            } else {
                                silence += FRAME_SECS;
                            }
                            let listen_elapsed = now - listen_start;
                            let hold = engine.hold_mode.load(Ordering::SeqCst);
                            let force_stop = engine.pending_stop.swap(false, Ordering::SeqCst);
                            let done = listen_elapsed > LISTEN_MAX
                                || force_stop
                                // Talk-button / double-clap turns: give up fast if
                                // nothing was said at all...
                                || (!hold && !spoke && listen_elapsed > NO_SPEECH_TIMEOUT)
                                // ...or process once trailing silence follows speech.
                                // Push-to-talk (hold) is exempt from both — it only
                                // ever ends on release, since the user may pause
                                // before speaking.
                                || (!hold
                                    && spoke
                                    && silence > SILENCE_END
                                    && listen_elapsed > 1.0);
                            if done {
                                // In push-to-talk we always process (the user
                                // decided when to send); otherwise require speech.
                                let heard = spoke || hold;
                                let audio =
                                    capture::resample_linear(&cmd_buf, src_rate, 16_000);
                                cmd_buf.clear();
                                engine.process_command(&app, audio, heard);
                                Self::flush(&rx, &mut leftover);
                                clap_count = 0;
                                last_clap = now;
                            }
                        }

                        // PROCESSING / DISPATCHING / RESPONDING / ERROR are driven
                        // synchronously inside process_command; just drop frames.
                        _ => {}
                    }

                    prev_rms = rms;
                }

                thread::sleep(Duration::from_millis(5));
            }
        });
    }

    /// Drop any buffered audio so a new turn starts clean.
    fn flush(rx: &std::sync::mpsc::Receiver<Vec<f32>>, leftover: &mut Vec<f32>) {
        leftover.clear();
        while rx.try_recv().is_ok() {}
    }

    /// Enter the LISTENING state with a brief "it heard you" flash.
    fn begin_listen(
        &self,
        app: &AppHandle,
        cmd_buf: &mut Vec<f32>,
        spoke: &mut bool,
        silence: &mut f32,
        listen_start: &mut f32,
        start: Instant,
    ) {
        self.set(app, VoiceState::WakeDetected, None);
        thread::sleep(Duration::from_millis(250));
        self.set(app, VoiceState::Listening, None);
        cmd_buf.clear();
        *spoke = false;
        *silence = 0.0;
        *listen_start = start.elapsed().as_secs_f32();
    }

    /// Transcribe → dispatch to the agent API → respond, then return to IDLE.
    fn process_command(&self, app: &AppHandle, audio: Vec<f32>, heard: bool) {
        let is_test = self.test_mode.load(Ordering::SeqCst);
        self.set(app, VoiceState::Processing, None);

        if !heard {
            return self.fail(app, "Didn't catch that.");
        }
        let peak = audio.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        if peak < MIC_SILENCE_FLOOR {
            return self.fail(app, "No mic signal — check the microphone permission for Jarvis.");
        }
        let transcriber = self.transcriber.lock().unwrap().clone();
        let Some(transcriber) = transcriber else {
            return self.fail(app, "Speech model still loading…");
        };
        let transcript = match transcriber.transcribe(&audio) {
            Ok(t) if !t.is_empty() => t,
            Ok(_) => return self.fail(app, "Didn't catch that."),
            Err(e) => return self.fail(app, &e),
        };
        let _ = app.emit(
            "voice-transcript",
            StateChanged {
                state: VoiceState::Processing,
                detail: Some(transcript.clone()),
            },
        );

        if is_test {
            // Echo test: skip the agent entirely and speak back what was heard.
            self.set(app, VoiceState::Responding, Some(format!("You said: {transcript}")));
            thread::sleep(Duration::from_millis(1800));
            return self.set(app, VoiceState::Idle, None);
        }

        self.set(app, VoiceState::Dispatching, Some(transcript.clone()));
        let backend = dispatcher::backend_for(&self.agent_settings());
        match tauri::async_runtime::block_on(backend.dispatch(transcript)) {
            Ok(response) => {
                self.set(app, VoiceState::Responding, Some(response));
                thread::sleep(Duration::from_millis(1800));
            }
            Err(e) => {
                self.set(app, VoiceState::Error, Some(e.to_string()));
                thread::sleep(Duration::from_millis(1600));
            }
        }
        self.set(app, VoiceState::Idle, None);
    }

    fn fail(&self, app: &AppHandle, msg: &str) {
        self.set(app, VoiceState::Error, Some(msg.to_string()));
        thread::sleep(Duration::from_millis(1400));
        self.set(app, VoiceState::Idle, None);
    }
}

impl Default for VoiceEngine {
    fn default() -> Self {
        Self::new()
    }
}
