//! Dictation: native microphone capture streamed to Anthropic's transcription
//! service over a WebSocket.
//!
//! Both halves live here rather than in the webview, for two independent reasons:
//!
//!  * The OAuth bearer token must never enter the browser's JS context. The GUI
//!    renders model output as HTML, so anything reachable from that context is
//!    reachable from injected content. The token is read, used and zeroized in
//!    this process, exactly as [`crate::web_history`] does it.
//!  * Capturing natively sidesteps WebView2 / WebKitGTK / WKWebView
//!    `getUserMedia` permission plumbing, which would be three separate
//!    per-engine implementations. OS-level microphone consent still applies and
//!    is attached to the host Eclipse process, not to us.
//!
//! Nothing here compiles C++. cpal does link libasound on the five Unix targets,
//! so those build images need `libasound2-dev` and the FreeBSD sysroot needs
//! alsa-lib.
//!
//! The wire protocol is the CLI's own `/api/ws/speech_to_text/voice_stream`:
//! binary `linear16` frames up, JSON transcript frames down, `KeepAlive` every
//! 8s, `CloseStream` to finalise. It is undocumented and can change.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures_util::{SinkExt, StreamExt};
use rubato::{FftFixedIn, Resampler};

/// The service takes 16 kHz mono signed 16-bit little-endian PCM.
const WIRE_RATE: usize = 16_000;
const VOICE_PATH: &str = "/api/ws/speech_to_text/voice_stream";
/// The CLI sends one immediately on open and then at this interval.
const KEEPALIVE: Duration = Duration::from_secs(8);
/// After `CloseStream`, how long to wait for the closing `TranscriptEndpoint`
/// before giving up and promoting whatever interim is pending. Mirrors the
/// CLI's own `safety` timeout.
const FINALIZE_SAFETY: Duration = Duration::from_millis(5000);
/// Samples handed to the resampler at a time. Fixed because `FftFixedIn` is a
/// fixed-size block resampler, and keeping one instance across the take is what
/// makes the chunk boundaries seamless.
const RESAMPLE_CHUNK: usize = 1024;
/// Ceiling on one take, matching the CLI's two-minute cap.
const MAX_TAKE: Duration = Duration::from_secs(120);

/// Sent with the upgrade. The CLI identifies itself here too; these exist
/// because the service is known to receive them, not because we impersonate it
/// for any other reason.
/// Every query field the service requires. Sending a subset is not tolerated:
/// it validates and rejects with `query.<field>: Field required` AFTER the
/// upgrade succeeds, so an incomplete request looks like a working connection.
/// endpointing_ms/utterance_end_ms are the silence windows that close an
/// utterance off. `language` is the recogniser target and the service takes a
/// fixed list (en, es, ja, ko, ...); Cebuano and Tagalog are NOT on it, so
/// code-switched speech is transcribed as English rather than refused.
const VOICE_QUERY_SHAPE: &str = concat!(
    "encoding=linear16",
    "&sample_rate=16000",
    "&channels=1",
    "&endpointing_ms=300",
    "&utterance_end_ms=1000",
    "&language=en",
    "&use_conversation_engine=true",
    "&forward_interims=typed",
);

const USER_AGENT: &str = "claude-cli/2.0 (external, cli)";
const CLIENT_PLATFORM: &str = "cli";

const KEEPALIVE_FRAME: &str = r#"{"type":"KeepAlive"}"#;
const CLOSE_FRAME: &str = r#"{"type":"CloseStream"}"#;

pub struct Dictation {
    recording: Arc<AtomicBool>,
    /// Raw capture at the device's own rate, drained by the streaming loop.
    pcm: Arc<Mutex<Vec<f32>>>,
    java_vm: Mutex<Option<Arc<jni::JavaVM>>>,
    callbacks: Mutex<Option<Arc<jni::objects::GlobalRef>>>,
}

impl Default for Dictation {
    fn default() -> Self {
        Self::new()
    }
}

impl Dictation {
    pub fn new() -> Self {
        Self {
            recording: Arc::new(AtomicBool::new(false)),
            pcm: Arc::new(Mutex::new(Vec::new())),
            java_vm: Mutex::new(None),
            callbacks: Mutex::new(None),
        }
    }

    pub fn register_callbacks(&self, vm: Arc<jni::JavaVM>, obj: jni::objects::GlobalRef) {
        *self.java_vm.lock().unwrap() = Some(vm);
        *self.callbacks.lock().unwrap() = Some(Arc::new(obj));
    }

