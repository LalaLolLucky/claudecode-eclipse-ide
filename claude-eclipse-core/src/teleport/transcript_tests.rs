use super::*;
use serde_json::json;

fn ev(seq: &str, kind: &str, created: &str) -> serde_json::Value {
    json!({
        "sequence_num": seq,
        "created_at": created,
        "event_type": kind,
        "payload": { "type": kind, "uuid": format!("u{}", seq),
                     "message": {"role": "user", "content": format!("m{}", seq)} }
    })
}

#[test]
fn keeps_only_the_four_types_that_carry_the_conversation() {
    // Counts taken from a real session: 34 control_request / 34
    // control_response / 6 rate_limit_event are protocol, not transcript.
    let events = vec![
        ev("1", "user", "2026-09-01T00:00:01Z"),
        ev("2", "control_request", "2026-09-01T00:00:02Z"),
        ev("3", "assistant", "2026-09-01T00:00:03Z"),
        ev("4", "control_response", "2026-09-01T00:00:04Z"),
        ev("5", "rate_limit_event", "2026-09-01T00:00:05Z"),
        ev("6", "system", "2026-09-01T00:00:06Z"),
        ev("7", "result", "2026-09-01T00:00:07Z"),
    ];
    let lines = to_transcript(&events, "local-1", "");
    let kinds: Vec<&str> = lines.iter().map(|l| l["type"].as_str().unwrap()).collect();
    assert_eq!(kinds, vec!["user", "assistant", "system", "result"]);
}

#[test]
fn orders_numerically_not_lexically() {
    // The killer: sequence_num arrives as a STRING, so a plain sort puts
    // "10" before "9" and silently scrambles any conversation past nine.
    let events = vec![
        ev("10", "user", "2026-09-01T00:00:10Z"),
        ev("9", "user", "2026-09-01T00:00:09Z"),
        ev("100", "user", "2026-09-01T00:01:40Z"),
        ev("1", "user", "2026-09-01T00:00:01Z"),
    ];
    let lines = to_transcript(&events, "local-1", "");
    let order: Vec<&str> = lines
        .iter()
        .map(|l| l["message"]["content"].as_str().unwrap())
        .collect();
    assert_eq!(order, vec!["m1", "m9", "m10", "m100"]);
}

#[test]
fn stamps_the_time_and_reassigns_the_session() {
    let events = vec![ev("1", "user", "2026-09-05T10:00:00Z")];
    let lines = to_transcript(&events, "local-42", "C:/ws");
    assert_eq!(lines[0]["timestamp"], "2026-09-05T10:00:00Z");
    assert_eq!(lines[0]["sessionId"], "local-42");
    assert_eq!(lines[0]["session_id"], "local-42");
    assert_eq!(lines[0]["cwd"], "C:/ws");
    // The payload's own uuid is preserved — the renderer keys off it.
    assert_eq!(lines[0]["uuid"], "u1");
}

#[test]
fn survives_events_with_no_payload_or_no_sequence() {
    let events = vec![
        json!({"created_at": "2026-09-01T00:00:00Z"}),
        json!({"payload": {"type": "user"}, "created_at": "2026-09-01T00:00:01Z"}),
    ];
    let lines = to_transcript(&events, "l", "");
    assert_eq!(lines.len(), 1, "the payload-less event is skipped, not fatal");
}

#[test]
fn a_remote_control_session_names_no_repo_and_no_branch() {
    // Verbatim shape from the live detail record: sources is empty and no
    // branch appears anywhere. This is the case that must NOT touch git.
    let detail = json!({
        "id": "cse_01Hasw", "title": "How remote control works",
        "tags": ["remote-control-sdk"],
        "config": {"sources": [], "model": "claude-opus-5"}
    });
    assert_eq!(session_repo_url(&detail), "");
    assert_eq!(session_branch(&detail), "");
    assert_eq!(
        classify(&session_repo_url(&detail), "").status,
        RepoStatus::NoRepoRequired
    );
}

#[test]
fn finds_a_repo_and_branch_when_the_session_has_them() {
    let detail = json!({"config": {"sources": [
        {"type": "other", "url": "ignored"},
        {"type": "git_repository", "url": "https://github.com/acme/widgets.git",
         "branch": "feature/x"}
    ]}});
    assert_eq!(session_repo_url(&detail), "https://github.com/acme/widgets.git");
    assert_eq!(session_branch(&detail), "feature/x");
}

/// A branch name is server-supplied and ends up as a git argument.
#[test]
fn a_hostile_branch_name_is_dropped_at_the_boundary() {
    let detail = json!({"branch": "--upload-pack=touch /tmp/pwned"});
    assert_eq!(session_branch(&detail), "", "must not reach git");
    let out: serde_json::Value =
        serde_json::from_str(&checkout_branch("", "--upload-pack=evil")).unwrap();
    assert_eq!(out["ok"], false);
    assert_eq!(out["message"], "Invalid branch name.");
}

#[test]
fn writes_and_reads_back_a_local_transcript() {
    // Point HOME at a temp dir so the real ~/.claude is never touched.
    let tmp = std::env::temp_dir().join(format!("claude-tp-write-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let prev_win = std::env::var("USERPROFILE").ok();
    let prev_unix = std::env::var("HOME").ok();
    std::env::set_var("USERPROFILE", &tmp);
    std::env::set_var("HOME", &tmp);

    let events = vec![ev("1", "user", "2026-09-01T00:00:01Z"), ev("2", "assistant", "2026-09-01T00:00:02Z")];
    let lines = to_transcript(&events, "pending", "C--tpws");
    let id = write_local_session("C--tpws", &lines).expect("should write");

    let path = tmp
        .join(".claude").join("projects")
        .join(crate::session::workspace_hash("C--tpws"))
        .join(format!("{}.jsonl", id));
    let body = std::fs::read_to_string(&path).expect("file should exist");
    assert_eq!(body.lines().count(), 2, "one json object per line");
    for l in body.lines() {
        serde_json::from_str::<serde_json::Value>(l).expect("each line is valid json");
    }
    // A uuid, not the remote id — teleporting twice must not collide.
    assert_ne!(id, "cse_01Hasw");
    assert_eq!(id.len(), 36, "uuid v4 with dashes");

    if let Some(v) = prev_win { std::env::set_var("USERPROFILE", v) } else { std::env::remove_var("USERPROFILE") }
    if let Some(v) = prev_unix { std::env::set_var("HOME", v) } else { std::env::remove_var("HOME") }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn the_boundary_marker_is_shaped_like_a_system_event() {
    let m = teleport_marker("local-1", "C:/ws");
    assert_eq!(m["type"], "system");
    assert_eq!(m["subtype"], "teleported_from_web");
    assert_eq!(m["sessionId"], "local-1");
    assert_eq!(m["cwd"], "C:/ws");
    let ts = m["timestamp"].as_str().unwrap();
    assert!(ts.ends_with('Z') && ts.len() == 24, "iso8601: {}", ts);
    // Must survive a jsonl round trip on one line.
    assert_eq!(m.to_string().lines().count(), 1);
}

#[test]
fn the_timestamp_is_a_real_date() {
    let ts = now_iso8601();
    let year: i32 = ts[0..4].parse().unwrap();
    let month: u32 = ts[5..7].parse().unwrap();
    let day: u32 = ts[8..10].parse().unwrap();
    assert!(year >= 2024 && year < 2100, "{}", ts);
    assert!((1..=12).contains(&month), "{}", ts);
    assert!((1..=31).contains(&day), "{}", ts);
    assert_eq!(&ts[4..5], "-");
    assert_eq!(&ts[10..11], "T");
}
