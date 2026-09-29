//! Microphone capture on Linux and FreeBSD, through ALSA loaded at runtime.
//!
//! Linking libasound -- which is what cpal does -- makes `libasound.so.2` a
//! hard load-time dependency of this entire library. On a machine without ALSA
//! the JVM then cannot load the library at all, and every native feature of the
//! plugin fails with it, not just dictation. Opening libasound with dlopen
//! instead keeps the library loadable everywhere: without ALSA, dictation simply
//! reports itself unavailable and the page hides the mic.
//!
//! Capture goes through the "default" PCM with ALSA's own plug layer converting
//! to 16 kHz mono S16_LE, so nothing is resampled here. On FreeBSD "default"
//! reaches OSS only when alsa-plugins is installed; without it the open fails
//! and ALSA's own reason is reported.

use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void, CStr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

use libloading::Library;

/// The runtime soname, the same on Linux and FreeBSD.
const LIBASOUND: &str = "libasound.so.2";

// Values from <alsa/pcm.h>.
const SND_PCM_STREAM_CAPTURE: c_int = 1;
const SND_PCM_FORMAT_S16_LE: c_int = 2;
const SND_PCM_ACCESS_RW_INTERLEAVED: c_int = 3;
#[cfg(target_os = "linux")]
const SND_PCM_NONBLOCK: c_int = 0x0000_0001;

/// What ALSA is asked to deliver: already the wire rate.
pub const RATE: u32 = 16_000;
/// Frames per blocking read -- 100 ms, so a stop request is seen within one.
const READ_FRAMES: usize = 1_600;
const LATENCY_US: c_uint = 100_000;