    fn hooks(&self) -> Option<Hooks> {
        let vm = self.java_vm.lock().unwrap().clone()?;
        let cb = self.callbacks.lock().unwrap().clone()?;
        Some(Hooks { vm, cb })
    }

    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::SeqCst)
    }

    /// Opens the default input device, connects, and streams until [`Self::stop`].
    /// Returns as soon as the worker is spawned; transcripts arrive on the
    /// callbacks.
    ///
    /// `keyterms` is a comma-joined recognition hint list (project name, branch,
    /// …) passed through to the recogniser.
    pub fn start(&self, keyterms: String) {
        if self.recording.swap(true, Ordering::SeqCst) {
            return; // already live
        }
        self.pcm.lock().unwrap().clear();

        let recording = self.recording.clone();
        let pcm = self.pcm.clone();
        let hooks = self.hooks();

        std::thread::spawn(move || {
            // A current-thread runtime: this is one socket and two timers, and a
            // worker pool would be idle threads for the life of the IDE.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            let result = match rt {
                Ok(rt) => rt.block_on(run_take(&recording, &pcm, &hooks, &keyterms)),
                Err(e) => Err(format!("runtime: {e}")),
            };
            recording.store(false, Ordering::SeqCst);
            if let Some(h) = &hooks {
                if let Err(e) = result {
                    h.str_call("onSttError", &e);
                }
                h.void_call("onSttDone");
            }
        });
    }

    pub fn stop(&self) {
        self.recording.store(false, Ordering::SeqCst);
    }
}

/// Opens the microphone. The returned stream must be kept alive to keep
/// capturing; samples land in `sink` as mono f32 at the returned rate.
fn open_microphone(
    sink: Arc<Mutex<Vec<f32>>>,
    recording: Arc<AtomicBool>,
) -> Result<(cpal::Stream, u32), String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "no microphone found".to_string())?;
    let cfg = device
        .default_input_config()
        .map_err(|e| format!("input config: {e}"))?;
    let rate = cfg.sample_rate().0;
    let channels = cfg.channels() as usize;
    let cap = rate as usize * MAX_TAKE.as_secs() as usize;

    let on_err = move |_e| recording.store(false, Ordering::SeqCst);
    // Downmix and nothing else: this is the audio thread, where anything slow
    // is a dropout.
    let stream = device
        .build_input_stream(
            &cfg.clone().into(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let Ok(mut buf) = sink.lock() else { return };
                if buf.len() >= cap {
                    return;
                }
                if channels <= 1 {
                    buf.extend_from_slice(data);
                } else {
                    for frame in data.chunks(channels) {
                        buf.push(frame.iter().sum::<f32>() / channels as f32);
                    }
                }
            },
            on_err,
            None,
        )
        .map_err(|e| format!("cannot open microphone: {e}"))?;
    stream
        .play()
        .map_err(|e| format!("cannot start capture: {e}"))?;
    Ok((stream, rate))
}

/// Builds the authenticated upgrade request. The credential is dropped — and so
/// zeroized — when this returns.
fn build_request(
    keyterms: &str,
) -> Result<(tokio_tungstenite::tungstenite::handshake::client::Request, String), String> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let cred = crate::web_history::read_credential()
        .ok_or_else(|| "not signed in to a Claude.ai account".to_string())?;
    if cred.is_expired() {
        return Err("Claude.ai session expired — sign in again".into());
    }

    // Same derivation the CLI uses: the REST base with the scheme swapped.
    let base = std::env::var("VOICE_STREAM_BASE_URL").unwrap_or_else(|_| {
        crate::web_history::BASE_API_URL
            .replace("https://", "wss://")
            .replace("http://", "ws://")
    });
    // The full set the CLI sends. A subset is not tolerated: the service
    // validates and rejects with `query.<field>: Field required` AFTER the
    // upgrade succeeds, which is why a broken request looked like a working
    // connection.
    //
    // endpointing_ms/utterance_end_ms are the silence windows that decide when
    // an utterance is closed off; language is the recogniser target, which the
    // CLI defaults to "en".
    let url = format!("{base}{VOICE_PATH}?{VOICE_QUERY_SHAPE}");

    let mut req = url
        .as_str()
        .into_client_request()
        .map_err(|e| format!("bad voice URL: {e}"))?;
    let h = req.headers_mut();
    let bearer = format!("Bearer {}", cred.token.as_str());
    h.insert(
        "Authorization",
        bearer.parse().map_err(|_| "bad token".to_string())?,
    );
    // Left as the CLI sends it: the service knows this value, and an
    // unrecognised one risks a rejection at the upgrade.
    h.insert("x-app", "cli".parse().unwrap());
    // The CLI sends both of these alongside x-app. Omitting them was an
    // oversight, not a decision -- the service may well validate them, and a
    // server-side rejection arrives as an `error` frame AFTER a successful
    // upgrade, which is exactly the shape of failure seen in the wild.
    h.insert("User-Agent", USER_AGENT.parse().unwrap());
    h.insert(
        "anthropic-client-platform",
        CLIENT_PLATFORM.parse().unwrap(),
    );
    if !keyterms.is_empty() {
        if let Ok(v) = keyterms.parse() {
            h.insert("x-config-keyterms", v);
        }
    }
    Ok((req, url))
}

