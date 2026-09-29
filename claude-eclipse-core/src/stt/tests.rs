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

/// The ratio must hold in STEADY STATE, not on the first block: the FFT resampler
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
