use super::*;

#[test]
fn parses_oauth_credential() {
    let raw = r#"{"claudeAiOauth":{"accessToken":"tok-abc","expiresAt":1757000000000,
                       "refreshToken":"r","scopes":[]},"organizationUuid":"o"}"#;
    let c = parse_credential(raw).expect("should parse");
    assert_eq!(c.token.as_str(), "tok-abc");
    assert_eq!(c.expires_at_ms, 1757000000000);
}

#[test]
fn api_key_only_login_reads_as_signed_out() {
    // No claudeAiOauth block — and this endpoint rejects API keys anyway.
    assert!(parse_credential(r#"{"primaryApiKey":"sk-ant-xxx"}"#).is_none());
    assert!(parse_credential(r#"{"claudeAiOauth":{"accessToken":""}}"#).is_none());
    assert!(parse_credential("not json").is_none());
}

#[test]
fn secret_debug_never_prints_the_token() {
    let s = Secret("super-secret-value".into());
    let shown = format!("{:?}", s);
    assert!(!shown.contains("super-secret-value"), "leaked: {}", shown);
    assert!(shown.contains("redacted"));
}

#[test]
fn missing_expiry_is_not_treated_as_expired() {
    // expiresAt absent → 0 → we must NOT loop into a refresh on every call.
    let c = parse_credential(r#"{"claudeAiOauth":{"accessToken":"t"}}"#).unwrap();
    assert_eq!(c.expires_at_ms, 0);
    assert!(!c.is_expired());
}

/// The two fields the CLI's own mapping picks that are easy to get wrong:
/// the time is `last_event_at` (not `updated_at`) and the status is
/// `worker_status` (not `status`) unless archived.
#[test]
fn maps_worker_status_and_last_event_at() {
    let body = r#"{"data":[{
            "id":"session_01A","title":"PR #101 & Build Tool",
            "status":"active","worker_status":"working",
            "created_at":"2026-09-01T00:00:00Z",
            "updated_at":"2026-09-02T00:00:00Z",
            "last_event_at":"2026-09-05T10:00:00Z",
            "config":{"sources":[{"type":"git_repository","name":"repo","owner":{"login":"acme"}}]}
        }]}"#;
    let rows = parse_sessions(body).expect("should parse");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "working");
    assert_eq!(rows[0].timestamp, "2026-09-05T10:00:00Z");
    assert_eq!(rows[0].repo, "acme/repo");
}

#[test]
fn archived_beats_worker_status() {
    let body = r#"{"data":[{"id":"s","status":"archived","worker_status":"idle"}]}"#;
    let rows = parse_sessions(body).unwrap();
    assert_eq!(rows[0].status, "archived");
}

#[test]
fn tolerates_thin_rows() {
    // Only `id` is required; everything else has a defined fallback, and a
    // row with no id is skipped rather than rendered blank.
    let body = r#"{"data":[{"id":"s1"},{"title":"no id here"},{"id":"s2","title":""}]}"#;
    let rows = parse_sessions(body).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].title, "Untitled");
    assert_eq!(rows[1].title, "Untitled");
    assert_eq!(rows[0].repo, "");
    assert_eq!(rows[0].timestamp, "");
}

#[test]
fn falls_back_to_created_at_when_no_events_yet() {
    let body = r#"{"data":[{"id":"s","created_at":"2026-09-01T00:00:00Z"}]}"#;
    let rows = parse_sessions(body).unwrap();
    assert_eq!(rows[0].timestamp, "2026-09-01T00:00:00Z");
}

#[test]
fn repo_without_owner_renders_bare_name() {
    let body = r#"{"data":[{"id":"s","config":{"sources":[
            {"type":"other","name":"x"},
            {"type":"git_repository","name":"solo"}]}}]}"#;
    let rows = parse_sessions(body).unwrap();
    assert_eq!(rows[0].repo, "solo");
}

#[test]
fn rejects_non_list_bodies() {
    assert!(parse_sessions(r#"{"error":{"message":"nope"}}"#).is_none());
    assert!(parse_sessions("").is_none());
}

#[test]
fn display_json_carries_no_credential_fields() {
    let rows = parse_sessions(r#"{"data":[{"id":"s","title":"t"}]}"#).unwrap();
    let json = rows_to_json(&rows).to_lowercase();
    for banned in ["accesstoken", "refreshtoken", "bearer", "authorization"] {
        assert!(
            !json.contains(banned),
            "display JSON must not carry {}: {}",
            banned,
            json
        );
    }
}

#[cfg(windows)]
#[test]
fn dpapi_round_trips() {
    let plain = br#"{"state":"ok","sessions":[]}"#;
    let blob = disk_cache::win_dpapi::protect(plain).expect("protect");
    assert_ne!(&blob[..], &plain[..], "must not store plaintext");
    let back = disk_cache::win_dpapi::unprotect(&blob).expect("unprotect");
    assert_eq!(back, plain);
}