/// One dictation take, from connect through finalise.
async fn run_take(
    recording: &Arc<AtomicBool>,
    pcm: &Arc<Mutex<Vec<f32>>>,
    hooks: &Option<Hooks>,
    keyterms: &str,
) -> Result<(), String> {
    use tokio_tungstenite::tungstenite::Message;

    let (req, url) = build_request(keyterms)?;
    if let Some(h) = hooks {
        // The URL carries no secret -- the bearer token is a header -- so it is
        // safe to print, and it is the first thing worth seeing when the service
        // rejects us.
        h.log(&format!("[voice] connecting to {url}"));
        h.log(&format!(
            "[voice] keyterms: {}",
            if keyterms.is_empty() { "(none)" } else { keyterms }
        ));
    }
    let (ws, resp) = match tokio_tungstenite::connect_async(req).await {
        Ok(v) => v,
        Err(e) => {
            // The upgrade response is where a 401/403 shows up, and it is far
            // more useful than the generic error text on its own.
            if let Some(h) = hooks {
                h.log(&format!("[voice] connect FAILED: {e}"));
            }
            return Err(format!("voice connection failed: {e}"));
        }
    };
    if let Some(h) = hooks {
        h.log(&format!("[voice] connected, http {}", resp.status()));
    }
    let (mut tx, mut rx) = ws.split();

    tx.send(Message::Text(KEEPALIVE_FRAME.into()))
        .await
        .map_err(|e| format!("send: {e}"))?;

    let (stream, device_rate) = open_microphone(pcm.clone(), recording.clone())?;
    if let Some(h) = hooks {
        h.log(&format!("[voice] mic open at {device_rate} Hz"));
    }
    let mut audio = Some(stream);
    let mut resampler = Resampling::new(device_rate)?;

    let mut keepalive = tokio::time::interval(KEEPALIVE);
    keepalive.tick().await; // consume the immediate tick; one was already sent
    let mut drain = tokio::time::interval(Duration::from_millis(100));
    let deadline = tokio::time::Instant::now() + MAX_TAKE;

    // Last interim not yet superseded by a TranscriptEndpoint. Promoted to a
    // final if the stream ends without one, which is what the CLI does.
    let mut pending = String::new();
    let mut closing = false;

    loop {
        tokio::select! {
            _ = drain.tick() => {
                let stopping = !recording.load(Ordering::SeqCst);
                // Flush before closing, or the last syllable never leaves.
                let mut sent = 0usize;
                let mut peak = 0f32;
                for buf in resampler.drain(pcm, stopping) {
                    peak = peak.max(rms_of_linear16(&buf));
                    sent += buf.len();
                    tx.send(Message::Binary(buf)).await.map_err(|e| format!("send: {e}"))?;
                }
                // Drives the composer's level meter. Sent every tick, including
                // silent ones, so the bars fall back to rest instead of sticking
                // at whatever the last loud sample was.
                if !stopping {
                    if let Some(h) = hooks {
                        h.str_call("onSttLevel", &format!("{peak:.4}"));
                    }
                }
                if sent > 0 {
                    if let Some(h) = hooks {
                        h.log(&format!("[voice] sent {sent} bytes, level {peak:.3}"));
                    }
                }
                if stopping {
                    tx.send(Message::Text(CLOSE_FRAME.into()))
                        .await
                        .map_err(|e| format!("send: {e}"))?;
                    closing = true;
                    audio = None;   // anything captured now would be dropped anyway
                    if let Some(h) = hooks {
                        h.log("[voice] CloseStream sent, awaiting endpoint");
                    }
                }
            }
            _ = keepalive.tick(), if !closing => {
                tx.send(Message::Text(KEEPALIVE_FRAME.into()))
                    .await
                    .map_err(|e| format!("send: {e}"))?;
            }
            _ = tokio::time::sleep_until(deadline), if !closing => {
                recording.store(false, Ordering::SeqCst);
            }
            msg = rx.next() => {
                let Some(msg) = msg else { break };
                let msg = msg.map_err(|e| format!("voice stream error: {e}"))?;
                match msg {
                    Message::Text(t) => {
                        // Verbatim: when the service rejects something the reason
                        // is in this frame, and guessing at it from the mapped
                        // error string has already cost a debugging round.
                        if let Some(h) = hooks {
                            h.log(&format!("[voice] <- {}", t.chars().take(400).collect::<String>()));
                        }
                        if handle_frame(&t, &mut pending, hooks)? && closing {
                            break; // the endpoint that closes out a finalise
                        }
                    }
                    Message::Close(cf) => {
                        if let Some(h) = hooks {
                            h.log(&format!("[voice] socket closed: {cf:?}"));
                        }
                        break;
                    }
                    _ => {}
                }
            }
            // Armed only while closing: before that, a quiet moment is just silence.
            _ = tokio::time::sleep(FINALIZE_SAFETY), if closing => break,
        }
    }

    drop(audio);
    if !pending.is_empty() {
        if let Some(h) = hooks {
            h.str_call("onSttFinal", &pending);
        }
    }
    Ok(())
}

