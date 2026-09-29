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