type Pcm = *mut c_void;
type OpenFn = unsafe extern "C" fn(*mut Pcm, *const c_char, c_int, c_int) -> c_int;
type SetParamsFn = unsafe extern "C" fn(Pcm, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
type ReadiFn = unsafe extern "C" fn(Pcm, *mut c_void, c_ulong) -> c_long;
type RecoverFn = unsafe extern "C" fn(Pcm, c_int, c_int) -> c_int;
type CloseFn = unsafe extern "C" fn(Pcm) -> c_int;
type StrerrorFn = unsafe extern "C" fn(c_int) -> *const c_char;
#[cfg(target_os = "linux")]
type CardNextFn = unsafe extern "C" fn(*mut c_int) -> c_int;

struct Alsa {
    open: OpenFn,
    set_params: SetParamsFn,
    readi: ReadiFn,
    recover: RecoverFn,
    close: CloseFn,
    strerror: StrerrorFn,
    #[cfg(target_os = "linux")]
    card_next: CardNextFn,
    /// Keeps every pointer above valid. Held in a static, so never unloaded.
    _lib: Library,
}

/// Set only once loading succeeds. A failure is retried on the next call, so
/// ALSA installed while Eclipse is running is picked up without a restart.
static ALSA: OnceLock<Alsa> = OnceLock::new();

impl Alsa {
    fn load() -> Result<Alsa, String> {
        // SAFETY: loading libasound runs no initialiser with preconditions, and
        // each symbol is typed exactly as <alsa/pcm.h> and <alsa/error.h> declare it.
        unsafe {
            let lib = Library::new(LIBASOUND)
                .map_err(|e| format!("ALSA is not installed ({LIBASOUND}: {e})"))?;
            macro_rules! sym {
                ($ty:ty, $name:literal) => {
                    *lib.get::<$ty>(concat!($name, "\0").as_bytes())
                        .map_err(|e| format!("{LIBASOUND} has no {}: {e}", $name))?
                };
            }
            let open = sym!(OpenFn, "snd_pcm_open");
            let set_params = sym!(SetParamsFn, "snd_pcm_set_params");
            let readi = sym!(ReadiFn, "snd_pcm_readi");
            let recover = sym!(RecoverFn, "snd_pcm_recover");
            let close = sym!(CloseFn, "snd_pcm_close");
            let strerror = sym!(StrerrorFn, "snd_strerror");
            #[cfg(target_os = "linux")]
            let card_next = sym!(CardNextFn, "snd_card_next");
            Ok(Alsa {
                open,
                set_params,
                readi,
                recover,
                close,
                strerror,
                #[cfg(target_os = "linux")]
                card_next,
                _lib: lib,
            })
        }
    }

    fn describe(&self, err: c_int) -> String {
        // SAFETY: snd_strerror returns a static string for any value.
        let p = unsafe { (self.strerror)(err) };
        if p.is_null() {
            return format!("ALSA error {err}");
        }
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

fn alsa() -> Result<&'static Alsa, String> {
    if let Some(a) = ALSA.get() {
        return Ok(a);
    }
    let loaded = Alsa::load()?;
    let _ = ALSA.set(loaded); // a concurrent caller may have won; either copy works
    Ok(ALSA.get().expect("set above"))
}

/// `None` when ALSA loads; otherwise why it does not.
pub fn unavailable_reason() -> Option<String> {
    alsa().err()
}

/// The OSS bridge alsa-plugins installs, at the path its FreeBSD package uses.
#[cfg(target_os = "freebsd")]
const OSS_PCM_MODULE: &str = "/usr/local/lib/alsa-lib/libasound_module_pcm_oss.so";

/// FreeBSD: true when alsa-plugins is not installed. FreeBSD's ALSA configuration
/// sends the default device to OSS through this module, so without it every open
/// fails even though libasound itself loads.
#[cfg(target_os = "freebsd")]
pub fn oss_module_missing() -> bool {
    !std::path::Path::new(OSS_PCM_MODULE).exists()
}

/// Linux: true when there is nothing to capture from. That takes BOTH: ALSA
/// knows no sound card, AND its default capture device will not open. A card
/// alone answers it, so a normal machine never has a device opened just to ask.
/// Without a card the open decides, because a sound server can still provide a
/// default input with no card behind it (a Bluetooth headset under PipeWire).
/// ALSA being absent is not this question; that is reported separately.
#[cfg(target_os = "linux")]
pub fn no_capture_device() -> bool {
    let Ok(alsa) = alsa() else { return false };
    let mut card: c_int = -1;
    // SAFETY: `card` is a valid in/out int; -1 asks for the first card.
    if unsafe { (alsa.card_next)(&mut card) } == 0 && card >= 0 {
        return false;
    }
    let mut pcm: Pcm = std::ptr::null_mut();
    // Non-blocking, so a device held by something else cannot stall the caller.
    // SAFETY: `pcm` is a valid out-pointer and the name is NUL-terminated.
    let rc = unsafe {
        (alsa.open)(&mut pcm, b"default\0".as_ptr().cast(), SND_PCM_STREAM_CAPTURE, SND_PCM_NONBLOCK)
    };
    if rc < 0 {
        return true;
    }
    unsafe { (alsa.close)(pcm) };
    false
}

/// A running capture. Dropping it stops the reader and closes the device.
pub struct Capture {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Joined so the device is closed before another take can open it.
        // Bounded by one READ_FRAMES read.
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

/// Opens the default capture device and starts reading it. Samples land in
/// `sink` as mono f32 at [`RATE`], up to `cap` of them.
///
/// The PCM handle is opened, used and closed on the reader thread, so it never
/// crosses threads; the open's outcome is handed back before this returns.
pub fn open(sink: Arc<Mutex<Vec<f32>>>, recording: Arc<AtomicBool>, cap: usize) -> Result<Capture, String> {
    let alsa = alsa()?;
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let reader_stop = stop.clone();

    let worker = std::thread::Builder::new()
        .name("claude-dictation-alsa".into())
        .spawn(move || {
            let mut pcm: Pcm = std::ptr::null_mut();
            // SAFETY: `pcm` is a valid out-pointer and the name is NUL-terminated.
            let rc = unsafe {
                (alsa.open)(&mut pcm, b"default\0".as_ptr().cast(), SND_PCM_STREAM_CAPTURE, 0)
            };
            if rc < 0 {
                let _ = ready_tx.send(Err(format!("cannot open microphone: {}", alsa.describe(rc))));
                return;
            }
            // Soft resampling on: the plug layer converts whatever the device
            // runs at into exactly what the service takes.
            let rc = unsafe {
                (alsa.set_params)(pcm, SND_PCM_FORMAT_S16_LE, SND_PCM_ACCESS_RW_INTERLEAVED, 1, RATE, 1, LATENCY_US)
            };
            if rc < 0 {
                unsafe { (alsa.close)(pcm) };
                let _ = ready_tx.send(Err(format!("cannot configure microphone: {}", alsa.describe(rc))));
                return;
            }
            let _ = ready_tx.send(Ok(()));

            let mut frames = vec![0i16; READ_FRAMES];
            while !reader_stop.load(Ordering::SeqCst) {
                // SAFETY: the buffer holds READ_FRAMES mono S16 frames.
                let n = unsafe { (alsa.readi)(pcm, frames.as_mut_ptr().cast(), READ_FRAMES as c_ulong) };
                if n < 0 {
                    // Overruns and suspends recover; anything else ends the take,
                    // exactly as a cpal stream error does elsewhere.
                    if unsafe { (alsa.recover)(pcm, n as c_int, 1) } < 0 {
                        recording.store(false, Ordering::SeqCst);
                        break;
                    }
                    continue;
                }
                let Ok(mut buf) = sink.lock() else { break };
                if buf.len() < cap {
                    buf.extend(frames[..n as usize].iter().map(|&s| s as f32 / 32_768.0));
                }
            }
            unsafe { (alsa.close)(pcm) };
        })
        .map_err(|e| format!("capture thread: {e}"))?;

    match ready_rx.recv() {
        Ok(Ok(())) => Ok(Capture { stop, worker: Some(worker) }),
        Ok(Err(e)) => {
            let _ = worker.join();
            Err(e)
        }
        Err(_) => {
            let _ = worker.join();
            Err("capture thread ended before opening the microphone".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Meant to be run both where libasound exists and where it does not: the
    /// probe must answer either way, never panic, and name what is missing.
    #[test]
    fn probe_answers_with_or_without_alsa() {
        match unavailable_reason() {
            None => eprintln!("ALSA loaded"),
            Some(reason) => {
                eprintln!("ALSA unavailable: {reason}");
                assert!(reason.contains(LIBASOUND), "reason should name the library: {reason}");
            }
        }
    }

    /// Set EXPECT_NO_CAPTURE_DEVICE=1 where there is no card and no usable default
    /// (a bare container), or =0 where the default opens without a card (an
    /// .asoundrc pointing it at the null device). Unset, it only checks that the
    /// probe answers.
    #[cfg(target_os = "linux")]
    #[test]
    fn no_capture_device_matches_the_environment() {
        let got = no_capture_device();
        eprintln!("no_capture_device() = {got}");
        if let Ok(want) = std::env::var("EXPECT_NO_CAPTURE_DEVICE") {
            assert_eq!(got, want == "1", "EXPECT_NO_CAPTURE_DEVICE={want}");
        }
    }

    /// With ALSA present but no sound device (a container), opening must come
    /// back as ALSA's own error rather than hang or panic.
    #[test]
    fn open_without_a_device_is_an_error() {
        if unavailable_reason().is_some() {
            eprintln!("skipped: no ALSA here");
            return;
        }
        match open(Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicBool::new(true)), RATE as usize) {
            Ok(_) => eprintln!("a capture device exists here; opened and closed it"),
            Err(e) => eprintln!("open failed as expected: {e}"),
        }
    }
}