/// Applies one server frame. Returns true when it was a `TranscriptEndpoint`.
fn handle_frame(text: &str, pending: &mut String, hooks: &Option<Hooks>) -> Result<bool, String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return Ok(false); // unparseable frames are ignored, as in the CLI
    };
    match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "TranscriptInterim" | "TranscriptText" => {
            let data = v.get("data").and_then(|d| d.as_str()).unwrap_or("");
            if !data.is_empty() {
                *pending = data.to_string();
                if let Some(h) = hooks {
                    h.str_call("onSttPartial", data);
                }
            }
            Ok(false)
        }
        "TranscriptEndpoint" => {
            if !pending.is_empty() {
                if let Some(h) = hooks {
                    h.str_call("onSttFinal", pending);
                }
                pending.clear();
            }
            Ok(true)
        }
        "TranscriptError" => Err(v
            .get("description")
            .or_else(|| v.get("error_code"))
            .and_then(|d| d.as_str())
            .unwrap_or("unknown transcription error")
            .to_string()),
        "error" => Err(error_text(&v)),
        _ => Ok(false),
    }
}

/// Pulls the human-readable reason out of an error frame.
///
/// The service NESTS it: {"type":"error","error":{"type":..,"message":..}}.
/// Reading only the top-level `message` -- as this did at first -- discarded
/// `query.channels: Field required` and reported a generic string instead,
/// which cost two debugging rounds. Both shapes are accepted now.
fn error_text(v: &serde_json::Value) -> String {
    if let Some(inner) = v.get("error") {
        if let Some(m) = inner.get("message").and_then(|m| m.as_str()) {
            return m.to_string();
        }
        if let Some(m) = inner.as_str() {
            return m.to_string();
        }
    }
    v.get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("voice stream error")
        .to_string()
}

/// Device rate → 16 kHz as one stateful resampler for the whole take, rather
/// than a fresh one per chunk, so block boundaries stay seamless.
struct Resampling {
    inner: Option<FftFixedIn<f32>>,
    carry: Vec<f32>,
}

impl Resampling {
    fn new(device_rate: u32) -> Result<Self, String> {
        let inner = if device_rate as usize == WIRE_RATE {
            None
        } else {
            Some(
                FftFixedIn::<f32>::new(device_rate as usize, WIRE_RATE, RESAMPLE_CHUNK, 2, 1)
                    .map_err(|e| format!("resampler: {e}"))?,
            )
        };
        Ok(Self {
            inner,
            carry: Vec::new(),
        })
    }

    /// Takes everything captured so far and returns it as `linear16` payloads.
    /// A partial trailing block is carried to the next call unless `flush`.
    fn drain(&mut self, pcm: &Arc<Mutex<Vec<f32>>>, flush: bool) -> Vec<Vec<u8>> {
        {
            let Ok(mut buf) = pcm.lock() else {
                return Vec::new();
            };
            self.carry.append(&mut buf);
        }
        let mut out = Vec::new();
        match &mut self.inner {
            None => {
                if !self.carry.is_empty() {
                    out.push(to_linear16(&std::mem::take(&mut self.carry)));
                }
            }
            Some(rs) => {
                while self.carry.len() >= RESAMPLE_CHUNK {
                    let block: Vec<f32> = self.carry.drain(..RESAMPLE_CHUNK).collect();
                    if let Ok(got) = rs.process(&[block], None) {
                        if let Some(ch) = got.into_iter().next() {
                            if !ch.is_empty() {
                                out.push(to_linear16(&ch));
                            }
                        }
                    }
                }
                if flush && !self.carry.is_empty() {
                    let mut block = std::mem::take(&mut self.carry);
                    block.resize(RESAMPLE_CHUNK, 0.0);
                    if let Ok(got) = rs.process(&[block], None) {
                        if let Some(ch) = got.into_iter().next() {
                            if !ch.is_empty() {
                                out.push(to_linear16(&ch));
                            }
                        }
                    }
                }
            }
        }
        out
    }
}

/// RMS of a `linear16` payload, normalised to roughly 0..1. Used only to drive
/// the level meter, so cheap and approximate beats exact.
fn rms_of_linear16(bytes: &[u8]) -> f32 {
    if bytes.len() < 2 {
        return 0.0;
    }
    let mut acc = 0f64;
    let n = bytes.len() / 2;
    for c in bytes.chunks_exact(2) {
        let v = i16::from_le_bytes([c[0], c[1]]) as f64 / i16::MAX as f64;
        acc += v * v;
    }
    ((acc / n as f64).sqrt() as f32).clamp(0.0, 1.0)
}

/// f32 [-1,1] → signed 16-bit little-endian, which is what `linear16` means.
fn to_linear16(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// JNI callbacks -- same shape as chat.rs's, kept local so the two can diverge.
// ---------------------------------------------------------------------------

struct Hooks {
    vm: Arc<jni::JavaVM>,
    cb: Arc<jni::objects::GlobalRef>,
}

impl Hooks {
    /// Diagnostic line for the server view. Gated on the native debug flag here
    /// AND on Debug mode again in Java, so nothing is emitted -- or crosses JNI --
    /// when the user has not asked to see it.
    fn log(&self, line: &str) {
        if !crate::is_debug() {
            return;
        }
        self.str_call("onSttLog", line);
    }

    fn str_call(&self, method: &str, value: &str) {
        let Ok(mut env) = self.vm.attach_current_thread() else {
            return;
        };
        let Ok(s) = env.new_string(value) else { return };
        let obj = jni::objects::JObject::from(s);
        if env
            .call_method(
                self.cb.as_ref(),
                method,
                "(Ljava/lang/String;)V",
                &[jni::objects::JValue::Object(&obj)],
            )
            .is_err()
        {
            let _ = env.exception_clear();
        }
    }

    fn void_call(&self, method: &str) {
        let Ok(mut env) = self.vm.attach_current_thread() else {
            return;
        };
        if env
            .call_method(self.cb.as_ref(), method, "()V", &[])
            .is_err()
        {
            let _ = env.exception_clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear16_is_signed_little_endian() {
        assert_eq!(to_linear16(&[0.0]), vec![0, 0]);
        // +1.0 saturates to i16::MAX = 0x7FFF -> FF 7F little-endian.
        assert_eq!(to_linear16(&[1.0]), vec![0xFF, 0x7F]);
        // Clamped, not wrapped: an over-range sample must not flip sign.
        assert_eq!(to_linear16(&[2.0]), vec![0xFF, 0x7F]);
        assert_eq!(to_linear16(&[-2.0]), vec![0x01, 0x80]);
    }

    #[test]
    fn passthrough_when_device_is_already_16k() {
        let mut r = Resampling::new(16_000).unwrap();
        let pcm = Arc::new(Mutex::new(vec![0.5f32; 100]));
        let out = r.drain(&pcm, false);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), 200); // 100 samples, 2 bytes each
    }

    /// The ratio must hold in STEADY STATE, not on the first block: FftFixedIn
    /// has startup latency, so an early chunk legitimately emits short. A
    /// persistent rate error would mean audio leaves at the wrong effective
    /// rate, which garbles every transcript, so this checks convergence.
    #[test]
    fn resamples_48k_to_about_a_third() {
        for blocks in [3usize, 10, 30, 90] {
            let mut r = Resampling::new(48_000).unwrap();
            let pcm = Arc::new(Mutex::new(vec![0.1f32; RESAMPLE_CHUNK * blocks]));
            let bytes: usize = r.drain(&pcm, true).iter().map(|b| b.len()).sum();
            let got = bytes / 2;
            let want = RESAMPLE_CHUNK * blocks / 3;
            eprintln!(
                "{blocks:>3} blocks: got {got:>6} want {want:>6}  ({:+.1}%)",
                (got as f32 / want as f32 - 1.0) * 100.0
            );
            if blocks >= 30 {
                assert!(
                    ((got as f32 / want as f32) - 1.0).abs() < 0.05,
                    "rate drifted: {got} vs {want} over {blocks} blocks"
                );
            }
        }
    }

    #[test]
    fn partial_block_is_carried_not_emitted() {
        let mut r = Resampling::new(48_000).unwrap();
        let pcm = Arc::new(Mutex::new(vec![0.1f32; RESAMPLE_CHUNK / 2]));
        assert!(r.drain(&pcm, false).is_empty(), "half a block must be held");
        assert!(!r.drain(&pcm, true).is_empty(), "flush must emit the tail");
    }

    #[test]
    fn interim_then_endpoint_promotes_once() {
        let mut pending = String::new();
        assert!(!handle_frame(
            r#"{"type":"TranscriptInterim","data":"hello"}"#,
            &mut pending,
            &None
        )
        .unwrap());
        assert_eq!(pending, "hello");
        assert!(handle_frame(r#"{"type":"TranscriptEndpoint"}"#, &mut pending, &None).unwrap());
        assert!(pending.is_empty(), "endpoint must consume the interim");
    }

    #[test]
    fn later_interim_supersedes_earlier() {
        let mut pending = String::new();
        handle_frame(
            r#"{"type":"TranscriptInterim","data":"hel"}"#,
            &mut pending,
            &None,
        )
        .unwrap();
        handle_frame(
            r#"{"type":"TranscriptInterim","data":"hello there"}"#,
            &mut pending,
            &None,
        )
        .unwrap();
        assert_eq!(pending, "hello there");
    }

    #[test]
    fn error_frames_surface_their_message() {
        let mut p = String::new();
        let e = handle_frame(
            r#"{"type":"TranscriptError","description":"boom"}"#,
            &mut p,
            &None,
        )
        .unwrap_err();
        assert_eq!(e, "boom");
        // error_code is the fallback when description is absent.
        let e = handle_frame(
            r#"{"type":"TranscriptError","error_code":"E42"}"#,
            &mut p,
            &None,
        )
        .unwrap_err();
        assert_eq!(e, "E42");
        let e = handle_frame(r#"{"type":"error","message":"nope"}"#, &mut p, &None).unwrap_err();
        assert_eq!(e, "nope");
    }

    /// The exact frame the service sent when `channels` was missing. The reason
    /// lives at error.message, and reading top-level `message` reported a
    /// useless generic string instead of naming the missing field.
    #[test]
    fn nested_error_frame_surfaces_the_real_reason() {
        let mut p = String::new();
        let raw = r#"{"type":"error","error":{"type":"invalid_request_error","message":"query.channels: Field required"},"request_id":"req_011"}"#;
        let e = handle_frame(raw, &mut p, &None).unwrap_err();
        assert_eq!(e, "query.channels: Field required");
    }

    #[test]
    fn every_query_param_the_service_requires_is_sent() {
        // Guards against dropping one again: each was rejected individually with
        // "query.<field>: Field required" until present.
        for want in [
            "encoding=linear16",
            "sample_rate=16000",
            "channels=1",
            "endpointing_ms=300",
            "utterance_end_ms=1000",
            "language=",
            "use_conversation_engine=true",
            "forward_interims=typed",
        ] {
            assert!(
                VOICE_QUERY_SHAPE.contains(want),
                "query string lost {want}"
            );
        }
    }

    #[test]
    fn unknown_and_malformed_frames_are_ignored() {
        let mut p = String::new();
        assert!(!handle_frame("not json", &mut p, &None).unwrap());
        assert!(!handle_frame(r#"{"type":"Something"}"#, &mut p, &None).unwrap());
        assert!(p.is_empty());
    }
}
